//! Local opt-in transcript history. Files contain user content and are never logged.
use crate::core::audio_input::AudioSource;
use crate::core::models::SubtitlePair;
use crate::core::session_archive::{
    SavedTranscript, TranscriptExportOptions, TranscriptPage, TranscriptPageEntry,
    TranscriptWriter, AUDIO_BYTE_LIMIT, TRANSCRIPT_PAGE_SIZE,
};
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
        if file.metadata()?.len() > crate::core::session_archive::AUDIO_BYTE_LIMIT as u64 + 44 {
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
        crate::core::session_archive::TranscriptExportFormat::Txt => "txt",
        crate::core::session_archive::TranscriptExportFormat::Markdown => "md",
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
mod tests {
    use super::*;
    use crate::core::models::SubtitlePair;

    fn history_with_saved_pair() -> (tempfile::TempDir, SessionHistory, String) {
        let directory = tempfile::tempdir().unwrap();
        let history = SessionHistory::new(directory.path().join("history"), false);
        let id = uuid::Uuid::new_v4().to_string();
        history.begin(&id, 100).unwrap();
        history
            .append_pair(
                &id,
                &SubtitlePair::new("Synthetic source".into(), "译文".into(), 150),
            )
            .unwrap();
        history.finalize(&id, 200, false).unwrap();
        (directory, history, id)
    }

    #[test]
    fn concurrent_private_exports_have_independent_file_ownership() {
        let (directory, history, id) = history_with_saved_pair();
        let exports = directory.path().join("exports");
        let (first, second) = std::thread::scope(|scope| {
            let export = || {
                history
                    .export_transcript_to_directory(
                        &id,
                        TranscriptExportOptions::default(),
                        &exports,
                    )
                    .unwrap()
            };
            let first = scope.spawn(export);
            let second = scope.spawn(export);
            (first.join().unwrap(), second.join().unwrap())
        });
        assert_ne!(first, second);
        let bytes = fs::read(&first).unwrap();
        fs::remove_file(first).unwrap();
        assert_eq!(fs::read(&second).unwrap(), bytes);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&exports).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(&second).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn title_updates_are_private_bounded_and_do_not_rewrite_saved_content() {
        let (_directory, history, id) = history_with_saved_pair();
        let transcript = fs::read(history.path(&id, "json").unwrap()).unwrap();
        assert!(history.list().unwrap()[0].title.is_none());
        history.rename(&id, "  日本語 / 中文 😀  ").unwrap();
        assert_eq!(
            history.list().unwrap()[0].title.as_deref(),
            Some("日本語 / 中文 😀")
        );
        assert_eq!(
            fs::read(history.path(&id, "json").unwrap()).unwrap(),
            transcript
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(history.path(&id, "meta.json").unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600,
            );
        }
        let title = "😀".repeat(MAX_TITLE_CHARACTERS);
        history.rename(&id, &title).unwrap();
        for invalid in ["😀".repeat(MAX_TITLE_CHARACTERS + 1), "Line\nbreak".into()] {
            assert!(history.rename(&id, &invalid).is_err());
            assert_eq!(
                history.list().unwrap()[0].title.as_deref(),
                Some(title.as_str())
            );
        }
        assert!(history.rename("../escape", "Title").is_err());
        assert!(history
            .rename(&uuid::Uuid::new_v4().to_string(), "Title")
            .is_err());
        assert_eq!(fs::read_dir(&history.directory).unwrap().count(), 2);
        history.rename(&id, " \t ").unwrap();
        assert!(history.list().unwrap()[0].title.is_none());
        assert!(!history.path(&id, "meta.json").unwrap().exists());
        history.rename(&id, "Delete with session").unwrap();
        history.delete(&id).unwrap();
        assert_eq!(fs::read_dir(&history.directory).unwrap().count(), 0);
    }

    #[test]
    fn damaged_or_oversized_titles_do_not_hide_readable_sessions() {
        let (_directory, history, id) = history_with_saved_pair();
        for contents in [
            b"invalid json".to_vec(),
            br#"{"version":2,"title":"Unsupported"}"#.to_vec(),
            vec![b'x'; MAX_TITLE_FILE_BYTES as usize + 1],
        ] {
            fs::write(history.path(&id, "meta.json").unwrap(), contents).unwrap();
            let items = history.list().unwrap();
            assert_eq!(items.len(), 1);
            assert!(items[0].title.is_none());
            assert_eq!(history.page(&id, "", 0).unwrap().total, 1);
        }
        history.rename(&id, "Recovered title").unwrap();
        assert_eq!(
            history.list().unwrap()[0].title.as_deref(),
            Some("Recovered title")
        );
    }

    #[test]
    fn failed_title_destination_leaves_transcript_and_no_temporary_copy() {
        let (_directory, history, id) = history_with_saved_pair();
        let destination = history.path(&id, "meta.json").unwrap();
        fs::create_dir(&destination).unwrap();
        let before = history.export_transcript(&id).unwrap();
        assert!(history.rename(&id, "Cannot replace a directory").is_err());
        assert!(destination.is_dir());
        assert_eq!(history.export_transcript(&id).unwrap(), before);
        assert_eq!(fs::read_dir(&history.directory).unwrap().count(), 2);
    }

    #[test]
    fn audio_only_and_recovered_sessions_can_have_titles() {
        let directory = tempfile::tempdir().unwrap();
        let history = SessionHistory::new(directory.path().join("history"), false);
        let id = uuid::Uuid::new_v4().to_string();
        history.begin(&id, 100).unwrap();
        history
            .append_pcm_from(&id, AudioSource::System, 16000, &[1, 0])
            .unwrap();
        history.rename(&id, "Audio only").unwrap();
        let items = history.list().unwrap();
        assert_eq!(items[0].count, 0);
        assert!(items[0].has_audio);
        assert_eq!(items[0].title.as_deref(), Some("Audio only"));
        history.finalize(&id, 200, false).unwrap();
        let restored = SessionHistory::new(history.directory.clone(), false);
        assert_eq!(
            restored.list().unwrap()[0].title.as_deref(),
            Some("Audio only")
        );
        restored.delete(&id).unwrap();
        assert_eq!(fs::read_dir(&history.directory).unwrap().count(), 0);
    }

    #[test]
    fn saved_transcript_export_options_use_the_shared_formatter() {
        use crate::core::session_archive::{TranscriptExportContent, TranscriptExportFormat};
        let (_directory, history, id) = history_with_saved_pair();
        let options = TranscriptExportOptions {
            format: TranscriptExportFormat::Markdown,
            content: TranscriptExportContent::Translation,
            include_timestamps: false,
        };
        let text = String::from_utf8(
            history
                .export_transcript_with_options(&id, options)
                .unwrap(),
        )
        .unwrap();
        assert!(text.starts_with("# Mimi transcript"));
        assert!(text.contains("译文"));
        assert!(!text.contains("Synthetic source"));
        assert!(!text.contains("1970-01-01"));
    }

    #[derive(Default)]
    struct TranscriptWriter {
        bytes: Vec<u8>,
        writes: usize,
        fail_after: Option<usize>,
        fail_flush: bool,
    }

    impl Write for TranscriptWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.writes += 1;
            let count = self.fail_after.map_or(bytes.len(), |limit| {
                bytes.len().min(limit.saturating_sub(self.bytes.len()))
            });
            if count == 0 && !bytes.is_empty() {
                return Err(io::Error::other("synthetic write failure"));
            }
            self.bytes.extend_from_slice(&bytes[..count]);
            Ok(count)
        }

        fn flush(&mut self) -> io::Result<()> {
            if self.fail_flush {
                Err(io::Error::other("synthetic flush failure"))
            } else {
                Ok(())
            }
        }
    }

    fn synthetic_final_transcript(count: usize) -> SavedTranscript {
        SavedTranscript {
            version: 1,
            id: uuid::Uuid::new_v4().to_string(),
            started_at_ms: 100,
            ended_at_ms: 200,
            limited: false,
            entries: (0..count)
                .map(|index| {
                    let mut pair = SubtitlePair::new(
                        "Synthetic \\\"\n日本語".into(),
                        "译文\t😀".into(),
                        index as u64 + 100,
                    );
                    if index % 2 != 0 {
                        pair.audio_source = AudioSource::Microphone;
                    }
                    pair
                })
                .collect(),
        }
    }

    struct ReadFailureAfterRewind {
        input: io::Cursor<Vec<u8>>,
        fail_at: usize,
        rewound: bool,
        failures: usize,
    }

    impl ReadFailureAfterRewind {
        fn new(bytes: &[u8], fail_at: usize) -> Self {
            Self {
                input: io::Cursor::new(bytes.to_vec()),
                fail_at,
                rewound: false,
                failures: 0,
            }
        }
    }

    impl Read for ReadFailureAfterRewind {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            if output.is_empty() {
                return Ok(0);
            }
            let input = self.fill_buf()?;
            let count = input.len().min(output.len());
            output[..count].copy_from_slice(&input[..count]);
            self.consume(count);
            Ok(count)
        }
    }

    impl BufRead for ReadFailureAfterRewind {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            let available = if self.rewound {
                let available = self.fail_at.saturating_sub(self.input.position() as usize);
                if available == 0 {
                    self.failures += 1;
                    return Err(io::Error::other("synthetic second-pass read failure"));
                }
                available
            } else {
                usize::MAX
            };
            let input = self.input.fill_buf()?;
            Ok(&input[..input.len().min(available)])
        }

        fn consume(&mut self, amount: usize) {
            self.input.consume(amount);
        }
    }

    impl Seek for ReadFailureAfterRewind {
        fn seek(&mut self, position: io::SeekFrom) -> io::Result<u64> {
            let offset = self.input.seek(position)?;
            if matches!(position, io::SeekFrom::Start(0)) {
                self.rewound = true;
            }
            Ok(offset)
        }
    }

    #[test]
    fn second_pass_read_failures_do_not_publish_or_return_shortened_history() {
        let directory = tempfile::tempdir().unwrap();
        let history = SessionHistory::new(directory.path().join("history"), false);
        let mut saved = synthetic_final_transcript(3);
        for pair in &mut saved.entries {
            pair.created_at_ms += 300;
        }
        history.begin(&saved.id, saved.started_at_ms).unwrap();
        let mut header = serde_json::to_value(&saved).unwrap();
        header["entries"] = serde_json::json!([]);
        let mut bytes = serde_json::to_vec(&header).unwrap();
        bytes.push(b'\n');
        let mut boundaries = Vec::new();
        for pair in &saved.entries {
            boundaries.push(bytes.len());
            serde_json::to_writer(&mut bytes, pair).unwrap();
            bytes.push(b'\n');
        }
        let journal = history.path(&saved.id, "jsonl").unwrap();
        fs::write(&journal, &bytes).unwrap();
        let final_path = history.path(&saved.id, "json").unwrap();
        let exports = directory.path().join("exports");
        for fail_at in boundaries {
            let mut input = ReadFailureAfterRewind::new(&bytes, fail_at);
            let original = reader::scan(&mut input, &saved.id, None).unwrap();
            assert_eq!(original.entries, saved.entries.len());
            input.rewind().unwrap();
            let error = persist_final_transcript(
                &history.directory,
                &final_path,
                &original,
                &mut input,
                900,
                true,
            )
            .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(input.failures > 0);
            assert!(!final_path.exists());
            assert_eq!(fs::read(&journal).unwrap(), bytes);
            assert_eq!(fs::read_dir(&history.directory).unwrap().count(), 1);

            let mut input = ReadFailureAfterRewind::new(&bytes, fail_at);
            let options = TranscriptExportOptions::default();
            let error = write_private_export(&exports, options, |writer| {
                write_transcript(&mut input, &saved.id, options, writer)
            })
            .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(input.failures > 0);
            assert_eq!(fs::read_dir(&exports).unwrap().count(), 0);

            for query in ["", "synthetic"] {
                let mut input = ReadFailureAfterRewind::new(&bytes, fail_at);
                let error = read_page(&mut input, &saved.id, query, 1)
                    .err()
                    .expect("a changed second pass must reject the page");
                assert_eq!(error.kind(), io::ErrorKind::InvalidData);
                assert!(input.failures > 0);
            }
        }
        // A successful finalization still applies the new end/limit values;
        // comparison uses the original header, not those output overrides.
        assert!(history.finalize(&saved.id, 900, true).unwrap());
        let finalized = history.read(&saved.id).unwrap();
        assert_eq!((finalized.ended_at_ms, finalized.limited), (900, true));
        assert_eq!(finalized.entries.len(), saved.entries.len());
    }

    #[test]
    fn streamed_pages_and_exports_preserve_saved_and_recovered_content() {
        use crate::core::session_archive::{TranscriptExportContent, TranscriptExportFormat};
        let directory = tempfile::tempdir().unwrap();
        let history = SessionHistory::new(directory.path().join("history"), false);
        let mut saved = synthetic_final_transcript(137);
        saved.limited = true;
        for (index, pair) in saved.entries.iter_mut().enumerate() {
            pair.source = format!(
                "Synthetic {index} {}",
                if index % 3 == 0 { "Needle" } else { "sample" }
            );
            pair.translation = format!("译文 **ÄPFEL**\n😀 {index}");
            // Confirmation order is independent of timestamps.
            pair.created_at_ms = 100 + (index % 7) as u64;
        }
        history.begin(&saved.id, saved.started_at_ms).unwrap();
        for extension in ["jsonl", "json"] {
            let path = history.path(&saved.id, extension).unwrap();
            let mut file = io::BufWriter::new(fs::File::create(path).unwrap());
            if extension == "json" {
                serde_json::to_writer(&mut file, &saved).unwrap();
            } else {
                let mut header = serde_json::to_value(&saved).unwrap();
                header["entries"].as_array_mut().unwrap().truncate(7);
                serde_json::to_writer(&mut file, &header).unwrap();
                for pair in &saved.entries[7..] {
                    file.write_all(b"\n{broken JSON}\n\n").unwrap();
                    serde_json::to_writer(&mut file, pair).unwrap();
                }
                file.write_all(b"\n{\"source\":\"unfinished").unwrap();
            }
            file.flush().unwrap();
            for query in ["", "  NEEDLE  ", "äpfel", "missing"] {
                for page in [0, 1, 2, usize::MAX] {
                    assert_eq!(
                        serde_json::to_value(history.page(&saved.id, query, page).unwrap())
                            .unwrap(),
                        serde_json::to_value(saved.page(query, page)).unwrap(),
                        "{extension}: query={query}, page={page}"
                    );
                }
            }
            for format in [
                TranscriptExportFormat::Txt,
                TranscriptExportFormat::Markdown,
            ] {
                for content in [
                    TranscriptExportContent::Bilingual,
                    TranscriptExportContent::Source,
                    TranscriptExportContent::Translation,
                ] {
                    for include_timestamps in [false, true] {
                        let options = TranscriptExportOptions {
                            format,
                            content,
                            include_timestamps,
                        };
                        let expected = saved.export_with_options(options).unwrap().into_bytes();
                        assert_eq!(
                            history
                                .export_transcript_with_options(&saved.id, options)
                                .unwrap(),
                            expected
                        );
                        let exported = history
                            .export_transcript_to_directory(
                                &saved.id,
                                options,
                                &directory.path().join("exports"),
                            )
                            .unwrap();
                        assert_eq!(fs::read(&exported).unwrap(), expected);
                        fs::remove_file(exported).unwrap();
                    }
                }
            }
        }
    }

    #[test]
    fn transcript_export_propagates_write_errors_and_does_not_publish_invalid_content() {
        let (directory, history, id) = history_with_saved_pair();
        let mut writer = TranscriptWriter {
            fail_after: Some(1),
            ..Default::default()
        };
        let error = history
            .export_transcript_to_writer(&id, TranscriptExportOptions::default(), &mut writer)
            .unwrap_err();
        assert_eq!(error.to_string(), "synthetic write failure");
        fs::write(history.path(&id, "json").unwrap(), b"{\"entries\":[]}").unwrap();
        let exports = directory.path().join("exports");
        assert!(history
            .export_transcript_to_directory(&id, TranscriptExportOptions::default(), &exports)
            .is_err());
        assert_eq!(fs::read_dir(exports).unwrap().count(), 0);
    }

    #[test]
    fn final_transcript_write_batches_small_tokens_and_preserves_bytes() {
        let saved = synthetic_final_transcript(512);
        let mut writer = TranscriptWriter::default();
        write_fixture_final_transcript(&mut writer, &saved).unwrap();
        assert_eq!(writer.bytes, serde_json::to_vec(&saved).unwrap());
        assert!(
            writer.writes < saved.entries.len() / 4,
            "finalization should batch JSON tokens, writes={}",
            writer.writes,
        );
    }

    #[test]
    fn final_transcript_write_propagates_tail_write_and_flush_failures() {
        let saved = synthetic_final_transcript(1);
        let expected = serde_json::to_vec(&saved).unwrap();
        assert!(expected.len() < 8192);
        for accepted in [0, expected.len() - 1] {
            let mut writer = TranscriptWriter {
                fail_after: Some(accepted),
                ..Default::default()
            };
            assert!(write_fixture_final_transcript(&mut writer, &saved).is_err());
            assert_eq!(writer.bytes, expected[..accepted]);
        }
        let mut writer = TranscriptWriter {
            fail_flush: true,
            ..Default::default()
        };
        assert!(write_fixture_final_transcript(&mut writer, &saved).is_err());
        assert_eq!(writer.bytes, expected);
    }

    fn write_fixture_final_transcript(
        writer: &mut impl Write,
        saved: &SavedTranscript,
    ) -> io::Result<()> {
        let mut input = io::Cursor::new(serde_json::to_vec(saved)?);
        let metadata = reader::scan(&mut input, &saved.id, None)?;
        input.rewind()?;
        write_final_transcript(
            writer,
            &metadata,
            &mut input,
            saved.ended_at_ms,
            saved.limited,
        )
    }

    fn assert_metadata_matches_transcript(history: &SessionHistory, id: &str) {
        let saved = history.read(id).unwrap();
        let metadata = history.read_metadata(id).unwrap();
        assert_eq!(metadata.id, saved.id);
        assert_eq!(metadata.started_at_ms, saved.started_at_ms);
        assert_eq!(metadata.ended_at_ms, saved.ended_at_ms);
        assert_eq!(metadata.limited, saved.limited);
        assert_eq!(metadata.entries, saved.entries.len());
    }

    #[test]
    fn list_metadata_preserves_saved_and_interrupted_journal_semantics() {
        let directory = tempfile::tempdir().unwrap();
        let history = SessionHistory::new(directory.path().join("history"), false);
        let id = uuid::Uuid::new_v4().to_string();
        history.begin(&id, 100).unwrap();
        for at in [150, 125, 180] {
            let mut pair =
                SubtitlePair::new("Synthetic \\\"\n日本語".into(), "译文\t😀".into(), at);
            pair.audio_source = AudioSource::Microphone;
            history.append_pair(&id, &pair).unwrap();
        }
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(history.path(&id, "jsonl").unwrap())
            .unwrap();
        file.write_all(b"\n{invalid json}\n{\"source\":5}\n")
            .unwrap();
        history
            .append_pair(
                &id,
                &SubtitlePair::new("Synthetic tail".into(), "译文".into(), 200),
            )
            .unwrap();
        assert_metadata_matches_transcript(&history, &id);
        assert_eq!(history.list().unwrap()[0].ended_at_ms, 200);

        // Invalid UTF-8 stops recovery at this line, even if valid JSON follows.
        file.write_all(&[b'{', 0xff, b'\n']).unwrap();
        history
            .append_pair(
                &id,
                &SubtitlePair::new("Synthetic ignored".into(), "译文".into(), 300),
            )
            .unwrap();
        assert_metadata_matches_transcript(&history, &id);
        assert_eq!(history.list().unwrap()[0].count, 4);
        assert!(history.finalize(&id, 250, true).unwrap());
        assert_metadata_matches_transcript(&history, &id);
        let listed = history.list().unwrap();
        assert_eq!(
            (listed[0].count, listed[0].ended_at_ms, listed[0].limited),
            (4, 250, true)
        );
    }

    #[test]
    fn list_metadata_preserves_header_and_appended_entry_count_limits() {
        let directory = tempfile::tempdir().unwrap();
        let history = SessionHistory::new(directory.path().join("history"), false);
        let id = uuid::Uuid::new_v4().to_string();
        history.begin(&id, 100).unwrap();
        let count = crate::core::session_archive::TRANSCRIPT_COUNT_LIMIT;
        let pair = SubtitlePair::new("Synthetic".into(), "译文".into(), 200);
        let saved = SavedTranscript {
            version: 1,
            id: id.clone(),
            started_at_ms: 100,
            ended_at_ms: 150,
            limited: true,
            entries: vec![pair.clone(); count + 1],
        };
        let path = history.path(&id, "jsonl").unwrap();
        let mut file = io::BufWriter::new(fs::File::create(&path).unwrap());
        serde_json::to_writer(&mut file, &saved).unwrap();
        file.write_all(b"\n").unwrap();
        serde_json::to_writer(&mut file, &pair).unwrap();
        file.flush().unwrap();
        assert_metadata_matches_transcript(&history, &id);
        let metadata = history.read_metadata(&id).unwrap();
        assert_eq!((metadata.entries, metadata.ended_at_ms), (count + 1, 150));

        let mut file = io::BufWriter::new(fs::File::create(path).unwrap());
        serde_json::to_writer(
            &mut file,
            &SavedTranscript {
                entries: Vec::new(),
                ..saved
            },
        )
        .unwrap();
        for index in 0..=count {
            file.write_all(b"\n").unwrap();
            let pair = SubtitlePair::new("Synthetic".into(), "译文".into(), index as u64 + 200);
            serde_json::to_writer(&mut file, &pair).unwrap();
        }
        file.flush().unwrap();
        assert_metadata_matches_transcript(&history, &id);
        let metadata = history.read_metadata(&id).unwrap();
        assert_eq!(
            (metadata.entries, metadata.ended_at_ms),
            (count, count as u64 + 199)
        );
    }

    #[test]
    fn list_metadata_rejects_the_same_invalid_saved_fields_as_full_reads() {
        let directory = tempfile::tempdir().unwrap();
        let history = SessionHistory::new(directory.path().join("history"), false);
        let id = uuid::Uuid::new_v4().to_string();
        history.begin(&id, 100).unwrap();
        let baseline = serde_json::json!({
            "version": 1, "id": id, "startedAtMs": 100, "endedAtMs": 200, "limited": false,
            "entries": [{"source": "Synthetic", "translation": "译文", "createdAt": 150}]
        });
        let mut invalid = Vec::new();
        for (field, replacement) in [
            ("source", serde_json::json!(5)),
            ("translation", serde_json::Value::Null),
            ("createdAt", serde_json::json!(-1)),
            ("audioSource", serde_json::json!("invalid")),
        ] {
            let mut value = baseline.clone();
            value["entries"][0][field] = replacement;
            invalid.push(value);
        }
        for field in ["source", "translation", "createdAt"] {
            let mut value = baseline.clone();
            value["entries"][0].as_object_mut().unwrap().remove(field);
            invalid.push(value);
        }
        for (field, replacement) in [
            ("version", serde_json::json!(2)),
            ("id", serde_json::json!("other")),
            ("entries", serde_json::json!({})),
        ] {
            let mut value = baseline.clone();
            value[field] = replacement;
            invalid.push(value);
        }
        let path = history.path(&id, "json").unwrap();
        for value in invalid {
            fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
            assert!(history.read(&id).is_err());
            assert!(history.read_metadata(&id).is_err());
            assert!(history.list().unwrap().is_empty());
        }
        fs::write(&path, serde_json::to_vec(&baseline).unwrap()).unwrap();
        assert_metadata_matches_transcript(&history, &id);
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(MAX_TRANSCRIPT_FILE_BYTES + 1)
            .unwrap();
        assert!(history.read(&id).is_err());
        assert!(history.read_metadata(&id).is_err());
        assert!(history.list().unwrap().is_empty());
    }

    #[test]
    fn saves_lists_reads_and_deletes_one_session_without_path_escape() {
        let directory = tempfile::tempdir().unwrap();
        let history = SessionHistory::new(directory.path().join("history"), false);
        let id = uuid::Uuid::new_v4().to_string();
        history.begin(&id, 100).unwrap();
        history
            .append_pair(&id, &SubtitlePair::new("source".into(), "译文".into(), 150))
            .unwrap();
        history.append_pcm(&id, 16_000, &[1, 0, 2, 0]).unwrap();
        assert!(history.finalize(&id, 200, false).unwrap());
        assert_eq!(history.list().unwrap()[0].count, 1);
        assert!(history.list().unwrap()[0].has_audio);
        assert_eq!(history.page(&id, "译文", 0).unwrap().total, 1);
        assert_eq!(&history.audio(&id).unwrap()[44..], &[1, 0, 2, 0]);
        assert!(history.path(&id, "json").unwrap().exists());
        assert!(history.path(&id, "wav").unwrap().exists());
        assert!(!history.path(&id, "jsonl").unwrap().exists());
        assert!(!history.path(&id, "pcm").unwrap().exists());
        assert!(history.delete("../escape").is_err());
        history.delete(&id).unwrap();
        assert!(history.list().unwrap().is_empty());
    }

    #[test]
    fn interrupted_session_is_readable_from_incremental_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history");
        let history = SessionHistory::new(path.clone(), false);
        let id = uuid::Uuid::new_v4().to_string();
        history.begin(&id, 100).unwrap();
        history
            .append_pair(&id, &SubtitlePair::new("spoken".into(), "译文".into(), 150))
            .unwrap();
        history.append_pcm(&id, 16_000, &[1, 0, 2, 0]).unwrap();
        drop(history);

        let restored = SessionHistory::new(path, false);
        let item = &restored.list().unwrap()[0];
        assert_eq!((item.count, item.has_audio), (1, true));
        assert_eq!(restored.page(&id, "spoken", 0).unwrap().total, 1);
        let wav = restored.audio(&id).unwrap();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[44..], &[1, 0, 2, 0]);
        restored.clear_text(&id).unwrap();
        assert_eq!(restored.list().unwrap()[0].count, 0);
        restored.delete(&id).unwrap();
        assert!(restored.list().unwrap().is_empty());
    }

    #[test]
    fn finalizes_audio_only_from_disk_and_discards_empty_sessions() {
        let directory = tempfile::tempdir().unwrap();
        let history = SessionHistory::new(directory.path().join("history"), false);
        let empty_id = uuid::Uuid::new_v4().to_string();
        history.begin(&empty_id, 100).unwrap();
        assert!(!history.finalize(&empty_id, 200, false).unwrap());
        assert!(!history.path(&empty_id, "jsonl").unwrap().exists());

        let audio_id = uuid::Uuid::new_v4().to_string();
        history.begin(&audio_id, 300).unwrap();
        history.append_pcm(&audio_id, 24_000, &[3, 0]).unwrap();
        assert!(history.finalize(&audio_id, 400, false).unwrap());
        let item = &history.list().unwrap()[0];
        assert_eq!((item.count, item.has_audio), (0, true));
        let wav = history.audio(&audio_id).unwrap();
        assert_eq!(&wav[24..28], &24_000_u32.to_le_bytes());
        assert_eq!(&wav[44..], &[3, 0]);
        assert!(!history.path(&audio_id, "pcm").unwrap().exists());
    }

    #[test]
    fn recovers_confirmed_pairs_before_a_truncated_final_write() {
        let directory = tempfile::tempdir().unwrap();
        let history = SessionHistory::new(directory.path().join("history"), false);
        let id = uuid::Uuid::new_v4().to_string();
        history.begin(&id, 100).unwrap();
        history
            .append_pair(&id, &SubtitlePair::new("source".into(), "译文".into(), 150))
            .unwrap();
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(history.path(&id, "jsonl").unwrap())
            .unwrap();
        file.write_all(&[b'{', 0xff]).unwrap();
        assert_eq!(history.page(&id, "", 0).unwrap().total, 1);
        assert!(history.finalize(&id, 200, false).unwrap());
        assert_eq!(history.page(&id, "", 0).unwrap().total, 1);
    }
    #[test]
    fn dual_inputs_remain_separate_through_journal_finalization_and_export() {
        let directory = tempfile::tempdir().unwrap();
        let history = SessionHistory::new(directory.path().join("history"), false);
        let id = uuid::Uuid::new_v4().to_string();
        history.begin(&id, 100).unwrap();
        let system = SubtitlePair::new(
            "Synthetic remote".into(),
            "Synthetic remote translation".into(),
            150,
        );
        let mut microphone = SubtitlePair::new(
            "Synthetic local".into(),
            "Synthetic local translation".into(),
            160,
        );
        microphone.audio_source = AudioSource::Microphone;
        history.append_pair(&id, &system).unwrap();
        history.append_pair(&id, &microphone).unwrap();
        history
            .append_pcm_from(&id, AudioSource::System, 16_000, &[1, 0, 2, 0])
            .unwrap();
        history
            .append_pcm_from(&id, AudioSource::Microphone, 24_000, &[3, 0, 4, 0])
            .unwrap();
        for finalized in [false, true] {
            if finalized {
                assert!(history.finalize(&id, 200, false).unwrap());
            }
            let items = history.list().unwrap();
            assert_eq!(items.len(), 1);
            assert_eq!(
                items[0].audio_sources,
                [AudioSource::System, AudioSource::Microphone]
            );
            assert_eq!(items[0].count, 2);
            let page = history.page(&id, "", 0).unwrap();
            assert_eq!(page.entries[0].audio_source, AudioSource::Microphone);
            assert_eq!(page.entries[1].audio_source, AudioSource::System);
            let wire = serde_json::to_value(&page).unwrap();
            assert_eq!(wire["entries"][0]["audioSource"], "microphone");
            assert_eq!(
                history.audio_selected(&id, None).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
            let system_wav = history.audio_from(&id, AudioSource::System).unwrap();
            let mic_wav = history.audio_from(&id, AudioSource::Microphone).unwrap();
            assert_eq!(&system_wav[24..28], &16_000_u32.to_le_bytes());
            assert_eq!(&system_wav[44..], &[1, 0, 2, 0]);
            assert_eq!(&mic_wav[24..28], &24_000_u32.to_le_bytes());
            assert_eq!(&mic_wav[44..], &[3, 0, 4, 0]);
            let transcript = String::from_utf8(history.export_transcript(&id).unwrap()).unwrap();
            assert!(transcript.contains("System audio"));
            assert!(transcript.contains("Microphone"));
        }
        history.delete(&id).unwrap();
        assert_eq!(fs::read_dir(&history.directory).unwrap().count(), 0);
    }

    #[test]
    fn microphone_only_history_is_selected_without_a_system_fallback() {
        let directory = tempfile::tempdir().unwrap();
        let history = SessionHistory::new(directory.path().join("history"), false);
        let id = uuid::Uuid::new_v4().to_string();
        history.begin(&id, 100).unwrap();
        history
            .append_pcm_from(&id, AudioSource::Microphone, 24_000, &[3, 0])
            .unwrap();
        assert_eq!(
            history.audio_sources(&id).unwrap(),
            [AudioSource::Microphone]
        );
        assert_eq!(&history.audio_selected(&id, None).unwrap()[44..], &[3, 0]);
        assert!(history
            .audio_selected(&id, Some(AudioSource::System))
            .is_err());
        assert!(history.finalize(&id, 200, false).unwrap());
        assert_eq!(&history.audio_selected(&id, None).unwrap()[44..], &[3, 0]);
        history.delete(&id).unwrap();
    }

    #[test]
    fn disabling_recording_and_discard_remove_every_current_audio_track() {
        let directory = tempfile::tempdir().unwrap();
        let history = SessionHistory::new(directory.path().join("history"), false);
        for discard in [false, true] {
            let id = uuid::Uuid::new_v4().to_string();
            history.begin(&id, 100).unwrap();
            for source in [AudioSource::System, AudioSource::Microphone] {
                history
                    .append_pcm_from(&id, source, 16_000, &[1, 0])
                    .unwrap();
            }
            if discard {
                history.discard(&id).unwrap();
                assert!(!history.path(&id, "jsonl").unwrap().exists());
            } else {
                history.clear_audio(&id).unwrap();
                assert!(!history.finalize(&id, 200, false).unwrap());
            }
            assert!(history.audio_sources(&id).unwrap().is_empty());
            assert!(history.audio_from(&id, AudioSource::System).is_err());
            assert!(history.audio_from(&id, AudioSource::Microphone).is_err());
        }
        assert_eq!(fs::read_dir(&history.directory).unwrap().count(), 0);
    }

    #[test]
    fn incompatible_audio_chunks_cannot_corrupt_either_track() {
        let directory = tempfile::tempdir().unwrap();
        let history = SessionHistory::new(directory.path().join("history"), false);
        let id = uuid::Uuid::new_v4().to_string();
        history.begin(&id, 100).unwrap();
        history
            .append_pcm_from(&id, AudioSource::System, 16_000, &[1, 0])
            .unwrap();
        assert!(history
            .append_pcm_from(&id, AudioSource::System, 24_000, &[2, 0])
            .is_err());
        assert!(history
            .append_pcm_from(&id, AudioSource::Microphone, 16_000, &[3])
            .is_err());
        assert!(history
            .append_pcm_from(&id, AudioSource::Microphone, 48_000, &[4, 0])
            .is_err());
        assert_eq!(history.audio_sources(&id).unwrap(), [AudioSource::System]);
        assert_eq!(&history.audio_selected(&id, None).unwrap()[44..], &[1, 0]);
    }

    #[test]
    fn audio_size_limit_applies_to_both_tracks_together() {
        let directory = tempfile::tempdir().unwrap();
        let history = SessionHistory::new(directory.path().join("history"), false);
        let id = uuid::Uuid::new_v4().to_string();
        history.begin(&id, 100).unwrap();
        history
            .append_pcm_from(&id, AudioSource::System, 16_000, &[1, 0])
            .unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(history.audio_path(&id, AudioSource::System, "pcm").unwrap())
            .unwrap()
            .set_len(AUDIO_BYTE_LIMIT as u64 + 4)
            .unwrap();
        assert!(history
            .append_pcm_from(&id, AudioSource::Microphone, 16_000, &[2, 0])
            .is_err());
        assert_eq!(history.audio_sources(&id).unwrap(), [AudioSource::System]);
    }

    fn pending_audio(history: &SessionHistory, source: AudioSource, rate: u32) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        history.begin(&id, 100).unwrap();
        history
            .append_pair(
                &id,
                &SubtitlePair::new(
                    "Synthetic original".into(),
                    "Synthetic translation".into(),
                    150,
                ),
            )
            .unwrap();
        history
            .append_pcm_from(&id, source, rate, &[1, 0, 2, 0])
            .unwrap();
        id
    }

    fn synthetic_wav(rate: u32) -> Vec<u8> {
        let mut wav = wav_header(rate, 4).to_vec();
        wav.extend_from_slice(&[1, 0, 2, 0]);
        wav
    }

    fn assert_pending_audio(history: &SessionHistory, id: &str, source: AudioSource, rate: u32) {
        let mut expected = rate.to_le_bytes().to_vec();
        expected.extend_from_slice(&[1, 0, 2, 0]);
        assert_eq!(
            fs::read(history.audio_path(id, source, "pcm").unwrap()).unwrap(),
            expected
        );
        assert!(history.path(id, "jsonl").unwrap().is_file());
        assert!(!history.path(id, "json").unwrap().exists());
    }

    #[test]
    fn pending_audio_export_uses_pcm_after_a_failed_wav_finalization() {
        let directory = tempfile::tempdir().unwrap();
        let history = SessionHistory::new(directory.path().join("history"), false);
        let mut stale_wav = synthetic_wav(16_000);
        stale_wav[44] ^= 1;
        for source in [AudioSource::System, AudioSource::Microphone] {
            for damaged_wav in [
                Vec::new(),
                b"damaged audio".to_vec(),
                synthetic_wav(24_000),
                stale_wav.clone(),
            ] {
                let id = pending_audio(&history, source, 16_000);
                let wav_path = history.audio_path(&id, source, "wav").unwrap();
                fs::write(&wav_path, &damaged_wav).unwrap();
                assert!(history.finalize(&id, 200, false).is_err());
                assert_pending_audio(&history, &id, source, 16_000);

                // A failed conversion must not make its output shadow the
                // complete original journal in playback or manual export.
                assert_eq!(
                    history.audio_selected(&id, Some(source)).unwrap(),
                    synthetic_wav(16_000)
                );
                assert_eq!(fs::read(&wav_path).unwrap(), damaged_wav);
                assert_pending_audio(&history, &id, source, 16_000);
                history.clear_audio(&id).unwrap();
                assert!(history.audio_selected(&id, Some(source)).is_err());
                assert_eq!(history.page(&id, "", 0).unwrap().total, 1);
            }
        }
    }

    #[test]
    fn invalid_pending_pcm_cannot_fall_back_to_an_existing_wav() {
        let directory = tempfile::tempdir().unwrap();
        let history = SessionHistory::new(directory.path().join("history"), false);
        for source in [AudioSource::System, AudioSource::Microphone] {
            let id = pending_audio(&history, source, 16_000);
            fs::write(
                history.audio_path(&id, source, "wav").unwrap(),
                synthetic_wav(16_000),
            )
            .unwrap();
            let pcm_path = history.audio_path(&id, source, "pcm").unwrap();
            let mut odd_pcm = 16_000_u32.to_le_bytes().to_vec();
            odd_pcm.push(1);
            for invalid_pcm in [48_000_u32.to_le_bytes().to_vec(), odd_pcm] {
                fs::write(&pcm_path, &invalid_pcm).unwrap();
                assert!(history.audio_from(&id, source).is_err());
                assert_eq!(fs::read(&pcm_path).unwrap(), invalid_pcm);
            }
        }
    }

    #[test]
    fn finalization_preserves_pcm_when_wav_output_is_not_a_file() {
        let directory = tempfile::tempdir().unwrap();
        let history = SessionHistory::new(directory.path().join("history"), false);
        for source in [AudioSource::System, AudioSource::Microphone] {
            let id = pending_audio(&history, source, 16_000);
            let wav = history.audio_path(&id, source, "wav").unwrap();
            fs::create_dir(&wav).unwrap();
            assert!(history.finalize(&id, 200, false).is_err());
            assert_pending_audio(&history, &id, source, 16_000);
            fs::remove_dir(&wav).unwrap();
            assert!(history.finalize(&id, 200, false).unwrap());
            assert_eq!(
                history.audio_from(&id, source).unwrap(),
                synthetic_wav(16_000)
            );
        }
    }

    #[test]
    fn finalization_rejects_invalid_existing_wav_without_consuming_pcm() {
        let directory = tempfile::tempdir().unwrap();
        let history = SessionHistory::new(directory.path().join("history"), false);
        for source in [AudioSource::System, AudioSource::Microphone] {
            for damage in [
                "empty",
                "header",
                "format",
                "channels",
                "rate",
                "bits",
                "data_length",
                "truncated",
                "trailing",
                "different_pcm_length",
                "different_pcm",
                "oversized",
            ] {
                let id = pending_audio(&history, source, 16_000);
                let path = history.audio_path(&id, source, "wav").unwrap();
                let mut wav = synthetic_wav(16_000);
                match damage {
                    "empty" => wav.clear(),
                    "header" => wav[0] = b'X',
                    "format" => wav[20..22].copy_from_slice(&3_u16.to_le_bytes()),
                    "channels" => wav[22..24].copy_from_slice(&2_u16.to_le_bytes()),
                    "rate" => wav = synthetic_wav(24_000),
                    "bits" => wav[34..36].copy_from_slice(&8_u16.to_le_bytes()),
                    "data_length" => wav[40..44].copy_from_slice(&2_u32.to_le_bytes()),
                    "truncated" => wav.truncate(wav.len() - 2),
                    "trailing" => wav.extend_from_slice(&[3, 0]),
                    "different_pcm_length" => {
                        wav = wav_header(16_000, 2).to_vec();
                        wav.extend_from_slice(&[1, 0]);
                    }
                    "different_pcm" => wav[44] ^= 1,
                    "oversized" => {}
                    _ => unreachable!(),
                }
                fs::write(&path, &wav).unwrap();
                if damage == "oversized" {
                    fs::OpenOptions::new()
                        .write(true)
                        .open(&path)
                        .unwrap()
                        .set_len(AUDIO_BYTE_LIMIT as u64 + 45)
                        .unwrap();
                }
                assert!(
                    history.finalize(&id, 200, false).is_err(),
                    "source={source:?} damage={damage}"
                );
                assert_pending_audio(&history, &id, source, 16_000);
                fs::remove_file(&path).unwrap();
                assert!(history.finalize(&id, 200, false).unwrap());
                assert_eq!(
                    history.audio_from(&id, source).unwrap(),
                    synthetic_wav(16_000)
                );
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn finalization_rejects_wav_symlinks_without_touching_saved_audio() {
        let directory = tempfile::tempdir().unwrap();
        let history = SessionHistory::new(directory.path().join("history"), false);
        let saved_id = pending_audio(&history, AudioSource::System, 16_000);
        history.finalize(&saved_id, 200, false).unwrap();
        let saved_wav = history
            .audio_path(&saved_id, AudioSource::System, "wav")
            .unwrap();
        for source in [AudioSource::System, AudioSource::Microphone] {
            for target in [&saved_wav, &directory.path().join("missing.wav")] {
                let id = pending_audio(&history, source, 16_000);
                let path = history.audio_path(&id, source, "wav").unwrap();
                std::os::unix::fs::symlink(target, &path).unwrap();
                assert!(history.finalize(&id, 200, false).is_err());
                assert_pending_audio(&history, &id, source, 16_000);
                assert!(fs::symlink_metadata(&path)
                    .unwrap()
                    .file_type()
                    .is_symlink());
                assert_eq!(history.audio(&saved_id).unwrap(), synthetic_wav(16_000));
                fs::remove_file(&path).unwrap();
                assert!(history.finalize(&id, 200, false).unwrap());
            }
        }
    }

    #[test]
    fn finalization_reuses_valid_wav_from_an_interrupted_conversion() {
        let directory = tempfile::tempdir().unwrap();
        let history = SessionHistory::new(directory.path().join("history"), false);
        for (source, rate) in [
            (AudioSource::System, 16_000),
            (AudioSource::Microphone, 24_000),
        ] {
            let id = pending_audio(&history, source, rate);
            let path = history.audio_path(&id, source, "wav").unwrap();
            let wav = synthetic_wav(rate);
            fs::write(&path, &wav).unwrap();
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            assert!(history.finalize(&id, 200, false).unwrap());
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
            assert_eq!(history.audio_from(&id, source).unwrap(), wav);
            assert!(!history.audio_path(&id, source, "pcm").unwrap().exists());
            assert!(!history.path(&id, "jsonl").unwrap().exists());
            assert_eq!(history.page(&id, "", 0).unwrap().total, 1);
        }
    }

    #[test]
    fn finalization_compares_existing_wav_payload_through_the_last_chunk() {
        let directory = tempfile::tempdir().unwrap();
        let history = SessionHistory::new(directory.path().join("history"), false);
        let pcm = [1, 0].repeat(8193);
        for (source, rate) in [
            (AudioSource::System, 16_000),
            (AudioSource::Microphone, 24_000),
        ] {
            let id = uuid::Uuid::new_v4().to_string();
            history.begin(&id, 100).unwrap();
            history.append_pcm_from(&id, source, rate, &pcm).unwrap();
            let path = history.audio_path(&id, source, "wav").unwrap();
            let mut wav = wav_header(rate, pcm.len() as u32).to_vec();
            wav.extend_from_slice(&pcm);
            *wav.last_mut().unwrap() ^= 1;
            fs::write(&path, &wav).unwrap();
            assert!(history.finalize(&id, 200, false).is_err());
            assert_eq!(fs::read(&path).unwrap(), wav);
            let mut expected_pcm = rate.to_le_bytes().to_vec();
            expected_pcm.extend_from_slice(&pcm);
            assert_eq!(
                fs::read(history.audio_path(&id, source, "pcm").unwrap()).unwrap(),
                expected_pcm
            );
            assert!(history.path(&id, "jsonl").unwrap().is_file());
            assert!(!history.path(&id, "json").unwrap().exists());
            *wav.last_mut().unwrap() ^= 1;
            fs::write(&path, &wav).unwrap();
            assert!(history.finalize(&id, 200, false).unwrap());
            assert_eq!(history.audio_from(&id, source).unwrap(), wav);
        }
    }

    #[test]
    fn finalization_retries_dual_input_after_one_wav_output_is_blocked() {
        let directory = tempfile::tempdir().unwrap();
        let history = SessionHistory::new(directory.path().join("history"), false);
        for existing_system_wav in [false, true] {
            let id = pending_audio(&history, AudioSource::System, 16_000);
            history
                .append_pcm_from(&id, AudioSource::Microphone, 24_000, &[1, 0, 2, 0])
                .unwrap();
            let system = history.audio_path(&id, AudioSource::System, "wav").unwrap();
            let microphone = history
                .audio_path(&id, AudioSource::Microphone, "wav")
                .unwrap();
            if existing_system_wav {
                fs::write(&system, synthetic_wav(16_000)).unwrap();
            }
            fs::create_dir(&microphone).unwrap();
            assert!(history.finalize(&id, 200, false).is_err());
            assert_pending_audio(&history, &id, AudioSource::System, 16_000);
            assert_pending_audio(&history, &id, AudioSource::Microphone, 24_000);
            assert_eq!(system.is_file(), existing_system_wav);
            fs::remove_dir(&microphone).unwrap();
            assert!(history.finalize(&id, 200, false).unwrap());
            assert_eq!(
                history.audio_from(&id, AudioSource::System).unwrap(),
                synthetic_wav(16_000)
            );
            assert_eq!(
                history.audio_from(&id, AudioSource::Microphone).unwrap(),
                synthetic_wav(24_000)
            );
            assert!(history.audio_selected(&id, None).is_err());
        }
    }

    #[test]
    fn finalization_never_replaces_an_existing_saved_record() {
        let directory = tempfile::tempdir().unwrap();
        let history = SessionHistory::new(directory.path().join("history"), false);
        let id = pending_audio(&history, AudioSource::System, 16_000);
        history.finalize(&id, 200, false).unwrap();
        let saved = fs::read(history.path(&id, "json").unwrap()).unwrap();
        let wav = history.audio(&id).unwrap();
        history.begin(&id, 300).unwrap();
        history
            .append_pcm_from(&id, AudioSource::System, 16_000, &[1, 0, 2, 0])
            .unwrap();
        assert!(history.finalize(&id, 400, false).is_err());
        assert_eq!(fs::read(history.path(&id, "json").unwrap()).unwrap(), saved);
        assert_eq!(history.audio(&id).unwrap(), wav);
        assert!(history.path(&id, "jsonl").unwrap().is_file());
        assert!(history
            .audio_path(&id, AudioSource::System, "pcm")
            .unwrap()
            .is_file());
    }
}
