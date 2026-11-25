//! Local opt-in transcript history. Files contain user content and are never logged.
use crate::core::audio::audio_input::AudioSource;
use crate::core::lifecycle::session_archive::{
    SavedTranscript, TranscriptExportOptions, TranscriptPage, TranscriptPageEntry,
    TranscriptWriter, AUDIO_BYTE_LIMIT, TRANSCRIPT_PAGE_SIZE,
};
use crate::core::models::SubtitlePair;
use serde::{de, Deserialize, Serialize};
use std::fs;
use std::io::{self, BufRead, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

mod reader;

const MAX_TRANSCRIPT_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TITLE_CHARACTERS: usize = 120;
const MAX_TITLE_FILE_BYTES: u64 = 4096;

#[derive(Clone)]
pub struct SessionHistory {
    directory: PathBuf,
    disabled: bool,
    io: Arc<Mutex<()>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryItem {
    pub audio_sources: Vec<AudioSource>,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
    pub count: usize,
    pub limited: bool,
    pub has_audio: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryDeleteReport {
    pub deleted: Vec<String>,
    pub failed: Vec<String>,
}

#[derive(Deserialize, Serialize)]
struct HistoryTitle {
    version: u32,
    title: String,
}

#[derive(Debug, PartialEq, Eq)]
struct HistoryMetadata {
    version: u32,
    id: String,
    started_at_ms: u64,
    ended_at_ms: u64,
    // Journal timestamps can mask a changed header timestamp during a rescan.
    header_ended_at_ms: u64,
    limited: bool,
    entries: usize,
}

// Validate the same persisted fields as SubtitlePair without retaining owned
// caption strings; serde may reuse scratch space for the largest input string.
#[derive(Deserialize)]
struct HistoryPairMetadata {
    #[serde(default, rename = "audioSource")]
    _audio_source: AudioSource,
    #[serde(rename = "source")]
    _source: HistoryText,
    #[serde(rename = "translation")]
    _translation: HistoryText,
    #[serde(rename = "createdAt")]
    created_at_ms: u64,
}

struct HistoryText;

impl<'de> Deserialize<'de> for HistoryText {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct TextVisitor;
        impl de::Visitor<'_> for TextVisitor {
            type Value = HistoryText;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a string")
            }

            fn visit_str<E: de::Error>(self, _value: &str) -> Result<Self::Value, E> {
                Ok(HistoryText)
            }
        }
        deserializer.deserialize_str(TextVisitor)
    }
}

impl SessionHistory {
    pub fn new(directory: PathBuf, disabled: bool) -> Self {
        Self {
            directory,
            disabled,
            io: Arc::new(Mutex::new(())),
        }
    }

    pub fn available(&self) -> bool {
        !self.disabled
    }

    fn path(&self, id: &str, extension: &str) -> io::Result<PathBuf> {
        uuid::Uuid::parse_str(id)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid history id"))?;
        Ok(self.directory.join(format!("{id}.{extension}")))
    }

    fn audio_path(&self, id: &str, source: AudioSource, extension: &str) -> io::Result<PathBuf> {
        // Preserve the original system-audio filename so saved sessions from
        // earlier versions remain readable without content migration.
        match source {
            AudioSource::System => self.path(id, extension),
            AudioSource::Microphone => self.path(id, &format!("microphone.{extension}")),
        }
    }

    pub fn audio_sources(&self, id: &str) -> io::Result<Vec<AudioSource>> {
        if self.disabled {
            return Ok(Vec::new());
        }
        let mut sources = Vec::new();
        for source in [AudioSource::System, AudioSource::Microphone] {
            if self
                .audio_path(id, source, "wav")?
                .metadata()
                .is_ok_and(|meta| meta.len() > 44)
                || self
                    .audio_path(id, source, "pcm")?
                    .metadata()
                    .is_ok_and(|meta| meta.len() > 4)
            {
                sources.push(source);
            }
        }
        Ok(sources)
    }

    fn open_transcript(&self, id: &str) -> io::Result<fs::File> {
        if self.disabled {
            return Err(io::Error::new(io::ErrorKind::NotFound, "history disabled"));
        }
        let final_path = self.path(id, "json")?;
        let path = if final_path.is_file() {
            final_path
        } else {
            self.path(id, "jsonl")?
        };
        let file = fs::File::open(path)?;
        if file.metadata()?.len() > MAX_TRANSCRIPT_FILE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "history file too large",
            ));
        }
        Ok(file)
    }

    #[cfg(test)]
    fn read(&self, id: &str) -> io::Result<SavedTranscript> {
        let _io = self.io.lock().unwrap();
        let mut input = io::BufReader::new(self.open_transcript(id)?);
        let mut entries = Vec::new();
        let metadata = reader::scan(
            &mut input,
            id,
            Some(&mut |_, pair| {
                entries.push(pair);
                Ok(())
            }),
        )?;
        Ok(SavedTranscript {
            version: metadata.version,
            id: metadata.id,
            started_at_ms: metadata.started_at_ms,
            ended_at_ms: metadata.ended_at_ms,
            limited: metadata.limited,
            entries,
        })
    }

    fn read_metadata(&self, id: &str) -> io::Result<HistoryMetadata> {
        let _io = self.io.lock().unwrap();
        reader::scan(&mut io::BufReader::new(self.open_transcript(id)?), id, None)
    }

    fn read_title(&self, id: &str) -> io::Result<Option<String>> {
        let _io = self.io.lock().unwrap();
        let path = self.path(id, "meta.json")?;
        let file = match fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        // Read a bounded amount even if an external writer grows the sidecar.
        let mut bytes = Vec::new();
        file.take(MAX_TITLE_FILE_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_TITLE_FILE_BYTES {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid title"));
        }
        let metadata: HistoryTitle = serde_json::from_slice(&bytes)?;
        if metadata.version != 1 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid title"));
        }
        normalized_title(&metadata.title)
    }

    pub fn rename(&self, id: &str, title: &str) -> io::Result<()> {
        let title = normalized_title(title)?;
        let path = self.path(id, "meta.json")?;
        let _io = self.io.lock().unwrap();
        // Never create detached annotations for an absent or deleted session.
        self.open_transcript(id)?;
        let Some(title) = title else {
            return remove_if_present(&path);
        };
        let mut temporary = tempfile::NamedTempFile::new_in(&self.directory)?;
        serde_json::to_writer(&mut temporary, &HistoryTitle { version: 1, title })
            .map_err(io::Error::other)?;
        temporary.as_file().sync_all()?;
        temporary.persist(path).map_err(|error| error.error)?;
        Ok(())
    }

    pub fn begin(&self, id: &str, started_at_ms: u64) -> io::Result<()> {
        if self.disabled {
            return Ok(());
        }
        fs::create_dir_all(&self.directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.directory, fs::Permissions::from_mode(0o700))?;
        }
        let _io = self.io.lock().unwrap();
        let path = self.path(id, "jsonl")?;
        let mut file = tempfile::NamedTempFile::new_in(&self.directory)?;
        let header = SavedTranscript {
            version: 1,
            id: id.to_owned(),
            started_at_ms,
            ended_at_ms: started_at_ms,
            limited: false,
            entries: Vec::new(),
        };
        serde_json::to_writer(&mut file, &header).map_err(io::Error::other)?;
        file.write_all(b"\n")?;
        file.as_file().sync_all()?;
        file.persist_noclobber(path).map_err(|error| error.error)?;
        Ok(())
    }

    pub fn append_pair(&self, id: &str, pair: &SubtitlePair) -> io::Result<()> {
        if self.disabled {
            return Ok(());
        }
        let _io = self.io.lock().unwrap();
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(self.path(id, "jsonl")?)?;
        let previous_len = file.metadata()?.len();
        let result = serde_json::to_writer(&mut file, pair)
            .map_err(io::Error::other)
            .and_then(|_| file.write_all(b"\n"));
        if result.is_err() {
            let _ = file.set_len(previous_len);
        }
        result
    }

    #[cfg(test)]
    pub fn append_pcm(&self, id: &str, sample_rate: u32, data: &[u8]) -> io::Result<()> {
        self.append_pcm_from(id, AudioSource::System, sample_rate, data)
    }

    pub fn append_pcm_from(
        &self,
        id: &str,
        source: AudioSource,
        sample_rate: u32,
        data: &[u8],
    ) -> io::Result<()> {
        if self.disabled || data.is_empty() {
            return Ok(());
        }
        if !matches!(sample_rate, 16_000 | 24_000) || !data.len().is_multiple_of(2) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid audio format",
            ));
        }
        let _io = self.io.lock().unwrap();
        let path = self.audio_path(id, source, "pcm")?;
        let mut total_bytes = 0_u64;
        for track in [AudioSource::System, AudioSource::Microphone] {
            if let Ok(meta) = self.audio_path(id, track, "pcm")?.metadata() {
                total_bytes = total_bytes.saturating_add(meta.len().saturating_sub(4));
            }
        }
        if data.len() as u64 > (AUDIO_BYTE_LIMIT as u64).saturating_sub(total_bytes) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "audio limit reached",
            ));
        }
        if !path.exists() {
            let mut temporary = tempfile::NamedTempFile::new_in(&self.directory)?;
            temporary.write_all(&sample_rate.to_le_bytes())?;
            temporary.write_all(data)?;
            temporary.as_file().sync_all()?;
            temporary
                .persist_noclobber(path)
                .map_err(|error| error.error)?;
            return Ok(());
        }
        let mut file = fs::OpenOptions::new().read(true).append(true).open(path)?;
        let mut rate = [0_u8; 4];
        file.read_exact(&mut rate)?;
        if u32::from_le_bytes(rate) != sample_rate {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "audio rate changed",
            ));
        }
        let previous_len = file.metadata()?.len();
        let result = file.write_all(data);
        if result.is_err() {
            let _ = file.set_len(previous_len);
        }
        result
    }

    pub fn clear_text(&self, id: &str) -> io::Result<()> {
        if self.disabled {
            return Ok(());
        }
        let _io = self.io.lock().unwrap();
        let path = self.path(id, "jsonl")?;
        let file = fs::File::open(&path)?;
        let header = io::BufReader::new(file)
            .lines()
            .next()
            .transpose()?
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "empty history"))?;
        let mut temporary = tempfile::NamedTempFile::new_in(&self.directory)?;
        temporary.write_all(header.as_bytes())?;
        temporary.write_all(b"\n")?;
        temporary.as_file().sync_all()?;
        temporary.persist(path).map_err(|error| error.error)?;
        Ok(())
    }

    pub fn clear_audio(&self, id: &str) -> io::Result<()> {
        if self.disabled {
            return Ok(());
        }
        let _io = self.io.lock().unwrap();
        self.remove_audio_files(id)
    }

    fn remove_audio_files(&self, id: &str) -> io::Result<()> {
        for source in [AudioSource::System, AudioSource::Microphone] {
            for extension in ["pcm", "wav"] {
                let path = self.audio_path(id, source, extension)?;
                if path.exists() {
                    fs::remove_file(path)?;
                }
            }
        }
        Ok(())
    }

    /// Commits the on-disk journals without keeping a second session-sized
    /// transcript or PCM recording alive in the session manager.
    pub fn finalize(&self, id: &str, ended_at_ms: u64, limited: bool) -> io::Result<bool> {
        if self.disabled {
            return Ok(false);
        }
        let io_guard = self.io.lock().unwrap();
        let mut input = io::BufReader::new(self.open_transcript(id)?);
        let saved = reader::scan(&mut input, id, None)?;
        let audio_sources = self.audio_sources(id)?;
        if saved.entries == 0 && audio_sources.is_empty() {
            drop(io_guard);
            self.discard(id)?;
            return Ok(false);
        }
        input.rewind()?;
        let mut created_audio = Vec::new();
        let result: io::Result<()> = (|| {
            for source in audio_sources {
                let pcm_path = self.audio_path(id, source, "pcm")?;
                let audio_path = self.audio_path(id, source, "wav")?;
                let mut input = fs::File::open(pcm_path)?;
                let length = input.metadata()?.len().saturating_sub(4);
                if length > AUDIO_BYTE_LIMIT as u64 || !length.is_multiple_of(2) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "invalid audio size",
                    ));
                }
                let mut rate = [0_u8; 4];
                input.read_exact(&mut rate)?;
                let rate = u32::from_le_bytes(rate);
                if !matches!(rate, 16_000 | 24_000) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "invalid audio rate",
                    ));
                }
                if verified_existing_wav(&audio_path, rate, length, &mut input)? {
                    continue;
                }
                let mut temporary = tempfile::NamedTempFile::new_in(&self.directory)?;
                temporary.write_all(&wav_header(rate, length as u32))?;
                io::copy(&mut input, &mut temporary)?;
                temporary.as_file().sync_all()?;
                temporary
                    .persist_noclobber(&audio_path)
                    .map_err(|error| error.error)?;
                created_audio.push(audio_path);
            }
            persist_final_transcript(
                &self.directory,
                &self.path(id, "json")?,
                &saved,
                &mut input,
                ended_at_ms,
                limited,
            )
        })();
        if result.is_err() {
            for path in created_audio {
                let _ = fs::remove_file(path);
            }
        }
        result?;
        let _ = fs::remove_file(self.path(id, "jsonl")?);
        for source in [AudioSource::System, AudioSource::Microphone] {
            let _ = fs::remove_file(self.audio_path(id, source, "pcm")?);
        }
        Ok(true)
    }

    pub fn list(&self) -> io::Result<Vec<HistoryItem>> {
        if self.disabled || !self.directory.exists() {
            return Ok(Vec::new());
        }
        let mut items = Vec::new();
        let mut ids = std::collections::HashSet::new();
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            if !matches!(
                entry.path().extension().and_then(|value| value.to_str()),
                Some("json" | "jsonl")
            ) {
                continue;
            }
            let Some(id) = entry
                .path()
                .file_stem()
                .and_then(|value| value.to_str())
                .map(str::to_owned)
            else {
                continue;
            };
            if !ids.insert(id.clone()) {
                continue;
            }
            let Ok(saved) = self.read_metadata(&id) else {
                continue;
            };
            let audio_sources = self.audio_sources(&id)?;
            let has_audio = !audio_sources.is_empty();
            if saved.entries == 0 && !has_audio {
                continue;
            }
            items.push(HistoryItem {
                audio_sources,
                has_audio,
                count: saved.entries,
                // An optional, damaged annotation must not hide the saved
                // transcript or prevent its recovery/export.
                title: self.read_title(&id).unwrap_or(None),
                id,
                started_at_ms: saved.started_at_ms,
                ended_at_ms: saved.ended_at_ms,
                limited: saved.limited,
            });
        }
        items.sort_by_key(|item| std::cmp::Reverse(item.started_at_ms));
        Ok(items)
    }

    pub fn page(&self, id: &str, query: &str, requested_page: usize) -> io::Result<TranscriptPage> {
        let _io = self.io.lock().unwrap();
        let mut input = io::BufReader::new(self.open_transcript(id)?);
        read_page(&mut input, id, query, requested_page)
    }

    pub fn export_transcript(&self, id: &str) -> io::Result<Vec<u8>> {
        self.export_transcript_with_options(id, TranscriptExportOptions::default())
    }

    pub fn export_transcript_with_options(
        &self,
        id: &str,
        options: TranscriptExportOptions,
    ) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.export_transcript_to_writer(id, options, &mut bytes)?;
        Ok(bytes)
    }

    fn export_transcript_to_writer(
        &self,
        id: &str,
        options: TranscriptExportOptions,
        writer: &mut impl Write,
    ) -> io::Result<()> {
        let _io = self.io.lock().unwrap();
        let mut input = io::BufReader::new(self.open_transcript(id)?);
        write_transcript(&mut input, id, options, writer)
    }

    /// Native adapters choose an app-private export directory and share only
    /// this completed file through their platform's explicit export action.
    pub fn export_transcript_to_directory(
        &self,
        id: &str,
        options: TranscriptExportOptions,
        directory: &Path,
    ) -> io::Result<PathBuf> {
        write_private_export(directory, options, |writer| {
            self.export_transcript_to_writer(id, options, writer)
        })
    }

    #[cfg(test)]
    pub fn audio(&self, id: &str) -> io::Result<Vec<u8>> {
        self.audio_selected(id, None)
    }

    /// Never guess which speaker the user intended when two tracks exist.
    pub fn audio_selected(&self, id: &str, source: Option<AudioSource>) -> io::Result<Vec<u8>> {
        let sources = self.audio_sources(id)?;
        let source = match (source, sources.as_slice()) {
            (Some(source), _) if sources.contains(&source) => source,
            (None, [source]) => *source,
            (None, [_, _, ..]) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "select audio source",
                ))
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "no audio for source",
                ))
            }
        };
        self.audio_from(id, source)
    }

    pub fn audio_from(&self, id: &str, source: AudioSource) -> io::Result<Vec<u8>> {
        self.read_metadata(id)?;
        if self.disabled {
            return Err(io::Error::new(io::ErrorKind::NotFound, "history disabled"));
        }
        // Until finalization consumes the journal, it is the authoritative
        // track. A failed conversion may leave an invalid or stale WAV beside
        // it; only an absent journal permits reading the finalized output.
        let pcm = match fs::File::open(self.audio_path(id, source, "pcm")?) {
            Ok(file) => Some(file),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        if let Some(mut file) = pcm {
            if file.metadata()?.len() > AUDIO_BYTE_LIMIT as u64 + 4 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "audio file too large",
                ));
            }
            let mut rate = [0_u8; 4];
            file.read_exact(&mut rate)?;
            let rate = u32::from_le_bytes(rate);
            let length = file.metadata()?.len().saturating_sub(4);
            if !matches!(rate, 16_000 | 24_000) || !length.is_multiple_of(2) {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid audio"));
            }
            let mut audio = Vec::with_capacity(length as usize + 44);
            audio.extend_from_slice(&wav_header(rate, length as u32));
            file.read_to_end(&mut audio)?;
            return Ok(audio);
        }
        let mut file = fs::File::open(self.audio_path(id, source, "wav")?)?;
        if file.metadata()?.len()
            > crate::core::lifecycle::session_archive::AUDIO_BYTE_LIMIT as u64 + 44
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "audio file too large",
            ));
        }
        let mut audio = Vec::new();
        file.read_to_end(&mut audio)?;
        Ok(audio)
    }

    pub fn delete(&self, id: &str) -> io::Result<()> {
        let _io = self.io.lock().unwrap();
        let final_path = self.path(id, "json")?;
        let journal_path = self.path(id, "jsonl")?;
        let path = if final_path.is_file() {
            final_path
        } else {
            journal_path
        };
        if self.disabled {
            return Err(io::Error::new(io::ErrorKind::NotFound, "history disabled"));
        }
        if !path.is_file() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "session missing"));
        }
        remove_if_present(&self.path(id, "meta.json")?)?;
        self.remove_audio_files(id)?;
        let journal_path = self.path(id, "jsonl")?;
        if journal_path.exists() && journal_path != path {
            fs::remove_file(journal_path)?;
        }
        fs::remove_file(path)
    }

    pub fn delete_many<S: AsRef<str>, P: AsRef<str>>(
        &self,
        ids: &[S],
        protected_ids: &[P],
    ) -> io::Result<HistoryDeleteReport> {
        if self.disabled {
            return Err(io::Error::new(io::ErrorKind::NotFound, "history disabled"));
        }
        let protected: std::collections::HashSet<&str> =
            protected_ids.iter().map(AsRef::as_ref).collect();
        let mut seen = std::collections::HashSet::new();
        let mut report = HistoryDeleteReport {
            deleted: Vec::new(),
            failed: Vec::new(),
        };
        for id in ids {
            let id = id.as_ref();
            if !seen.insert(id) {
                continue;
            }
            if protected.contains(id) {
                report.failed.push(id.to_owned());
                continue;
            }
            match self.delete(id) {
                Ok(()) => report.deleted.push(id.to_owned()),
                Err(_) => report.failed.push(id.to_owned()),
            }
        }
        Ok(report)
    }

    pub fn discard(&self, id: &str) -> io::Result<()> {
        if self.disabled {
            return Ok(());
        }
        let _io = self.io.lock().unwrap();
        remove_if_present(&self.path(id, "meta.json")?)?;
        self.remove_audio_files(id)?;
        let path = self.path(id, "jsonl")?;
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }
}

fn normalized_title(title: &str) -> io::Result<Option<String>> {
    let title = title.trim();
    if title.chars().count() > MAX_TITLE_CHARACTERS || title.chars().any(char::is_control) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid title"));
    }
    Ok((!title.is_empty()).then(|| title.to_owned()))
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn read_page(
    input: &mut (impl BufRead + Seek),
    id: &str,
    query: &str,
    requested_page: usize,
) -> io::Result<TranscriptPage> {
    let query = query.trim().to_lowercase();
    let matches = |pair: &SubtitlePair| {
        query.is_empty()
            || pair.source.to_lowercase().contains(&query)
            || pair.translation.to_lowercase().contains(&query)
    };
    // The usual newest page needs only one pass. A ring retains the last
    // 30 matches while metadata and recovery validation continue to EOF.
    if requested_page == 0 {
        let mut entries = Vec::with_capacity(TRANSCRIPT_PAGE_SIZE);
        let mut total = 0;
        reader::scan(
            input,
            id,
            Some(&mut |index, pair| {
                if matches(&pair) {
                    total += 1;
                    let entry = TranscriptPageEntry {
                        audio_source: pair.audio_source,
                        index: index + 1,
                        source: pair.source,
                        translation: pair.translation,
                        created_at_ms: pair.created_at_ms,
                    };
                    if entries.len() < TRANSCRIPT_PAGE_SIZE {
                        entries.push(entry);
                    } else {
                        entries[(total - 1) % TRANSCRIPT_PAGE_SIZE] = entry;
                    }
                }
                Ok(())
            }),
        )?;
        if total >= TRANSCRIPT_PAGE_SIZE {
            entries.rotate_left(total % TRANSCRIPT_PAGE_SIZE);
        }
        entries.reverse();
        return Ok(TranscriptPage {
            total,
            page: 0,
            entries,
        });
    }
    let mut total = 0;
    let metadata = if query.is_empty() {
        let metadata = reader::scan(input, id, None)?;
        total = metadata.entries;
        metadata
    } else {
        reader::scan(
            input,
            id,
            Some(&mut |_, pair| {
                total += usize::from(matches(&pair));
                Ok(())
            }),
        )?
    };
    let page = requested_page.min(total.saturating_sub(1) / TRANSCRIPT_PAGE_SIZE);
    let end = total.saturating_sub(page * TRANSCRIPT_PAGE_SIZE);
    let start = end.saturating_sub(TRANSCRIPT_PAGE_SIZE);
    let mut entries = Vec::with_capacity(end - start);
    let mut matched = 0;
    input.rewind()?;
    reader::scan_verified(
        input,
        &metadata,
        Some(&mut |index, pair| {
            if matches(&pair) {
                if (start..end).contains(&matched) {
                    entries.push(TranscriptPageEntry {
                        audio_source: pair.audio_source,
                        index: index + 1,
                        source: pair.source,
                        translation: pair.translation,
                        created_at_ms: pair.created_at_ms,
                    });
                }
                matched += 1;
            }
            Ok(())
        }),
    )?;
    if matched != total {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "history changed during read",
        ));
    }
    entries.reverse();
    Ok(TranscriptPage {
        total,
        page,
        entries,
    })
}

fn write_transcript(
    input: &mut (impl BufRead + Seek),
    id: &str,
    options: TranscriptExportOptions,
    writer: &mut impl Write,
) -> io::Result<()> {
    let metadata = reader::scan(input, id, None)?;
    if metadata.entries == 0 {
        return Err(io::Error::new(io::ErrorKind::NotFound, "no transcript"));
    }
    input.rewind()?;
    let mut formatter = TranscriptWriter::new(writer, metadata.started_at_ms, options);
    reader::scan_verified(
        input,
        &metadata,
        Some(&mut |_, pair| formatter.append(&pair)),
    )?;
    formatter.finish(metadata.limited)?;
    Ok(())
}

fn write_private_export(
    directory: &Path,
    options: TranscriptExportOptions,
    write: impl FnOnce(&mut io::BufWriter<&mut tempfile::NamedTempFile>) -> io::Result<()>,
) -> io::Result<PathBuf> {
    fs::create_dir_all(directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    }
    let extension = match options.format {
        crate::core::lifecycle::session_archive::TranscriptExportFormat::Txt => "txt",
        crate::core::lifecycle::session_archive::TranscriptExportFormat::Markdown => "md",
    };
    let mut temporary = tempfile::Builder::new()
        .prefix("mimi-")
        .suffix(&format!(".{extension}"))
        .tempfile_in(directory)?;
    {
        let mut writer = io::BufWriter::with_capacity(8 * 1024, &mut temporary);
        write(&mut writer)?;
        writer.flush()?;
    }
    temporary.as_file().sync_all()?;
    let (_, path) = temporary.keep().map_err(|error| error.error)?;
    Ok(path)
}

fn persist_final_transcript(
    directory: &Path,
    path: &Path,
    original: &HistoryMetadata,
    input: &mut impl BufRead,
    ended_at_ms: u64,
    limited: bool,
) -> io::Result<()> {
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    write_final_transcript(&mut temporary, original, input, ended_at_ms, limited)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(path)
        .map_err(|error| error.error)?;
    Ok(())
}

fn write_final_transcript(
    writer: &mut impl Write,
    original: &HistoryMetadata,
    input: &mut impl BufRead,
    ended_at_ms: u64,
    limited: bool,
) -> io::Result<()> {
    // Emit the same compact JSON schema while retaining only one parsed pair.
    // Flush errors must reach the caller before it syncs or publishes.
    let mut writer = io::BufWriter::with_capacity(8 * 1024, writer);
    write!(writer, "{{\"version\":{},\"id\":", original.version)?;
    serde_json::to_writer(&mut writer, &original.id).map_err(io::Error::other)?;
    write!(
        writer,
        ",\"startedAtMs\":{},\"endedAtMs\":{},\"limited\":{},\"entries\":[",
        original.started_at_ms, ended_at_ms, limited
    )?;
    reader::scan_verified(
        input,
        original,
        Some(&mut |index, pair| {
            if index != 0 {
                writer.write_all(b",")?;
            }
            serde_json::to_writer(&mut writer, &pair).map_err(io::Error::other)
        }),
    )?;
    writer.write_all(b"]}")?;
    writer.flush()
}

/// Reuse only a completed conversion of this pending PCM track. Existing
/// outputs are compared with bounded buffers before the journal is consumed.
fn verified_existing_wav(
    path: &Path,
    rate: u32,
    pcm_length: u64,
    pcm: &mut fs::File,
) -> io::Result<bool> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "invalid existing audio");
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != pcm_length + 44
    {
        return Err(invalid());
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        #[cfg(target_os = "macos")]
        options.custom_flags(0x100); // O_NOFOLLOW
        #[cfg(not(target_os = "macos"))]
        options.custom_flags(0x20000);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let mut file = options.open(path)?;
    let opened = file.metadata()?;
    if !opened.is_file() || opened.file_type().is_symlink() || opened.len() != metadata.len() {
        return Err(invalid());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.dev() != opened.dev() || metadata.ino() != opened.ino() {
            return Err(invalid());
        }
    }
    let mut header = [0_u8; 44];
    file.read_exact(&mut header)?;
    if header != wav_header(rate, pcm_length as u32) {
        return Err(invalid());
    }
    let mut wave_bytes = [0_u8; 8192];
    let mut pcm_bytes = [0_u8; 8192];
    let mut remaining = pcm_length as usize;
    while remaining > 0 {
        let count = remaining.min(wave_bytes.len());
        file.read_exact(&mut wave_bytes[..count])?;
        pcm.read_exact(&mut pcm_bytes[..count])?;
        if wave_bytes[..count] != pcm_bytes[..count] {
            return Err(invalid());
        }
        remaining -= count;
    }
    let mut tail = [0_u8; 1];
    if file.read(&mut tail)? != 0 || pcm.read(&mut tail)? != 0 {
        return Err(invalid());
    }
    Ok(true)
}

fn wav_header(rate: u32, length: u32) -> [u8; 44] {
    let mut header = [0_u8; 44];
    header[0..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&(length + 36).to_le_bytes());
    header[8..16].copy_from_slice(b"WAVEfmt ");
    header[16..20].copy_from_slice(&16_u32.to_le_bytes());
    header[20..22].copy_from_slice(&1_u16.to_le_bytes());
    header[22..24].copy_from_slice(&1_u16.to_le_bytes());
    header[24..28].copy_from_slice(&rate.to_le_bytes());
    header[28..32].copy_from_slice(&(rate * 2).to_le_bytes());
    header[32..34].copy_from_slice(&2_u16.to_le_bytes());
    header[34..36].copy_from_slice(&16_u16.to_le_bytes());
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&length.to_le_bytes());
    header
}

#[cfg(test)]
mod tests;
