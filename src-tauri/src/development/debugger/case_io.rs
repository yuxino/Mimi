//! EvidenceCase background writer, event recording, and report/audio/trace persistence.

use super::workspace::{
    copy_case_file, open_case_file, private_directory, private_file, stored_bytes,
};
use super::DebuggerSnapshot;
use crate::core::audio::audio_input::AudioSource;
use crate::core::development::development_debug as trace;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Instant;

pub(super) const CONTENT_BYTE_LIMIT: u64 = 16 * 1024 * 1024;
pub(super) const ALL_CASES_BYTE_LIMIT: u64 = 128 * 1024 * 1024;
pub(super) const AUDIO_FILE_BYTE_LIMIT: u64 = 20 * 1024 * 1024;
pub(super) const CASE_RESERVED_BYTES: u64 =
    CONTENT_BYTE_LIMIT + AUDIO_FILE_BYTE_LIMIT + 4 * 1024 * 1024;
pub(super) const CASE_COUNT_LIMIT: usize = 64;

#[derive(Clone, Copy)]
pub(super) enum ContentKind {
    Snapshot,
    Private,
    Trace,
}

pub(super) type ContentSender = mpsc::SyncSender<(ContentKind, Vec<u8>)>;

pub(super) struct EvidenceCase {
    pub(super) id: String,
    pub(super) directory: PathBuf,
    pub(super) epoch: Instant,
    pub(super) tx: Mutex<Option<ContentSender>>,
    pub(super) write_gate: Mutex<()>,
    pub(super) snapshots: AtomicU64,
    pub(super) private_events: AtomicU64,
    pub(super) trace_entries: AtomicU64,
    pub(super) trace_bytes: AtomicU64,
    pub(super) trace_dropped: AtomicU64,
    pub(super) trace_limited: AtomicBool,
    pub(super) trace_failed: AtomicBool,
    pub(super) trace_persistence_enabled: bool,
    pub(super) trace_sink: Mutex<Option<trace::TraceSink>>,
    pub(super) bytes: AtomicU64,
    pub(super) dropped: AtomicU64,
    pub(super) limited: AtomicBool,
    pub(super) failed: AtomicBool,
    pub(super) writer: Mutex<Option<std::thread::JoinHandle<()>>>,
    pub(super) finalized: AtomicBool,
}

pub(super) fn start_case(
    root: PathBuf,
    route: &serde_json::Value,
    epoch: Instant,
) -> Result<Arc<EvidenceCase>, &'static str> {
    private_directory(&root).map_err(|_| "development_evidence_storage_failed")?;
    if stored_bytes(&root).map_err(|_| "development_evidence_storage_failed")?
        > ALL_CASES_BYTE_LIMIT.saturating_sub(CASE_RESERVED_BYTES)
        || fs::read_dir(&root)
            .map_err(|_| "development_evidence_storage_failed")?
            .count()
            >= CASE_COUNT_LIMIT
    {
        return Err("development_evidence_storage_limit");
    }
    let id = uuid::Uuid::new_v4().to_string();
    let directory = root.join(&id);
    private_directory(&directory).map_err(|_| "development_evidence_storage_failed")?;
    let mut manifest = private_file(&directory.join("manifest.json"))
        .map_err(|_| "development_evidence_storage_failed")?;
    serde_json::to_writer_pretty(&mut manifest, &serde_json::json!({"schemaVersion":1,"id":id,"route":route,"contentByteLimit":CONTENT_BYTE_LIMIT,"audioBoundary":"socketSendCompleted","serverReceiptProven":false,"containsPrivateAudioAndSubtitles":true})).map_err(|_| "development_evidence_storage_failed")?;
    let mut file = private_file(&directory.join("snapshots.jsonl"))
        .map_err(|_| "development_evidence_storage_failed")?;
    let mut private_events = private_file(&directory.join("events.jsonl"))
        .map_err(|_| "development_evidence_storage_failed")?;
    let mut trace_events = private_file(&directory.join("trace-events.jsonl"))
        .map_err(|_| "development_evidence_storage_failed")?;
    let (tx, rx) = mpsc::sync_channel::<(ContentKind, Vec<u8>)>(32);
    let case = Arc::new(EvidenceCase {
        id,
        directory,
        epoch,
        tx: Mutex::new(Some(tx)),
        write_gate: Mutex::new(()),
        snapshots: AtomicU64::new(0),
        private_events: AtomicU64::new(0),
        trace_entries: AtomicU64::new(0),
        trace_bytes: AtomicU64::new(0),
        trace_dropped: AtomicU64::new(0),
        trace_limited: AtomicBool::new(false),
        trace_failed: AtomicBool::new(false),
        trace_persistence_enabled: true,
        trace_sink: Mutex::new(None),
        bytes: AtomicU64::new(0),
        dropped: AtomicU64::new(0),
        limited: AtomicBool::new(false),
        failed: AtomicBool::new(false),
        writer: Mutex::new(None),
        finalized: AtomicBool::new(false),
    });
    let writer = Arc::clone(&case);
    let handle = std::thread::spawn(move || {
        while let Ok((kind, bytes)) = rx.recv() {
            if writer
                .bytes
                .load(Ordering::Relaxed)
                .saturating_add(bytes.len() as u64)
                > CONTENT_BYTE_LIMIT
            {
                writer.limited.store(true, Ordering::Relaxed);
                writer.dropped.fetch_add(1, Ordering::Relaxed);
                if matches!(kind, ContentKind::Trace) {
                    writer.trace_limited.store(true, Ordering::Relaxed);
                    writer.trace_dropped.fetch_add(1, Ordering::Relaxed);
                }
                continue;
            }
            let _gate = writer.write_gate.lock().unwrap();
            if (match kind {
                ContentKind::Snapshot => file.write_all(&bytes),
                ContentKind::Private => private_events.write_all(&bytes),
                ContentKind::Trace => trace_events.write_all(&bytes),
            })
            .is_err()
            {
                writer.failed.store(true, Ordering::Relaxed);
                writer.dropped.fetch_add(1, Ordering::Relaxed);
                if matches!(kind, ContentKind::Trace) {
                    writer.trace_failed.store(true, Ordering::Relaxed);
                    writer.trace_dropped.fetch_add(1, Ordering::Relaxed);
                }
                continue;
            }
            writer
                .bytes
                .fetch_add(bytes.len() as u64, Ordering::Relaxed);
            match kind {
                ContentKind::Snapshot => {
                    writer.snapshots.fetch_add(1, Ordering::Relaxed);
                }
                ContentKind::Private => {
                    writer.private_events.fetch_add(1, Ordering::Relaxed);
                }
                ContentKind::Trace => {
                    writer.trace_entries.fetch_add(1, Ordering::Relaxed);
                    writer
                        .trace_bytes
                        .fetch_add(bytes.len() as u64, Ordering::Relaxed);
                }
            }
        }
        if file.sync_all().is_err() || private_events.sync_all().is_err() {
            writer.failed.store(true, Ordering::Relaxed);
        }
        if trace_events.sync_all().is_err() {
            writer.failed.store(true, Ordering::Relaxed);
            writer.trace_failed.store(true, Ordering::Relaxed);
        }
    });
    *case.writer.lock().unwrap() = Some(handle);
    let weak = Arc::downgrade(&case);
    *case.trace_sink.lock().unwrap() = Some(trace::TraceSink::new(move |entry| {
        if let Some(case) = weak.upgrade() {
            record_trace_event(&case, entry);
        }
    }));
    Ok(case)
}

pub(super) fn record_private_event(case: &EvidenceCase, event: serde_json::Value) {
    if case.limited.load(Ordering::Relaxed) || case.failed.load(Ordering::Relaxed) {
        return;
    }
    let tx = case.tx.lock().unwrap();
    let Some(tx) = tx.as_ref() else {
        return;
    };
    let Ok(mut bytes) = serde_json::to_vec(
        &serde_json::json!({"elapsedMs":case.epoch.elapsed().as_millis() as u64,"event":event}),
    ) else {
        case.failed.store(true, Ordering::Relaxed);
        return;
    };
    if bytes.len() > 512 * 1024 {
        case.dropped.fetch_add(1, Ordering::Relaxed);
        return;
    }
    bytes.push(b'\n');
    if tx.try_send((ContentKind::Private, bytes)).is_err() {
        case.dropped.fetch_add(1, Ordering::Relaxed);
    }
}

fn trace_loss(case: &EvidenceCase) {
    case.trace_dropped.fetch_add(1, Ordering::Relaxed);
    case.dropped.fetch_add(1, Ordering::Relaxed);
}

pub(super) fn record_trace_event(case: &EvidenceCase, entry: trace::DebugEntry) {
    if case.limited.load(Ordering::Relaxed) || case.failed.load(Ordering::Relaxed) {
        case.trace_limited
            .store(case.limited.load(Ordering::Relaxed), Ordering::Relaxed);
        case.trace_failed
            .store(case.failed.load(Ordering::Relaxed), Ordering::Relaxed);
        trace_loss(case);
        return;
    }
    let tx = case.tx.lock().unwrap();
    let Some(tx) = tx.as_ref() else {
        trace_loss(case);
        return;
    };
    let Ok(mut bytes) = serde_json::to_vec(&entry) else {
        case.trace_failed.store(true, Ordering::Relaxed);
        case.failed.store(true, Ordering::Relaxed);
        trace_loss(case);
        return;
    };
    bytes.push(b'\n');
    if bytes.len() > 512 * 1024 || tx.try_send((ContentKind::Trace, bytes)).is_err() {
        trace_loss(case);
    }
}

pub(super) fn seal_case_trace(
    case: &EvidenceCase,
    timeout: std::time::Duration,
) -> Result<(), &'static str> {
    let target = case.trace_sink.lock().unwrap().clone();
    if target.is_some_and(|target| !target.wait_idle(timeout)) {
        return Err("development_evidence_trace_busy");
    }
    case.tx.lock().unwrap().take();
    case.trace_sink.lock().unwrap().take();
    Ok(())
}

pub(super) fn validate_case_id(actual: &str, requested: &str) -> Result<(), &'static str> {
    if actual != requested {
        return Err("development_evidence_case_changed");
    }
    Ok(())
}

pub(super) fn require_finalized(case: &EvidenceCase) -> Result<(), &'static str> {
    if !case.finalized.load(Ordering::Acquire) {
        return Err("development_evidence_requires_stop");
    }
    Ok(())
}

pub(super) fn persist_case_report(
    case_directory: &Path,
    report: &DebuggerSnapshot,
) -> Result<(), &'static str> {
    let temporary = case_directory.join(format!(".trace-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file =
            private_file(&temporary).map_err(|_| "development_evidence_storage_failed")?;
        serde_json::to_writer_pretty(&mut file, report)
            .map_err(|_| "development_evidence_storage_failed")?;
        file.flush()
            .map_err(|_| "development_evidence_storage_failed")?;
        fs::rename(&temporary, case_directory.join("trace.json"))
            .map_err(|_| "development_evidence_storage_failed")
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn case_audio_relative_path(source: AudioSource) -> &'static str {
    match source {
        AudioSource::System => "audio-evidence/system.wav",
        AudioSource::Microphone => "audio-evidence/microphone.wav",
    }
}

pub(super) fn case_audio_path(case_directory: &Path, source: AudioSource) -> PathBuf {
    case_directory.join(case_audio_relative_path(source))
}

pub(super) fn read_case_audio(
    case_directory: &Path,
    source: AudioSource,
) -> Result<Vec<u8>, &'static str> {
    let file =
        File::open(case_audio_path(case_directory, source)).map_err(|_| "audio_evidence_empty")?;
    let length = file.metadata().map_err(|_| "audio_read_failed")?.len();
    if !(44..=AUDIO_FILE_BYTE_LIMIT).contains(&length) {
        return Err("audio_evidence_invalid");
    }
    let mut bytes = Vec::with_capacity(length as usize);
    file.take(AUDIO_FILE_BYTE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "audio_read_failed")?;
    if bytes.len() as u64 != length
        || bytes.get(0..4) != Some(b"RIFF")
        || bytes.get(8..12) != Some(b"WAVE")
    {
        return Err("audio_evidence_invalid");
    }
    Ok(bytes)
}

pub(super) fn copy_case_audio(
    case_directory: &Path,
    source: AudioSource,
    destination: &Path,
) -> Result<bool, &'static str> {
    let relative = case_audio_relative_path(source);
    let source_path = case_directory.join(relative);
    if !source_path.exists() {
        return Ok(false);
    }
    let length = fs::metadata(&source_path)
        .map_err(|_| "development_export_failed")?
        .len();
    if !(44..=AUDIO_FILE_BYTE_LIMIT).contains(&length) {
        return Err("audio_evidence_invalid");
    }
    copy_case_file(case_directory, relative, destination, AUDIO_FILE_BYTE_LIMIT)
}

pub(super) fn read_saved_report(directory: &Path) -> Result<DebuggerSnapshot, &'static str> {
    let file =
        File::open(directory.join("trace.json")).map_err(|_| "development_case_read_failed")?;
    if file
        .metadata()
        .map_err(|_| "development_case_read_failed")?
        .len()
        > 4 * 1024 * 1024
    {
        return Err("development_case_invalid");
    }
    let mut report: DebuggerSnapshot = serde_json::from_reader(file.take(4 * 1024 * 1024 + 1))
        .map_err(|_| "development_case_invalid")?;
    if report.trace.schema_version != 1
        || report.trace.enabled
        || report.trace.entries.len() > trace::ENTRY_LIMIT
        || report.content_bytes > CONTENT_BYTE_LIMIT
        || report.trace_bytes > report.content_bytes
    {
        return Err("development_case_invalid");
    }
    if let Some(route) = report.route.as_object_mut() {
        route
            .entry("evidenceWorkspace")
            .or_insert_with(|| serde_json::json!("default"));
    }
    Ok(report)
}

pub(super) fn saved_case_directory(root: &Path, case_id: &str) -> Result<PathBuf, &'static str> {
    let id = uuid::Uuid::parse_str(case_id).map_err(|_| "development_case_invalid")?;
    if id.to_string() != case_id {
        return Err("development_case_invalid");
    }
    let directory = root.join(case_id);
    if !fs::symlink_metadata(&directory)
        .map_err(|_| "development_case_read_failed")?
        .is_dir()
    {
        return Err("development_case_invalid");
    }
    Ok(directory)
}

pub(super) fn read_trace_page(
    directory: &Path,
    offset: usize,
    limit: usize,
) -> Result<Vec<trace::DebugEntry>, &'static str> {
    if limit == 0 || limit > 64 {
        return Err("development_debug_batch_limit");
    }
    let file = open_case_file(directory, "trace-events.jsonl", CONTENT_BYTE_LIMIT)?;
    BufReader::new(file.take(CONTENT_BYTE_LIMIT))
        .lines()
        .skip(offset)
        .take(limit)
        .map(|line| {
            let line = line.map_err(|_| "development_case_read_failed")?;
            if line.len() > 512 * 1024 {
                return Err("development_case_invalid");
            }
            serde_json::from_str(&line).map_err(|_| "development_case_invalid")
        })
        .collect()
}
