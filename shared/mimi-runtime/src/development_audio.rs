//! Explicitly opted-in, private development evidence at the WebSocket egress.
//! This is separate from production diagnostics: no credentials, URLs or JSON
//! messages are persisted. Only decoded PCM and allowlisted send metadata exist
//! here. A successful sink send is transport evidence, not provider receipt.

use crate::core::audio_input::AudioSource;
use base64::Engine;
use serde::Serialize;
use std::fs::{self, File, OpenOptions};
use std::future::Future;
#[cfg(test)]
use std::io::Read;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const DISK_LIMIT: u64 = 20 * 1_024 * 1_024;
const QUEUE_CAPACITY: usize = 64;
const MAX_FRAME_BYTES: usize = 256 * 1_024;
const WAV_HEADER_BYTES: u64 = 44;

tokio::task_local! {
    static AUDIO_CONTEXT: AudioContext;
}

#[derive(Clone, Copy)]
pub struct AudioContext {
    source: AudioSource,
    generation: u64,
}

/// Wrap connect, send_audio, finish and lifecycle-spawned audio work in the
/// lane's generation. This prevents unlabelled or mixed-source evidence.
pub async fn scope<F: Future>(source: AudioSource, generation: u64, future: F) -> F::Output {
    AUDIO_CONTEXT
        .scope(AudioContext { source, generation }, future)
        .await
}

pub fn context() -> Option<AudioContext> {
    AUDIO_CONTEXT.try_with(|context| *context).ok()
}

/// Tokio tasks do not inherit task-local values automatically. Provider idle
/// loops capture this value during connect and explicitly restore it here.
pub async fn scope_context<F: Future>(context: Option<AudioContext>, future: F) -> F::Output {
    match context {
        Some(context) => AUDIO_CONTEXT.scope(context, future).await,
        None => future.await,
    }
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioEvidenceStatus {
    pub enabled: bool,
    pub evidence_id: Option<String>,
    pub disk_limit_bytes: u64,
    pub disk_bytes: u64,
    pub limited: bool,
    pub failed_storage: bool,
    pub error: Option<&'static str>,
    pub sources: Vec<AudioSourceStatus>,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioSourceStatus {
    pub source: AudioSource,
    #[serde(rename = "sampleRateHz")]
    pub sample_rate: u32,
    #[serde(rename = "bytes")]
    pub pcm_bytes: u64,
    pub attempted_frames: u64,
    #[serde(rename = "successfulChunks")]
    pub sent_frames: u64,
    #[serde(rename = "failedChunks")]
    pub failed_frames: u64,
    #[serde(rename = "cancelledChunks")]
    pub cancelled_frames: u64,
    #[serde(rename = "droppedChunks")]
    pub dropped_frames: u64,
}

#[derive(Default)]
struct SourceCounters {
    sample_rate: AtomicU64,
    pcm_bytes: AtomicU64,
    attempted: AtomicU64,
    sent: AtomicU64,
    failed: AtomicU64,
    cancelled: AtomicU64,
    dropped: AtomicU64,
}

struct Shared {
    enabled: AtomicBool,
    id: String,
    started: Instant,
    next_sequence: AtomicU64,
    disk_bytes: AtomicU64,
    limited: AtomicBool,
    failed_storage: AtomicBool,
    in_flight: AtomicU64,
    error: Mutex<Option<&'static str>>,
    sources: [SourceCounters; 2],
}

impl Shared {
    fn source(&self, source: AudioSource) -> &SourceCounters {
        &self.sources[source_index(source)]
    }

    fn status(&self) -> AudioEvidenceStatus {
        AudioEvidenceStatus {
            enabled: self.enabled.load(Ordering::SeqCst),
            evidence_id: Some(self.id.clone()),
            disk_limit_bytes: DISK_LIMIT,
            disk_bytes: self.disk_bytes.load(Ordering::Relaxed),
            limited: self.limited.load(Ordering::Relaxed),
            failed_storage: self.failed_storage.load(Ordering::Relaxed),
            error: *self.error.lock().unwrap(),
            sources: [AudioSource::System, AudioSource::Microphone]
                .into_iter()
                .map(|source| {
                    let counters = self.source(source);
                    AudioSourceStatus {
                        source,
                        sample_rate: counters.sample_rate.load(Ordering::Relaxed) as u32,
                        pcm_bytes: counters.pcm_bytes.load(Ordering::Relaxed),
                        attempted_frames: counters.attempted.load(Ordering::Relaxed),
                        sent_frames: counters.sent.load(Ordering::Relaxed),
                        failed_frames: counters.failed.load(Ordering::Relaxed),
                        cancelled_frames: counters.cancelled.load(Ordering::Relaxed),
                        dropped_frames: counters.dropped.load(Ordering::Relaxed),
                    }
                })
                .collect(),
        }
    }

    fn fail(&self, label: &'static str) {
        *self.error.lock().unwrap() = Some(label);
    }
}

struct Recorder {
    shared: Arc<Shared>,
    tx: mpsc::SyncSender<Command>,
    thread: Option<JoinHandle<()>>,
}

static RECORDER: OnceLock<Mutex<Option<Recorder>>> = OnceLock::new();

fn registry() -> &'static Mutex<Option<Recorder>> {
    RECORDER.get_or_init(|| Mutex::new(None))
}

/// Caller owns development-identifier / feature gating. Merely loading this
/// module does nothing; recording is off until this explicit call enables it.
/// Each enabled capture has its own bounded, private evidence directory.
pub fn configure(root: PathBuf, enabled: bool) -> Result<(), &'static str> {
    configure_at(root, enabled, Instant::now())
}
pub fn configure_at(root: PathBuf, enabled: bool, epoch: Instant) -> Result<(), &'static str> {
    if !enabled {
        let current = registry()
            .lock()
            .unwrap()
            .as_ref()
            .map(|recorder| (recorder.shared.clone(), recorder.tx.clone()));
        if let Some((shared, writer)) = current {
            shared.enabled.store(false, Ordering::SeqCst);
            let deadline = Instant::now() + Duration::from_secs(5);
            while shared.in_flight.load(Ordering::SeqCst) != 0 {
                if Instant::now() >= deadline {
                    shared.fail("audio_send_still_pending");
                    return Err("audio_send_still_pending");
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            let (tx, rx) = mpsc::channel();
            writer
                .send(Command::Barrier(tx))
                .map_err(|_| "audio_writer_unavailable")?;
            rx.recv_timeout(Duration::from_secs(2))
                .map_err(|_| "audio_writer_timeout")?;
        }
        return Ok(());
    }
    configure(PathBuf::new(), false)?;
    let mut previous = registry().lock().unwrap().take();
    if let Some(previous) = previous.as_mut() {
        let _ = previous.tx.send(Command::Shutdown);
        if let Some(thread) = previous.thread.take() {
            let _ = thread.join();
        }
    }
    let recorder = Recorder::create_at(&root, epoch)?;
    *registry().lock().unwrap() = Some(recorder);
    Ok(())
}

pub fn status() -> AudioEvidenceStatus {
    registry()
        .lock()
        .unwrap()
        .as_ref()
        .map(|recorder| recorder.shared.status())
        .unwrap_or_else(|| AudioEvidenceStatus {
            disk_limit_bytes: DISK_LIMIT,
            ..Default::default()
        })
}

impl Recorder {
    #[cfg(test)]
    fn create(root: &Path) -> Result<Self, &'static str> {
        Self::create_at(root, Instant::now())
    }
    fn create_at(root: &Path, epoch: Instant) -> Result<Self, &'static str> {
        private_directory(root).map_err(|_| "audio_directory_failed")?;
        let id = uuid::Uuid::new_v4().to_string();
        let directory = root.join("audio-evidence");
        private_directory(&directory).map_err(|_| "audio_directory_failed")?;
        let manifest =
            private_file(&directory.join("audio-index.jsonl")).map_err(|_| "audio_file_failed")?;
        let shared = Arc::new(Shared {
            enabled: AtomicBool::new(true),
            id,
            started: epoch,
            next_sequence: AtomicU64::new(1),
            disk_bytes: AtomicU64::new(0),
            limited: AtomicBool::new(false),
            failed_storage: AtomicBool::new(false),
            in_flight: AtomicU64::new(0),
            error: Mutex::new(None),
            sources: Default::default(),
        });
        let (tx, rx) = mpsc::sync_channel(QUEUE_CAPACITY);
        let writer = Writer {
            directory,
            manifest,
            waves: [None, None],
            shared: shared.clone(),
        };
        let thread = std::thread::Builder::new()
            .name("mimi-dev-audio-evidence".into())
            .spawn(move || writer.run(rx))
            .map_err(|_| "audio_writer_failed")?;
        Ok(Self {
            shared,
            tx,
            thread: Some(thread),
        })
    }
}

fn source_index(source: AudioSource) -> usize {
    match source {
        AudioSource::System => 0,
        AudioSource::Microphone => 1,
    }
}

fn source_filename(source: AudioSource) -> &'static str {
    match source {
        AudioSource::System => "system.wav",
        AudioSource::Microphone => "microphone.wav",
    }
}

fn private_directory(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn private_file(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// The ticket is created immediately before the socket send. Dropping its
/// future, including outer timeouts, records cancellation instead of success.
pub struct SendTicket(Option<Ticket>);

struct Ticket {
    shared: Arc<Shared>,
    tx: mpsc::SyncSender<Command>,
    context: AudioContext,
    sequence: u64,
    sample_rate: u32,
    started_ms: u64,
    pcm: Vec<u8>,
}

impl SendTicket {
    pub async fn observe<F, E>(self, future: F) -> Result<(), E>
    where
        F: Future<Output = Result<(), E>>,
    {
        let result = future.await;
        self.complete(result.is_ok());
        result
    }

    pub fn complete(mut self, success: bool) {
        if let Some(ticket) = self.0.take() {
            ticket.finish(if success { "sent" } else { "failed" });
        }
    }
}

impl Drop for SendTicket {
    fn drop(&mut self) {
        if let Some(ticket) = self.0.take() {
            ticket.finish("cancelled");
        }
    }
}

impl Ticket {
    fn finish(self, outcome: &'static str) {
        let counters = self.shared.source(self.context.source);
        match outcome {
            "sent" => &counters.sent,
            "failed" => &counters.failed,
            _ => &counters.cancelled,
        }
        .fetch_add(1, Ordering::Relaxed);
        let event = EgressEvent {
            sequence: self.sequence,
            source: self.context.source,
            generation: self.context.generation,
            sample_rate: self.sample_rate,
            pcm_bytes: self.pcm.len() as u64,
            started_ms: self.started_ms,
            finished_ms: self.shared.started.elapsed().as_millis() as u64,
            outcome,
            wav_offset: None,
        };
        let pcm = if outcome == "sent" {
            self.pcm
        } else {
            Vec::new()
        };
        if self.tx.try_send(Command::Write(event, pcm)).is_err() {
            counters.dropped.fetch_add(1, Ordering::Relaxed);
        }
        self.shared.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

fn active() -> Option<(Arc<Shared>, mpsc::SyncSender<Command>, AudioContext)> {
    let context = context()?;
    let registry = registry().lock().unwrap();
    let recorder = registry.as_ref()?;
    recorder
        .shared
        .enabled
        .load(Ordering::Relaxed)
        .then(|| (recorder.shared.clone(), recorder.tx.clone(), context))
}

pub fn begin_pcm(pcm: &[u8], sample_rate: u32) -> SendTicket {
    let Some(active) = active() else {
        return SendTicket(None);
    };
    make_ticket(active, pcm, sample_rate)
}

fn make_ticket(
    active: (Arc<Shared>, mpsc::SyncSender<Command>, AudioContext),
    pcm: &[u8],
    sample_rate: u32,
) -> SendTicket {
    let (shared, tx, context) = active;
    if pcm.is_empty() || !shared.enabled.load(Ordering::SeqCst) {
        return SendTicket(None);
    }
    if shared.limited.load(Ordering::Relaxed) || shared.failed_storage.load(Ordering::Relaxed) {
        shared
            .source(context.source)
            .dropped
            .fetch_add(1, Ordering::Relaxed);
        return SendTicket(None);
    }
    if pcm.len() > MAX_FRAME_BYTES
        || !pcm.len().is_multiple_of(2)
        || !matches!(sample_rate, 16_000 | 24_000)
    {
        shared
            .source(context.source)
            .dropped
            .fetch_add(1, Ordering::Relaxed);
        shared.fail("audio_frame_invalid");
        return SendTicket(None);
    }
    shared.in_flight.fetch_add(1, Ordering::SeqCst);
    if !shared.enabled.load(Ordering::SeqCst) {
        shared.in_flight.fetch_sub(1, Ordering::SeqCst);
        return SendTicket(None);
    }
    let sequence = shared.next_sequence.fetch_add(1, Ordering::Relaxed);
    let started_ms = shared.started.elapsed().as_millis() as u64;
    shared
        .source(context.source)
        .attempted
        .fetch_add(1, Ordering::Relaxed);
    SendTicket(Some(Ticket {
        shared,
        tx,
        context,
        sequence,
        sample_rate,
        started_ms,
        pcm: pcm.to_vec(),
    }))
}

/// Decode the audio field from the exact outgoing JSON. Never persist or
/// inspect setup messages, headers, provider prompts or credentials.
pub fn begin_json(text: &str, sample_rate: u32) -> SendTicket {
    let Some(active) = active() else {
        return SendTicket(None);
    };
    let Some(encoded) = json_audio(text) else {
        return SendTicket(None);
    };
    if encoded.len() > MAX_FRAME_BYTES * 2 {
        active
            .0
            .source(active.2.source)
            .dropped
            .fetch_add(1, Ordering::Relaxed);
        active.0.fail("audio_frame_invalid");
        return SendTicket(None);
    }
    match base64::engine::general_purpose::STANDARD.decode(encoded) {
        Ok(pcm) => make_ticket(active, &pcm, sample_rate),
        Err(_) => {
            active
                .0
                .source(active.2.source)
                .dropped
                .fetch_add(1, Ordering::Relaxed);
            active.0.fail("audio_frame_invalid");
            SendTicket(None)
        }
    }
}

fn json_audio(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let encoded = if matches!(
        value.get("type").and_then(serde_json::Value::as_str),
        Some("input_audio_buffer.append" | "session.input_audio_buffer.append")
    ) {
        value.get("audio")?.as_str()?
    } else {
        value.pointer("/realtimeInput/audio/data")?.as_str()?
    };
    Some(encoded.to_owned())
}

/// Volcano AST v2 transports raw protobuf (no gzip envelope). Only TaskRequest
/// event 200 / SourceAudio field 14 is PCM; setup and finish are excluded.
pub fn begin_volcano(frame: &[u8], sample_rate: u32) -> SendTicket {
    let Some(active) = active() else {
        return SendTicket(None);
    };
    match volcano_audio(frame) {
        Some(pcm) => make_ticket(active, pcm, sample_rate),
        None => SendTicket(None),
    }
}

fn volcano_audio(frame: &[u8]) -> Option<&[u8]> {
    let mut remaining = frame;
    let mut event = None;
    let mut source_audio = None;
    while !remaining.is_empty() {
        let tag = read_varint(&mut remaining)?;
        match (tag >> 3, tag & 7) {
            (2, 0) => event = Some(read_varint(&mut remaining)?),
            (4, 2) => source_audio = Some(read_bytes(&mut remaining)?),
            (_, 0) => {
                read_varint(&mut remaining)?;
            }
            (_, 2) => {
                read_bytes(&mut remaining)?;
            }
            (_, 1) => remaining = remaining.get(8..)?,
            (_, 5) => remaining = remaining.get(4..)?,
            _ => return None,
        }
    }
    if event != Some(200) {
        return None;
    }
    let mut remaining = source_audio?;
    while !remaining.is_empty() {
        let tag = read_varint(&mut remaining)?;
        match (tag >> 3, tag & 7) {
            (14, 2) => return read_bytes(&mut remaining),
            (_, 0) => {
                read_varint(&mut remaining)?;
            }
            (_, 2) => {
                read_bytes(&mut remaining)?;
            }
            (_, 1) => remaining = remaining.get(8..)?,
            (_, 5) => remaining = remaining.get(4..)?,
            _ => return None,
        }
    }
    None
}

fn read_varint(input: &mut &[u8]) -> Option<u64> {
    let mut value = 0;
    for shift in (0..70).step_by(7) {
        let (&byte, rest) = input.split_first()?;
        *input = rest;
        if shift == 63 && byte > 1 {
            return None;
        }
        value |= u64::from(byte & 127) << shift;
        if byte & 128 == 0 {
            return Some(value);
        }
    }
    None
}

fn read_bytes<'a>(input: &mut &'a [u8]) -> Option<&'a [u8]> {
    let length = usize::try_from(read_varint(input)?).ok()?;
    let (value, rest) = input.split_at_checked(length)?;
    *input = rest;
    Some(value)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EgressEvent {
    sequence: u64,
    source: AudioSource,
    generation: u64,
    sample_rate: u32,
    pcm_bytes: u64,
    started_ms: u64,
    finished_ms: u64,
    outcome: &'static str,
    wav_offset: Option<u64>,
}

enum Command {
    Write(EgressEvent, Vec<u8>),
    #[cfg(test)]
    ReadAudio(AudioSource, mpsc::Sender<Result<Vec<u8>, &'static str>>),
    Barrier(mpsc::Sender<()>),
    Shutdown,
}

struct Wave {
    file: File,
    pcm_bytes: u64,
    sample_rate: u32,
}

struct Writer {
    directory: PathBuf,
    manifest: File,
    waves: [Option<Wave>; 2],
    shared: Arc<Shared>,
}

impl Writer {
    fn run(mut self, rx: mpsc::Receiver<Command>) {
        while let Ok(command) = rx.recv() {
            match command {
                Command::Write(event, pcm) => {
                    let source = event.source;
                    if self.write(event, &pcm).is_err() {
                        self.shared
                            .source(source)
                            .dropped
                            .fetch_add(1, Ordering::Relaxed);
                        self.shared.fail("audio_write_failed");
                        self.shared.failed_storage.store(true, Ordering::Relaxed);
                    }
                }
                #[cfg(test)]
                Command::ReadAudio(source, reply) => {
                    let _ = reply.send(self.read(source));
                }
                Command::Barrier(reply) => {
                    let _ = reply.send(());
                }
                Command::Shutdown => break,
            }
        }
    }

    fn write(&mut self, mut event: EgressEvent, pcm: &[u8]) -> std::io::Result<()> {
        if self.shared.failed_storage.load(Ordering::Relaxed) {
            self.shared
                .source(event.source)
                .dropped
                .fetch_add(1, Ordering::Relaxed);
            return Ok(());
        }
        let index = source_index(event.source);
        let new_wave = !pcm.is_empty() && self.waves[index].is_none();
        if let Some(wave) = &self.waves[index] {
            if !pcm.is_empty() && wave.sample_rate != event.sample_rate {
                self.shared
                    .source(event.source)
                    .dropped
                    .fetch_add(1, Ordering::Relaxed);
                self.shared.fail("audio_sample_rate_changed");
                return Ok(());
            }
        }
        if !pcm.is_empty() {
            event.wav_offset = Some(self.waves[index].as_ref().map_or(0, |wave| wave.pcm_bytes));
        }
        let mut metadata = serde_json::to_vec(&event)?;
        metadata.push(b'\n');
        let needed =
            metadata.len() as u64 + pcm.len() as u64 + if new_wave { WAV_HEADER_BYTES } else { 0 };
        let total = self.shared.disk_bytes.load(Ordering::Relaxed);
        if total.saturating_add(needed) > DISK_LIMIT {
            self.shared.limited.store(true, Ordering::Relaxed);
            self.shared
                .source(event.source)
                .dropped
                .fetch_add(1, Ordering::Relaxed);
            return Ok(());
        }
        if new_wave {
            let mut file = private_file(&self.directory.join(source_filename(event.source)))?;
            file.write_all(&wav_header(event.sample_rate, 0))?;
            self.shared
                .disk_bytes
                .fetch_add(WAV_HEADER_BYTES, Ordering::Relaxed);
            self.waves[index] = Some(Wave {
                file,
                pcm_bytes: 0,
                sample_rate: event.sample_rate,
            });
        }
        if !pcm.is_empty() {
            let wave = self.waves[index].as_mut().unwrap();
            wave.file.seek(SeekFrom::End(0))?;
            wave.file.write_all(pcm)?;
            wave.pcm_bytes += pcm.len() as u64;
            wave.file.seek(SeekFrom::Start(0))?;
            wave.file
                .write_all(&wav_header(wave.sample_rate, wave.pcm_bytes as u32))?;
            wave.file.flush()?;
            let counters = self.shared.source(event.source);
            counters
                .sample_rate
                .store(event.sample_rate as u64, Ordering::Relaxed);
            counters.pcm_bytes.store(wave.pcm_bytes, Ordering::Relaxed);
            self.shared
                .disk_bytes
                .fetch_add(pcm.len() as u64, Ordering::Relaxed);
        }
        self.manifest.write_all(&metadata)?;
        self.manifest.flush()?;
        self.shared
            .disk_bytes
            .fetch_add(metadata.len() as u64, Ordering::Relaxed);
        Ok(())
    }

    #[cfg(test)]
    fn read(&mut self, source: AudioSource) -> Result<Vec<u8>, &'static str> {
        let wave = self.waves[source_index(source)]
            .as_mut()
            .ok_or("audio_evidence_empty")?;
        wave.file
            .seek(SeekFrom::Start(0))
            .map_err(|_| "audio_read_failed")?;
        let mut bytes = Vec::with_capacity(wave.pcm_bytes as usize + WAV_HEADER_BYTES as usize);
        (&mut wave.file)
            .take(DISK_LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "audio_read_failed")?;
        if bytes.len() as u64 > DISK_LIMIT {
            return Err("audio_evidence_limit");
        }
        Ok(bytes)
    }
}

fn wav_header(sample_rate: u32, pcm_bytes: u32) -> [u8; 44] {
    let mut wav = [0; 44];
    wav[0..4].copy_from_slice(b"RIFF");
    wav[4..8].copy_from_slice(&(pcm_bytes + 36).to_le_bytes());
    wav[8..16].copy_from_slice(b"WAVEfmt ");
    wav[16..20].copy_from_slice(&16_u32.to_le_bytes());
    wav[20..22].copy_from_slice(&1_u16.to_le_bytes());
    wav[22..24].copy_from_slice(&1_u16.to_le_bytes());
    wav[24..28].copy_from_slice(&sample_rate.to_le_bytes());
    wav[28..32].copy_from_slice(&(sample_rate * 2).to_le_bytes());
    wav[32..34].copy_from_slice(&2_u16.to_le_bytes());
    wav[34..36].copy_from_slice(&16_u16.to_le_bytes());
    wav[36..40].copy_from_slice(b"data");
    wav[40..44].copy_from_slice(&pcm_bytes.to_le_bytes());
    wav
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::{SourceLanguage, TargetLanguage};
    use crate::core::protocols::{
        gemini_live::{GeminiLiveEndpoint, GeminiLiveRequestEncoder},
        openai_realtime::{OpenAIRealtimeEndpoint, OpenAIRealtimeRequestEncoder},
        volcano_engine::{VolcanoEngineEndpoint, VolcanoEngineRequestEncoder},
    };

    #[test]
    fn wire_audio_decodes_exact_provider_frames_without_protocol_bytes() {
        let pcm = vec![0x31; OpenAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT];
        let wire = OpenAIRealtimeRequestEncoder::audio_append(&pcm)
            .unwrap()
            .to_string();
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(json_audio(&wire).unwrap())
                .unwrap(),
            pcm
        );
        let pcm = vec![0x32; GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT];
        let wire = GeminiLiveRequestEncoder::audio(&pcm).unwrap().to_string();
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(json_audio(&wire).unwrap())
                .unwrap(),
            pcm
        );
        let pcm = vec![0x33; VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT];
        let wire = VolcanoEngineRequestEncoder::audio("synthetic", &pcm).unwrap();
        assert_eq!(volcano_audio(&wire), Some(pcm.as_slice()));
        let setup = VolcanoEngineRequestEncoder::start_session(
            "synthetic",
            SourceLanguage::English,
            TargetLanguage::SimplifiedChinese,
        )
        .unwrap();
        assert!(volcano_audio(&setup).is_none());
        assert!(
            volcano_audio(&VolcanoEngineRequestEncoder::finish_session("synthetic").unwrap())
                .is_none()
        );
        assert!(json_audio(r#"{"type":"session.update","audio":"secret"}"#).is_none());
    }

    #[tokio::test]
    async fn sent_pcm_is_exact_and_failed_cancelled_or_disabled_frames_never_enter_wav() {
        // Use a local recorder so concurrently running provider tests cannot
        // accidentally opt into this private capture.
        let root = tempfile::tempdir().unwrap();
        let mut recorder = Recorder::create(root.path()).unwrap();
        let context = AudioContext {
            source: AudioSource::System,
            generation: 42,
        };
        let ticket = |pcm: &[u8]| {
            make_ticket(
                (recorder.shared.clone(), recorder.tx.clone(), context),
                pcm,
                16_000,
            )
        };
        ticket(&[1, 2, 3, 4]).complete(true);
        assert!(ticket(&[5, 6])
            .observe(async { Err::<(), ()>(()) })
            .await
            .is_err());
        assert!(tokio::time::timeout(
            Duration::from_millis(1),
            ticket(&[7, 8]).observe(std::future::pending::<Result<(), ()>>())
        )
        .await
        .is_err());
        ticket(&[9, 10]).complete(true);
        recorder.shared.enabled.store(false, Ordering::SeqCst);
        ticket(&[11, 12]).complete(true);
        assert_eq!(recorder.shared.in_flight.load(Ordering::SeqCst), 0);
        let (tx, rx) = mpsc::channel();
        recorder
            .tx
            .send(Command::ReadAudio(AudioSource::System, tx))
            .unwrap();
        let wav = rx.recv().unwrap().unwrap();
        assert_eq!(&wav[44..], &[1, 2, 3, 4, 9, 10]);
        assert_eq!(&wav[24..28], &16_000_u32.to_le_bytes());
        let status = recorder.shared.status();
        assert_eq!(status.sources[0].pcm_bytes, 6);
        assert_eq!(status.sources[0].sent_frames, 2);
        assert_eq!(status.sources[0].failed_frames, 1);
        assert_eq!(status.sources[0].cancelled_frames, 1);
        let path = root.path().join("audio-evidence");
        let manifest = fs::read_to_string(path.join("audio-index.jsonl")).unwrap();
        assert!(manifest.contains("\"outcome\":\"failed\""));
        assert!(manifest.contains("\"outcome\":\"cancelled\""));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(path.join("system.wav"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        recorder.tx.send(Command::Shutdown).unwrap();
        recorder.thread.take().unwrap().join().unwrap();
    }

    #[test]
    fn actual_json_and_protobuf_frames_produce_byte_identical_separate_waves() {
        let root = tempfile::tempdir().unwrap();
        let mut recorder = Recorder::create(root.path()).unwrap();
        let openai_pcm: Vec<u8> = (0..OpenAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT)
            .map(|i| i as u8)
            .collect();
        let openai_wire = OpenAIRealtimeRequestEncoder::audio_append(&openai_pcm)
            .unwrap()
            .to_string();
        let gemini_pcm = vec![0x41; GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT];
        let gemini_wire = GeminiLiveRequestEncoder::audio(&gemini_pcm)
            .unwrap()
            .to_string();
        let volcano_pcm = vec![0x52; VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT];
        let volcano_wire = VolcanoEngineRequestEncoder::audio("synthetic", &volcano_pcm).unwrap();
        for (source, pcm, sample_rate) in [
            (
                AudioSource::System,
                base64::engine::general_purpose::STANDARD
                    .decode(json_audio(&openai_wire).unwrap())
                    .unwrap(),
                24_000,
            ),
            (
                AudioSource::Microphone,
                base64::engine::general_purpose::STANDARD
                    .decode(json_audio(&gemini_wire).unwrap())
                    .unwrap(),
                16_000,
            ),
            (
                AudioSource::Microphone,
                volcano_audio(&volcano_wire).unwrap().to_vec(),
                16_000,
            ),
        ] {
            make_ticket(
                (
                    recorder.shared.clone(),
                    recorder.tx.clone(),
                    AudioContext {
                        source,
                        generation: 7,
                    },
                ),
                &pcm,
                sample_rate,
            )
            .complete(true);
        }
        for (source, expected, rate) in [
            (AudioSource::System, openai_pcm, 24_000_u32),
            (
                AudioSource::Microphone,
                [gemini_pcm, volcano_pcm].concat(),
                16_000_u32,
            ),
        ] {
            let (tx, rx) = mpsc::channel();
            recorder.tx.send(Command::ReadAudio(source, tx)).unwrap();
            let wave = rx.recv().unwrap().unwrap();
            assert_eq!(&wave[24..28], &rate.to_le_bytes());
            assert_eq!(&wave[40..44], &(expected.len() as u32).to_le_bytes());
            assert_eq!(&wave[44..], expected);
        }
        let index =
            fs::read_to_string(root.path().join("audio-evidence/audio-index.jsonl")).unwrap();
        let records: Vec<serde_json::Value> = index
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.len(), 3);
        assert_eq!(
            records[2]["wavOffset"],
            GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT as u64
        );
        assert_eq!(records[2]["generation"], 7);
        recorder.tx.send(Command::Shutdown).unwrap();
        recorder.thread.take().unwrap().join().unwrap();
    }

    #[test]
    fn full_queue_never_blocks_send_and_exposes_missing_evidence() {
        let root = tempfile::tempdir().unwrap();
        let mut recorder = Recorder::create(root.path()).unwrap();
        let (tx, _unconsumed) = mpsc::sync_channel(1);
        let (barrier, _reply) = mpsc::channel();
        tx.send(Command::Barrier(barrier)).unwrap();
        make_ticket(
            (
                recorder.shared.clone(),
                tx,
                AudioContext {
                    source: AudioSource::System,
                    generation: 1,
                },
            ),
            &[1, 2],
            16_000,
        )
        .complete(true);
        let status = recorder.shared.status();
        assert_eq!(status.sources[0].sent_frames, 1);
        assert_eq!(status.sources[0].dropped_frames, 1);
        assert_eq!(status.sources[0].pcm_bytes, 0);
        assert_eq!(recorder.shared.in_flight.load(Ordering::SeqCst), 0);
        recorder.tx.send(Command::Shutdown).unwrap();
        recorder.thread.take().unwrap().join().unwrap();
    }

    #[test]
    fn disk_limit_is_explicit() {
        let root = tempfile::tempdir().unwrap();
        let mut recorder = Recorder::create(root.path()).unwrap();
        recorder
            .shared
            .disk_bytes
            .store(DISK_LIMIT - 1, Ordering::Relaxed);
        make_ticket(
            (
                recorder.shared.clone(),
                recorder.tx.clone(),
                AudioContext {
                    source: AudioSource::Microphone,
                    generation: 1,
                },
            ),
            &[1, 2],
            24_000,
        )
        .complete(true);
        let (tx, rx) = mpsc::channel();
        recorder.tx.send(Command::Barrier(tx)).unwrap();
        rx.recv().unwrap();
        let status = recorder.shared.status();
        assert!(status.limited);
        assert_eq!(status.sources[1].dropped_frames, 1);
        assert_eq!(status.sources[1].pcm_bytes, 0);
        assert!(status.disk_bytes <= DISK_LIMIT);
        recorder.tx.send(Command::Shutdown).unwrap();
        recorder.thread.take().unwrap().join().unwrap();
    }
}
