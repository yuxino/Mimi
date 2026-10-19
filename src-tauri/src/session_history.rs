//! Local opt-in transcript history. Files contain user content and are never logged.
use crate::core::audio_input::AudioSource;
use crate::core::models::SubtitlePair;
use crate::core::session_archive::{SavedTranscript, TranscriptPage, AUDIO_BYTE_LIMIT};
use serde::{de, Deserialize, Serialize};
use std::fs;
use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const MAX_TRANSCRIPT_FILE_BYTES: u64 = 16 * 1024 * 1024;

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
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
    pub count: usize,
    pub limited: bool,
    pub has_audio: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryMetadata {
    version: u32,
    id: String,
    started_at_ms: u64,
    ended_at_ms: u64,
    limited: bool,
    #[serde(deserialize_with = "count_history_entries")]
    entries: usize,
}

// Validate the same persisted fields as SubtitlePair without retaining or
// allocating its source/translation strings while reading a list summary.
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

fn count_history_entries<'de, D: de::Deserializer<'de>>(
    deserializer: D,
) -> Result<usize, D::Error> {
    struct EntriesVisitor;
    impl<'de> de::Visitor<'de> for EntriesVisitor {
        type Value = usize;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("an array of subtitle pairs")
        }

        fn visit_seq<A: de::SeqAccess<'de>>(self, mut entries: A) -> Result<Self::Value, A::Error> {
            let mut count = 0;
            while entries.next_element::<HistoryPairMetadata>()?.is_some() {
                count += 1;
            }
            Ok(count)
        }
    }
    deserializer.deserialize_seq(EntriesVisitor)
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

    fn read(&self, id: &str) -> io::Result<SavedTranscript> {
        let _io = self.io.lock().unwrap();
        let file = self.open_transcript(id)?;
        let mut lines = io::BufReader::new(file).lines();
        let first = lines
            .next()
            .transpose()?
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "empty history"))?;
        let mut saved: SavedTranscript = serde_json::from_str(&first)?;
        for line in lines {
            let Ok(line) = line else { break };
            if line.is_empty() {
                continue;
            }
            let Ok(pair) = serde_json::from_str::<SubtitlePair>(&line) else {
                continue;
            };
            if saved.entries.len() >= crate::core::session_archive::TRANSCRIPT_COUNT_LIMIT {
                break;
            }
            saved.ended_at_ms = saved.ended_at_ms.max(pair.created_at_ms);
            saved.entries.push(pair);
        }
        if saved.version != 1 || saved.id != id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid history file",
            ));
        }
        Ok(saved)
    }

    fn read_metadata(&self, id: &str) -> io::Result<HistoryMetadata> {
        let _io = self.io.lock().unwrap();
        let mut reader = io::BufReader::new(self.open_transcript(id)?);
        // Reuse one bounded line buffer. Finalized JSON stores the whole
        // transcript on its first line, but its entries never become an owned
        // transcript or a second copy of all subtitle text just to list it.
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "empty history"));
        }
        let mut metadata: HistoryMetadata = serde_json::from_str(&line)?;
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let Ok(pair) = serde_json::from_str::<HistoryPairMetadata>(&line) else {
                continue;
            };
            if metadata.entries >= crate::core::session_archive::TRANSCRIPT_COUNT_LIMIT {
                break;
            }
            metadata.ended_at_ms = metadata.ended_at_ms.max(pair.created_at_ms);
            metadata.entries += 1;
        }
        if metadata.version != 1 || metadata.id != id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid history file",
            ));
        }
        Ok(metadata)
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
        let mut saved = self.read(id)?;
        saved.ended_at_ms = ended_at_ms;
        saved.limited = limited;
        let audio_sources = self.audio_sources(id)?;
        if saved.entries.is_empty() && audio_sources.is_empty() {
            self.discard(id)?;
            return Ok(false);
        }

        let _io = self.io.lock().unwrap();
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
            let mut temporary = tempfile::NamedTempFile::new_in(&self.directory)?;
            write_final_transcript(&mut temporary, &saved)?;
            temporary.as_file().sync_all()?;
            temporary
                .persist_noclobber(self.path(id, "json")?)
                .map_err(|error| error.error)?;
            Ok(())
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
                id,
                started_at_ms: saved.started_at_ms,
                ended_at_ms: saved.ended_at_ms,
                limited: saved.limited,
            });
        }
        items.sort_by_key(|item| std::cmp::Reverse(item.started_at_ms));
        Ok(items)
    }

    pub fn page(&self, id: &str, query: &str, page: usize) -> io::Result<TranscriptPage> {
        Ok(self.read(id)?.page(query, page))
    }

    pub fn export_transcript(&self, id: &str) -> io::Result<Vec<u8>> {
        self.read(id)?
            .export()
            .map(String::into_bytes)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no transcript"))
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
        self.read(id)?;
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
        self.remove_audio_files(id)?;
        let path = self.path(id, "jsonl")?;
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }
}

fn write_final_transcript(writer: &mut impl Write, saved: &SavedTranscript) -> io::Result<()> {
    // Batch the serializer's small tokens without another transcript-sized
    // copy. Flush errors must reach the caller before it syncs or publishes.
    let mut writer = io::BufWriter::with_capacity(8 * 1024, writer);
    serde_json::to_writer(&mut writer, saved).map_err(io::Error::other)?;
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

    #[test]
    fn final_transcript_write_batches_small_tokens_and_preserves_bytes() {
        let saved = synthetic_final_transcript(512);
        let mut writer = TranscriptWriter::default();
        write_final_transcript(&mut writer, &saved).unwrap();
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
            assert!(write_final_transcript(&mut writer, &saved).is_err());
            assert_eq!(writer.bytes, expected[..accepted]);
        }
        let mut writer = TranscriptWriter {
            fail_flush: true,
            ..Default::default()
        };
        assert!(write_final_transcript(&mut writer, &saved).is_err());
        assert_eq!(writer.bytes, expected);
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
