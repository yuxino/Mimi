//! Development-only commands. Content evidence is private and explicitly enabled,
//! separate from the content-free diagnostic journal.
use crate::commands::AppState;
use crate::core::audio_input::AudioSource;
use crate::core::development_debug::{self as trace, FrontendObservation};
use crate::session_manager::SessionStateEvent;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

const CONTENT_BYTE_LIMIT: u64 = 16 * 1024 * 1024;
const ALL_CASES_BYTE_LIMIT: u64 = 128 * 1024 * 1024;
const AUDIO_FILE_BYTE_LIMIT: u64 = 20 * 1024 * 1024;
const CASE_RESERVED_BYTES: u64 = CONTENT_BYTE_LIMIT + AUDIO_FILE_BYTE_LIMIT + 4 * 1024 * 1024;
const CASE_COUNT_LIMIT: usize = 64;
type ContentSender = mpsc::SyncSender<(bool, Vec<u8>)>;

#[derive(Default)]
struct OperationGate {
    busy: AtomicBool,
}
struct OperationGuard<'a> {
    gate: &'a OperationGate,
}
impl OperationGate {
    fn try_begin(&self) -> Result<OperationGuard<'_>, &'static str> {
        self.busy
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|_| "development_debug_busy")?;
        Ok(OperationGuard { gate: self })
    }
}
impl Drop for OperationGuard<'_> {
    fn drop(&mut self) {
        self.gate.busy.store(false, Ordering::Release);
    }
}
fn operation_gate() -> &'static OperationGate {
    static GATE: OnceLock<OperationGate> = OnceLock::new();
    GATE.get_or_init(Default::default)
}

struct EvidenceCase {
    id: String,
    directory: PathBuf,
    epoch: Instant,
    tx: Mutex<Option<ContentSender>>,
    write_gate: Mutex<()>,
    snapshots: AtomicU64,
    private_events: AtomicU64,
    bytes: AtomicU64,
    dropped: AtomicU64,
    limited: AtomicBool,
    failed: AtomicBool,
    writer: Mutex<Option<std::thread::JoinHandle<()>>>,
    finalized: AtomicBool,
}
#[derive(Default)]
struct DebuggerState {
    root: Option<PathBuf>,
    case: Option<Arc<EvidenceCase>>,
    route: serde_json::Value,
    loaded_report: Option<DebuggerSnapshot>,
}
fn state() -> &'static Mutex<DebuggerState> {
    static STATE: OnceLock<Mutex<DebuggerState>> = OnceLock::new();
    STATE.get_or_init(Default::default)
}
#[derive(Default)]
struct FlushState {
    nonce: u64,
    pending: Vec<trace::DebugWindow>,
}
fn flush_state() -> &'static Mutex<FlushState> {
    static FLUSH: OnceLock<Mutex<FlushState>> = OnceLock::new();
    FLUSH.get_or_init(Default::default)
}
fn flush_notify() -> &'static tokio::sync::Notify {
    static NOTIFY: OnceLock<tokio::sync::Notify> = OnceLock::new();
    NOTIFY.get_or_init(Default::default)
}
pub fn initialize(app: &AppHandle) {
    let available = crate::windows::is_dev_build()
        && app.config().identifier == crate::settings_store::DEVELOPMENT_APPLICATION_IDENTIFIER;
    trace::initialize(available);
    if available {
        state().lock().unwrap().root = app
            .path()
            .app_data_dir()
            .ok()
            .map(|p| p.join("development-evidence"));
    }
}
fn available() -> Result<(), &'static str> {
    if trace::snapshot(0).available {
        Ok(())
    } else {
        Err("development_debug_unavailable")
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
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}
fn stored_bytes(root: &Path) -> std::io::Result<u64> {
    if !root.exists() {
        return Ok(0);
    }
    let mut bytes = 0_u64;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            bytes = bytes.saturating_add(stored_bytes(&entry.path())?);
        } else if entry.file_type()?.is_file() {
            bytes = bytes.saturating_add(entry.metadata()?.len());
        }
    }
    Ok(bytes)
}
fn start_case(
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
    let (tx, rx) = mpsc::sync_channel::<(bool, Vec<u8>)>(32);
    let case = Arc::new(EvidenceCase {
        id,
        directory,
        epoch,
        tx: Mutex::new(Some(tx)),
        write_gate: Mutex::new(()),
        snapshots: AtomicU64::new(0),
        private_events: AtomicU64::new(0),
        bytes: AtomicU64::new(0),
        dropped: AtomicU64::new(0),
        limited: AtomicBool::new(false),
        failed: AtomicBool::new(false),
        writer: Mutex::new(None),
        finalized: AtomicBool::new(false),
    });
    let writer = Arc::clone(&case);
    let handle = std::thread::spawn(move || {
        while let Ok((snapshot, bytes)) = rx.recv() {
            if writer
                .bytes
                .load(Ordering::Relaxed)
                .saturating_add(bytes.len() as u64)
                > CONTENT_BYTE_LIMIT
            {
                writer.limited.store(true, Ordering::Relaxed);
                writer.dropped.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            let _gate = writer.write_gate.lock().unwrap();
            if (if snapshot {
                file.write_all(&bytes)
            } else {
                private_events.write_all(&bytes)
            })
            .is_err()
            {
                writer.failed.store(true, Ordering::Relaxed);
                writer.dropped.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            writer
                .bytes
                .fetch_add(bytes.len() as u64, Ordering::Relaxed);
            if snapshot {
                writer.snapshots.fetch_add(1, Ordering::Relaxed);
            } else {
                writer.private_events.fetch_add(1, Ordering::Relaxed);
            }
        }
    });
    *case.writer.lock().unwrap() = Some(handle);
    Ok(case)
}
pub fn record_snapshot(
    snapshot: &SessionStateEvent,
    settings: &crate::settings_store::SettingsStore,
) {
    let case = state().lock().unwrap().case.clone();
    let Some(case) = case else {
        return;
    };
    let tx = case.tx.lock().unwrap();
    let Some(tx) = tx.as_ref() else {
        return;
    };
    if case.limited.load(Ordering::Relaxed) || case.failed.load(Ordering::Relaxed) {
        return;
    }
    // Only the existing bounded display is serialized; no full-session memory.
    let preferences = settings.preferences();
    let mut bytes = match serde_json::to_vec(
        &serde_json::json!({"elapsedMs":case.epoch.elapsed().as_millis() as u64,"snapshot":snapshot,"projectionSettings":{
            "sourceLanguage":preferences.source_language,"targetLanguage":preferences.target_language,"subtitleDisplayMode":preferences.subtitle_display_mode,"fontSize":preferences.font_size,
            "subtitleAlignment":preferences.subtitle_alignment,"subtitleColor":preferences.subtitle_color,"blendsWithBackground":preferences.subtitle_blends_with_background,
        }}),
    ) {
        Ok(bytes) => bytes,
        Err(_) => {
            case.failed.store(true, Ordering::Relaxed);
            return;
        }
    };
    if bytes.len() > 512 * 1024 {
        case.dropped.fetch_add(1, Ordering::Relaxed);
        return;
    }
    bytes.push(b'\n');
    if tx.try_send((true, bytes)).is_err() {
        case.dropped.fetch_add(1, Ordering::Relaxed);
    }
}
fn record_private_event(case: &EvidenceCase, event: serde_json::Value) {
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
    if tx.try_send((false, bytes)).is_err() {
        case.dropped.fetch_add(1, Ordering::Relaxed);
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DebuggerSnapshot {
    trace: trace::DebugSnapshot,
    route: serde_json::Value,
    audio: serde_json::Value,
    case_id: Option<String>,
    replay_snapshots: u64,
    #[serde(default)]
    private_events: u64,
    content_bytes: u64,
    content_dropped: u64,
    content_limited: bool,
    content_failed: bool,
}
#[tauri::command]
pub fn development_debug_snapshot() -> DebuggerSnapshot {
    let state = state().lock().unwrap();
    if let Some(report) = &state.loaded_report {
        let mut report = report.clone();
        report.trace.entries = report
            .trace
            .entries
            .into_iter()
            .rev()
            .take(trace::VIEW_LIMIT)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        return report;
    }
    let case = state.case.as_ref();
    DebuggerSnapshot {
        trace: trace::snapshot(trace::VIEW_LIMIT),
        route: state.route.clone(),
        audio: if case.is_some() {
            serde_json::to_value(crate::development_audio::status()).unwrap_or_default()
        } else {
            serde_json::json!({})
        },
        case_id: case.map(|c| c.id.clone()),
        replay_snapshots: case.map_or(0, |c| c.snapshots.load(Ordering::Relaxed)),
        private_events: case.map_or(0, |c| c.private_events.load(Ordering::Relaxed)),
        content_bytes: case.map_or(0, |c| c.bytes.load(Ordering::Relaxed)),
        content_dropped: case.map_or(0, |c| c.dropped.load(Ordering::Relaxed)),
        content_limited: case.is_some_and(|c| c.limited.load(Ordering::Relaxed)),
        content_failed: case.is_some_and(|c| c.failed.load(Ordering::Relaxed)),
    }
}
fn bound_case(case_id: &str) -> Result<Arc<EvidenceCase>, &'static str> {
    let case = state()
        .lock()
        .unwrap()
        .case
        .clone()
        .ok_or("development_evidence_empty")?;
    validate_case_id(&case.id, case_id)?;
    Ok(case)
}
fn validate_case_id(actual: &str, requested: &str) -> Result<(), &'static str> {
    if actual != requested {
        return Err("development_evidence_case_changed");
    }
    Ok(())
}
fn require_finalized(case: &EvidenceCase) -> Result<(), &'static str> {
    if !case.finalized.load(Ordering::Acquire) {
        return Err("development_evidence_requires_stop");
    }
    Ok(())
}
fn persist_case_report(
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

fn case_audio_path(case_directory: &Path, source: AudioSource) -> PathBuf {
    case_directory.join("audio-evidence").join(match source {
        AudioSource::System => "system.wav",
        AudioSource::Microphone => "microphone.wav",
    })
}
fn read_case_audio(case_directory: &Path, source: AudioSource) -> Result<Vec<u8>, &'static str> {
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
fn copy_case_audio(
    case_directory: &Path,
    source: AudioSource,
    destination: &Path,
) -> Result<bool, &'static str> {
    let source = case_audio_path(case_directory, source);
    if !source.exists() {
        return Ok(false);
    }
    let length = fs::metadata(&source)
        .map_err(|_| "development_export_failed")?
        .len();
    if !(44..=AUDIO_FILE_BYTE_LIMIT).contains(&length) {
        return Err("audio_evidence_invalid");
    }
    fs::copy(source, destination).map_err(|_| "development_export_failed")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(destination, fs::Permissions::from_mode(0o600))
            .map_err(|_| "development_export_failed")?;
    }
    Ok(true)
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedCase {
    id: String,
    created_at_unix_ms: u64,
    snapshots: u64,
    provider: serde_json::Value,
}
fn read_saved_report(directory: &Path) -> Result<DebuggerSnapshot, &'static str> {
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
    let report: DebuggerSnapshot = serde_json::from_reader(file.take(4 * 1024 * 1024 + 1))
        .map_err(|_| "development_case_invalid")?;
    if report.trace.schema_version != 1
        || report.trace.enabled
        || report.trace.entries.len() > trace::ENTRY_LIMIT
        || report.content_bytes > CONTENT_BYTE_LIMIT
    {
        return Err("development_case_invalid");
    }
    Ok(report)
}
fn saved_case_directory(root: &Path, case_id: &str) -> Result<PathBuf, &'static str> {
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
#[tauri::command]
pub async fn development_debug_cases() -> Result<Vec<SavedCase>, String> {
    available()?;
    let root = state()
        .lock()
        .unwrap()
        .root
        .clone()
        .ok_or("development_evidence_storage_failed")?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut cases = Vec::new();
        if !root.exists() {
            return Ok(cases);
        }
        for entry in fs::read_dir(&root)
            .map_err(|_| "development_case_read_failed")?
            .take(CASE_COUNT_LIMIT)
        {
            let entry = entry.map_err(|_| "development_case_read_failed")?;
            let id = entry.file_name().to_string_lossy().into_owned();
            let Ok(directory) = saved_case_directory(&root, &id) else {
                continue;
            };
            let Ok(report) = read_saved_report(&directory) else {
                continue;
            };
            if report.case_id.as_deref() != Some(&id) {
                continue;
            }
            cases.push(SavedCase {
                id,
                created_at_unix_ms: report.route["createdAtUnixMs"].as_u64().unwrap_or_default(),
                snapshots: report.replay_snapshots,
                provider: report.route["provider"].clone(),
            });
        }
        cases.sort_by_key(|case| std::cmp::Reverse(case.created_at_unix_ms));
        Ok::<_, &'static str>(cases)
    })
    .await
    .map_err(|_| "development_case_read_failed")?
    .map_err(str::to_string)
}
#[tauri::command]
pub async fn development_debug_open_case(case_id: String) -> Result<(), String> {
    available()?;
    let operation = operation_gate().try_begin()?;
    if trace::is_enabled() {
        return Err("development_evidence_requires_stop".into());
    }
    let root = state()
        .lock()
        .unwrap()
        .root
        .clone()
        .ok_or("development_evidence_storage_failed")?;
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = operation;
        let directory = saved_case_directory(&root, &case_id)?;
        let mut report = read_saved_report(&directory)?;
        validate_case_id(
            report
                .case_id
                .as_deref()
                .ok_or("development_case_invalid")?,
            &case_id,
        )?;
        report.trace.available = true;
        let case = Arc::new(EvidenceCase {
            id: case_id,
            directory,
            epoch: Instant::now(),
            tx: Mutex::new(None),
            write_gate: Mutex::new(()),
            snapshots: AtomicU64::new(report.replay_snapshots),
            private_events: AtomicU64::new(report.private_events),
            bytes: AtomicU64::new(report.content_bytes),
            dropped: AtomicU64::new(report.content_dropped),
            limited: AtomicBool::new(report.content_limited),
            failed: AtomicBool::new(report.content_failed),
            writer: Mutex::new(None),
            finalized: AtomicBool::new(true),
        });
        let mut debugger = state().lock().unwrap();
        debugger.case = Some(case);
        debugger.route = report.route.clone();
        debugger.loaded_report = Some(report);
        Ok::<(), &'static str>(())
    })
    .await
    .map_err(|_| "development_case_read_failed")??;
    Ok(())
}
#[tauri::command]
pub async fn development_debug_private_events(
    case_id: String,
    offset: usize,
) -> Result<Vec<serde_json::Value>, String> {
    available()?;
    let operation = operation_gate().try_begin()?;
    let case = bound_case(&case_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = operation;
        let _gate = case.write_gate.lock().unwrap();
        let file = File::open(case.directory.join("events.jsonl"))
            .map_err(|_| "development_case_read_failed")?;
        BufReader::new(file.take(CONTENT_BYTE_LIMIT))
            .lines()
            .skip(offset)
            .take(32)
            .map(|line| {
                let line = line.map_err(|_| "development_case_read_failed")?;
                if line.len() > 512 * 1024 {
                    return Err("development_case_invalid");
                }
                serde_json::from_str(&line).map_err(|_| "development_case_invalid")
            })
            .collect::<Result<Vec<serde_json::Value>, &'static str>>()
    })
    .await
    .map_err(|_| "development_case_read_failed")?
    .map_err(str::to_string)
}

#[tauri::command]
pub fn development_debug_start(
    app: AppHandle,
    app_state: State<'_, AppState>,
    with_audio: bool,
) -> Result<(), String> {
    available()?;
    let _operation = operation_gate().try_begin()?;
    if trace::is_enabled() {
        return Err("development_debug_already_running".into());
    }
    if with_audio && app_state.session.is_active() {
        return Err("development_recording_requires_stop".into());
    }
    let preferences = app_state.settings.preferences();
    let epoch = Instant::now();
    let profile = app_state.settings.active_profile().ok();
    let route = serde_json::json!({
        "appVersion":app.package_info().version.to_string(), "buildRevision":option_env!("MIMI_DEBUG_REVISION").unwrap_or("unknown"), "uiTest":app_state.settings.is_ui_test(),
        "createdAtUnixMs":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64,"clock":"sharedMonotonicEpoch",
        "provider":profile.as_ref().map(|p|p.provider),"textTranslation":profile.as_ref().map(|p|p.text_translation()),
        "sourceLanguage":preferences.source_language,"targetLanguage":preferences.target_language,"translationMode":preferences.translation_mode,
        "audioInput":preferences.audio_input,"subtitleDisplayMode":preferences.subtitle_display_mode,"fontSize":preferences.font_size,
        "blendsWithBackground":preferences.subtitle_blends_with_background,"recordedContent":with_audio,
    });
    let mut debugger = state().lock().unwrap();
    if let Some(case) = debugger.case.as_ref() {
        case.tx.lock().unwrap().take();
    }
    if with_audio {
        let case = start_case(
            debugger
                .root
                .clone()
                .ok_or("development_evidence_storage_failed")?,
            &route,
            epoch,
        )?;
        if let Err(error) =
            crate::development_audio::configure_at(case.directory.clone(), true, epoch)
        {
            case.tx.lock().unwrap().take();
            return Err(error.into());
        }
        let weak = Arc::downgrade(&case);
        crate::development_content::configure(Some(Arc::new(move |event| {
            if let Some(case) = weak.upgrade() {
                record_private_event(&case, event);
            }
        })));
        debugger.case = Some(case);
    } else {
        crate::development_audio::configure(PathBuf::new(), false)?;
        debugger.case = None;
        crate::development_content::configure(None);
    }
    debugger.route = route;
    debugger.loaded_report = None;
    trace::set_enabled_at(true, epoch)?;
    drop(debugger);
    app_state.session.publish_state();
    Ok(())
}
#[tauri::command]
pub async fn development_debug_stop(app: AppHandle) -> Result<(), String> {
    available()?;
    let operation = operation_gate().try_begin()?;
    if !trace::is_enabled()
        && state()
            .lock()
            .unwrap()
            .case
            .as_ref()
            .is_none_or(|case| case.finalized.load(Ordering::Acquire))
    {
        return Ok(());
    }
    let nonce = {
        let mut flush = flush_state().lock().unwrap();
        flush.nonce += 1;
        flush.pending = trace::observed_windows();
        flush.nonce
    };
    let _ = app.emit(
        "development-debug-flush",
        serde_json::json!({"nonce":nonce}),
    );
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(750);
    loop {
        let notified = flush_notify().notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if flush_state().lock().unwrap().pending.is_empty() {
            break;
        }
        if tokio::time::timeout_at(deadline, notified).await.is_err() {
            break;
        }
    }
    trace::record(trace::DebugEvent::Stopped {
        unflushed_windows: flush_state().lock().unwrap().pending.clone(),
    });
    trace::set_enabled(false)?;
    crate::development_content::configure(None);
    let case = state().lock().unwrap().case.clone();
    if let Some(case) = &case {
        case.tx.lock().unwrap().take();
    }
    tauri::async_runtime::spawn_blocking(move || {
        // Move the lease into the worker: cancellation of the IPC future must
        // not allow a new case while this worker still stops the old recorder.
        let _operation = operation;
        crate::development_audio::configure(PathBuf::new(), false)?;
        if let Some(case) = case {
            if let Some(writer) = case.writer.lock().unwrap().take() {
                writer
                    .join()
                    .map_err(|_| "development_evidence_writer_failed")?;
            }
            let mut report = development_debug_snapshot();
            report.trace = trace::snapshot(trace::ENTRY_LIMIT);
            persist_case_report(&case.directory, &report)?;
            case.finalized.store(true, Ordering::Release);
        }
        Ok::<(), &'static str>(())
    })
    .await
    .map_err(|_| "development_audio_stop_failed")??;
    Ok(())
}
/// Normal quit must finalize a private case just like the explicit stop button.
/// Production and a dev launch with collection off do no extra work.
pub async fn finish_on_quit(app: AppHandle) -> Result<(), String> {
    if trace::snapshot(0).available {
        development_debug_stop(app).await?;
    }
    Ok(())
}
#[tauri::command]
pub fn development_debug_flush_ack(nonce: u64, window: trace::DebugWindow) {
    let mut flush = flush_state().lock().unwrap();
    if flush.nonce == nonce {
        flush.pending.retain(|pending| *pending != window);
        flush_notify().notify_one();
    }
}
#[tauri::command]
pub fn development_debug_observe(observations: Vec<FrontendObservation>) -> Result<(), String> {
    available()?;
    if observations.len() > trace::BATCH_LIMIT {
        return Err("development_debug_batch_limit".into());
    }
    for observation in observations {
        for value in [
            observation.scroll_top,
            observation.scroll_height,
            observation.viewport_height,
            observation.viewport_width,
        ]
        .into_iter()
        .flatten()
        {
            if !value.is_finite() || value.abs() > 10_000_000.0 {
                return Err("development_debug_invalid_observation".into());
            }
        }
        trace::record(trace::DebugEvent::Frontend { observation });
    }
    Ok(())
}
#[tauri::command]
pub async fn development_debug_audio(
    source: AudioSource,
    case_id: String,
) -> Result<tauri::ipc::Response, String> {
    available()?;
    let operation = operation_gate().try_begin()?;
    if trace::is_enabled() {
        return Err("development_evidence_requires_stop".into());
    }
    let case = bound_case(&case_id)?;
    require_finalized(&case)?;
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        let _operation = operation;
        read_case_audio(&case.directory, source)
    })
    .await
    .map_err(|_| "development_audio_read_failed")??;
    Ok(tauri::ipc::Response::new(bytes))
}
#[tauri::command]
pub async fn development_debug_replay(
    index: usize,
    case_id: String,
) -> Result<serde_json::Value, String> {
    available()?;
    let operation = operation_gate().try_begin()?;
    let case = bound_case(&case_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = operation;
        let _gate = case.write_gate.lock().unwrap();
        let file = File::open(case.directory.join("snapshots.jsonl"))
            .map_err(|_| "development_replay_read_failed")?;
        let line = BufReader::new(file.take(CONTENT_BYTE_LIMIT))
            .lines()
            .nth(index)
            .ok_or("development_replay_empty")?
            .map_err(|_| "development_replay_read_failed")?;
        if line.len() > 512 * 1024 {
            return Err("development_case_invalid");
        }
        serde_json::from_str(&line).map_err(|_| "development_replay_read_failed")
    })
    .await
    .map_err(|_| "development_replay_read_failed")?
    .map_err(str::to_string)
}
#[tauri::command]
pub async fn development_debug_export(app: AppHandle) -> Result<bool, String> {
    available()?;
    let operation = operation_gate().try_begin()?;
    if trace::is_enabled() {
        return Err("development_export_requires_stop".into());
    }
    let loaded_report = state().lock().unwrap().loaded_report.clone();
    let loaded = loaded_report.is_some();
    let mut report = loaded_report.unwrap_or_else(development_debug_snapshot);
    if !loaded {
        report.trace = trace::snapshot(trace::ENTRY_LIMIT);
    }
    let case = state().lock().unwrap().case.clone();
    if let Some(case) = &case {
        require_finalized(case)?;
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |path| {
        let _ = tx.send(path);
    });
    let Some(path) = rx.await.map_err(|_| "development_export_failed")? else {
        return Ok(false);
    };
    let parent = path.into_path().map_err(|_| "development_export_failed")?;
    tauri::async_runtime::spawn_blocking(move || -> Result<(), &'static str> {
        let _operation = operation;
        let directory = parent.join(format!("mimi-debug-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&directory).map_err(|_| "development_export_failed")?;
        private_directory(&directory).map_err(|_| "development_export_failed")?;
        let mut file =
            private_file(&directory.join("trace.json")).map_err(|_| "development_export_failed")?;
        serde_json::to_writer_pretty(&mut file, &report)
            .map_err(|_| "development_export_failed")?;
        if let Some(case) = case {
            let _gate = case.write_gate.lock().unwrap();
            for name in ["manifest.json", "snapshots.jsonl", "events.jsonl"] {
                fs::copy(case.directory.join(name), directory.join(name))
                    .map_err(|_| "development_export_failed")?;
            }
            if case
                .directory
                .join("audio-evidence/audio-index.jsonl")
                .exists()
            {
                fs::copy(
                    case.directory.join("audio-evidence/audio-index.jsonl"),
                    directory.join("audio-index.jsonl"),
                )
                .map_err(|_| "development_export_failed")?;
            }
            for source in [AudioSource::System, AudioSource::Microphone] {
                let name = match source {
                    AudioSource::System => "sent-system.wav",
                    AudioSource::Microphone => "sent-microphone.wav",
                };
                copy_case_audio(&case.directory, source, &directory.join(name))?;
            }
        }
        private_file(&directory.join("ANALYZE.md"))
            .map_err(|_| "development_export_failed")?
            .write_all(include_bytes!(
                "../../docs/development/debugger-evidence.md"
            ))
            .map_err(|_| "development_export_failed")?;
        Ok(())
    })
    .await
    .map_err(|_| "development_export_failed")??;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn operation_lease_excludes_other_operations_across_await_and_recovers_on_drop() {
        let gate = OperationGate::default();
        let operation = gate.try_begin().unwrap();
        tokio::task::yield_now().await;
        assert!(matches!(gate.try_begin(), Err("development_debug_busy")));
        drop(operation);
        let operation = gate.try_begin().unwrap();
        drop(operation);
        assert!(gate.try_begin().is_ok());
    }

    #[test]
    fn private_case_seals_and_reopens_the_same_bounded_recording() {
        let root = tempfile::tempdir().unwrap();
        let route = serde_json::json!({"provider":"alibabaCloud","createdAtUnixMs":123});
        let case = start_case(root.path().to_owned(), &route, Instant::now()).unwrap();
        record_private_event(
            &case,
            serde_json::json!({"kind":"synthetic","source":"example"}),
        );
        case.tx
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .send((true, b"{\"snapshot\":{\"synthetic\":true}}\n".to_vec()))
            .unwrap();
        case.tx.lock().unwrap().take();
        case.writer.lock().unwrap().take().unwrap().join().unwrap();
        assert_eq!(case.private_events.load(Ordering::Relaxed), 1);
        assert_eq!(case.snapshots.load(Ordering::Relaxed), 1);
        let event_bytes = fs::read(case.directory.join("events.jsonl")).unwrap();
        let snapshot_bytes = fs::read(case.directory.join("snapshots.jsonl")).unwrap();
        assert_eq!(
            case.bytes.load(Ordering::Relaxed),
            (event_bytes.len() + snapshot_bytes.len()) as u64
        );
        // A late callback to a sealed case cannot append or enter a new case.
        record_private_event(&case, serde_json::json!({"kind":"late"}));
        assert_eq!(
            fs::read(case.directory.join("events.jsonl")).unwrap(),
            event_bytes
        );
        let mut trace = trace::snapshot(0);
        trace.enabled = false;
        let mut report = DebuggerSnapshot {
            trace,
            route,
            audio: serde_json::json!({}),
            case_id: Some(case.id.clone()),
            replay_snapshots: 1,
            private_events: 1,
            content_bytes: case.bytes.load(Ordering::Relaxed),
            content_dropped: 0,
            content_limited: false,
            content_failed: false,
        };
        persist_case_report(&case.directory, &report).unwrap();
        let reopened =
            read_saved_report(&saved_case_directory(root.path(), &case.id).unwrap()).unwrap();
        assert_eq!(reopened.case_id, Some(case.id.clone()));
        assert_eq!(reopened.private_events, 1);
        assert_eq!(reopened.replay_snapshots, 1);
        assert!(saved_case_directory(root.path(), "../other").is_err());
        report.trace.enabled = true;
        persist_case_report(&case.directory, &report).unwrap();
        assert!(matches!(
            read_saved_report(&case.directory),
            Err("development_case_invalid")
        ));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&case.directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
            for name in [
                "manifest.json",
                "events.jsonl",
                "snapshots.jsonl",
                "trace.json",
            ] {
                assert_eq!(
                    fs::metadata(case.directory.join(name))
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777,
                    0o600
                );
            }
        }
    }

    #[test]
    fn starting_a_case_reserves_the_entire_case_budget() {
        let root = tempfile::tempdir().unwrap();
        private_file(&root.path().join("existing.bin"))
            .unwrap()
            .set_len(ALL_CASES_BYTE_LIMIT - CASE_RESERVED_BYTES + 1)
            .unwrap();
        assert!(matches!(
            start_case(
                root.path().to_owned(),
                &serde_json::json!({}),
                Instant::now()
            ),
            Err("development_evidence_storage_limit")
        ));
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn case_bound_audio_and_export_do_not_read_another_cases_recording() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("first");
        let second = root.path().join("second");
        for (directory, sample) in [(&first, 0x11), (&second, 0x22)] {
            private_directory(&directory.join("audio-evidence")).unwrap();
            let mut wav = vec![0; 48];
            wav[0..4].copy_from_slice(b"RIFF");
            wav[8..12].copy_from_slice(b"WAVE");
            wav[44..48].fill(sample);
            private_file(&case_audio_path(directory, AudioSource::System))
                .unwrap()
                .write_all(&wav)
                .unwrap();
        }
        let captured = first.clone();
        assert!(validate_case_id("second", "first").is_err());
        assert_eq!(
            &read_case_audio(&captured, AudioSource::System).unwrap()[44..],
            &[0x11; 4]
        );
        let destination = root.path().join("export.wav");
        assert!(copy_case_audio(&captured, AudioSource::System, &destination).unwrap());
        assert_eq!(&fs::read(&destination).unwrap()[44..], &[0x11; 4]);
        assert!(!copy_case_audio(
            &captured,
            AudioSource::Microphone,
            &root.path().join("missing.wav")
        )
        .unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(destination).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let oversize = private_file(&case_audio_path(&first, AudioSource::Microphone)).unwrap();
        oversize.set_len(AUDIO_FILE_BYTE_LIMIT + 1).unwrap();
        assert!(read_case_audio(&first, AudioSource::Microphone).is_err());
        assert!(copy_case_audio(
            &first,
            AudioSource::Microphone,
            &root.path().join("oversize.wav")
        )
        .is_err());
    }
}
