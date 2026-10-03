//! Development-only commands. Content evidence is private and explicitly enabled,
//! separate from the content-free diagnostic journal.
use crate::commands::AppState;
use crate::core::audio_input::AudioSource;
use crate::core::development_debug::{self as trace, FrontendObservation};
use crate::core::development_evidence_workspace::{
    valid_workspace_name, EvidenceWorkspace, EXTRA_WORKSPACE_LIMIT,
};
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
#[derive(Clone, Copy)]
enum ContentKind {
    Snapshot,
    Private,
    Trace,
}
type ContentSender = mpsc::SyncSender<(ContentKind, Vec<u8>)>;

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
    trace_entries: AtomicU64,
    trace_bytes: AtomicU64,
    trace_dropped: AtomicU64,
    trace_limited: AtomicBool,
    trace_failed: AtomicBool,
    trace_persistence_enabled: bool,
    trace_sink: Mutex<Option<trace::TraceSink>>,
    bytes: AtomicU64,
    dropped: AtomicU64,
    limited: AtomicBool,
    failed: AtomicBool,
    writer: Mutex<Option<std::thread::JoinHandle<()>>>,
    finalized: AtomicBool,
}
#[derive(Default)]
struct DebuggerState {
    app_directory: Option<PathBuf>,
    root: Option<PathBuf>,
    workspace: EvidenceWorkspace,
    storage_error: Option<&'static str>,
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
    let selection = initialize_evidence_storage(
        available,
        || app.path().app_data_dir().ok(),
        || std::env::var_os("MIMI_DEVELOPMENT_EVIDENCE_WORKSPACE"),
    );
    let mut debugger = state().lock().unwrap();
    match selection {
        Ok(Some((app_directory, root, workspace))) => {
            debugger.route = serde_json::json!({"evidenceWorkspace":workspace.label()});
            debugger.app_directory = Some(app_directory);
            debugger.root = Some(root);
            debugger.workspace = workspace;
        }
        Ok(None) => {}
        Err(error) => {
            debugger.storage_error = Some(error);
            debugger.route = serde_json::json!({"evidenceWorkspaceError":error});
        }
    }
}

// Both closures are deliberately evaluated only after the exact dev gate.
// Production must neither read this selector nor create an evidence directory.
fn initialize_evidence_storage(
    available: bool,
    app_directory: impl FnOnce() -> Option<PathBuf>,
    selector: impl FnOnce() -> Option<std::ffi::OsString>,
) -> Result<Option<(PathBuf, PathBuf, EvidenceWorkspace)>, &'static str> {
    if !available {
        return Ok(None);
    }
    let value = selector();
    let workspace = EvidenceWorkspace::parse(
        value
            .as_ref()
            .map(|value| {
                value
                    .to_str()
                    .ok_or("development_evidence_workspace_invalid")
            })
            .transpose()?,
    )?;
    let app_directory = app_directory().ok_or("development_evidence_storage_failed")?;
    let root = select_evidence_root(&app_directory, &workspace)?;
    Ok(Some((app_directory, root, workspace)))
}

fn require_directory(path: &Path) -> Result<(), &'static str> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "development_evidence_storage_failed")?;
    if !metadata.is_dir() {
        return Err("development_evidence_workspace_invalid");
    }
    Ok(())
}

fn workspace_count(container: &Path) -> Result<usize, &'static str> {
    let mut count = 0;
    for entry in fs::read_dir(container).map_err(|_| "development_evidence_storage_failed")? {
        let entry = entry.map_err(|_| "development_evidence_storage_failed")?;
        let file_type = entry
            .file_type()
            .map_err(|_| "development_evidence_storage_failed")?;
        let name = entry.file_name();
        if name == ".DS_Store" {
            // Finder metadata is not an evidence catalog. Keep this exception
            // narrow and bounded; links and oversized files remain invalid.
            if !file_type.is_file()
                || entry
                    .metadata()
                    .map_err(|_| "development_evidence_storage_failed")?
                    .len()
                    > 64 * 1024
            {
                return Err("development_evidence_workspace_invalid");
            }
            continue;
        }
        if !file_type.is_dir() || !name.to_str().is_some_and(valid_workspace_name) {
            return Err("development_evidence_workspace_invalid");
        }
        count += 1;
        if count > EXTRA_WORKSPACE_LIMIT {
            return Err("development_evidence_workspace_limit");
        }
    }
    Ok(count)
}

fn existing_workspace_count(container: &Path) -> Result<usize, &'static str> {
    match fs::symlink_metadata(container) {
        Ok(_) => {
            require_directory(container)?;
            workspace_count(container)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(_) => Err("development_evidence_storage_failed"),
    }
}

fn workspace_directory(app_directory: &Path, workspace: &EvidenceWorkspace) -> PathBuf {
    match workspace {
        EvidenceWorkspace::Default => app_directory.join("development-evidence"),
        EvidenceWorkspace::Named(name) => app_directory
            .join("development-evidence-workspaces")
            .join(name),
    }
}

fn select_evidence_root(
    app_directory: &Path,
    workspace: &EvidenceWorkspace,
) -> Result<PathBuf, &'static str> {
    private_directory(app_directory).map_err(|error| {
        if error.kind() == std::io::ErrorKind::InvalidInput {
            "development_evidence_workspace_invalid"
        } else {
            "development_evidence_storage_failed"
        }
    })?;
    let root = workspace_directory(app_directory, workspace);
    let container = app_directory.join("development-evidence-workspaces");
    existing_workspace_count(&container)?;
    if let EvidenceWorkspace::Named(name) = workspace {
        if !valid_workspace_name(name) || name == "default" {
            return Err("development_evidence_workspace_invalid");
        }
        private_directory(&container).map_err(|error| {
            if error.kind() == std::io::ErrorKind::InvalidInput {
                "development_evidence_workspace_invalid"
            } else {
                "development_evidence_storage_failed"
            }
        })?;
        let count = workspace_count(&container)?;
        match fs::symlink_metadata(&root) {
            Ok(_) => require_directory(&root)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if count >= EXTRA_WORKSPACE_LIMIT {
                    return Err("development_evidence_workspace_limit");
                }
                let mut builder = fs::DirBuilder::new();
                builder.recursive(false);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;
                    builder.mode(0o700);
                }
                let created = match builder.create(&root) {
                    Ok(()) => true,
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
                    Err(_) => return Err("development_evidence_storage_failed"),
                };
                // Recheck after creation: competing initializations cannot retain
                // a ninth extra catalog. Only our newly created empty directory
                // is eligible for rollback; existing evidence is never removed.
                if let Err(error) = workspace_count(&container) {
                    if created {
                        let _ = fs::remove_dir(&root);
                    }
                    return Err(error);
                }
                require_directory(&root)?;
            }
            Err(_) => return Err("development_evidence_storage_failed"),
        }
    }
    private_directory(&root).map_err(|error| {
        if error.kind() == std::io::ErrorKind::InvalidInput {
            "development_evidence_workspace_invalid"
        } else {
            "development_evidence_storage_failed"
        }
    })?;
    Ok(root)
}

fn evidence_root(debugger: &DebuggerState) -> Result<PathBuf, &'static str> {
    if let Some(error) = debugger.storage_error {
        return Err(error);
    }
    let app_directory = debugger
        .app_directory
        .as_deref()
        .ok_or("development_evidence_storage_failed")?;
    require_directory(app_directory)?;
    let container = app_directory.join("development-evidence-workspaces");
    if matches!(debugger.workspace, EvidenceWorkspace::Named(_)) {
        require_directory(&container)?;
    }
    existing_workspace_count(&container)?;
    let root = debugger
        .root
        .as_ref()
        .ok_or("development_evidence_storage_failed")?;
    require_directory(root)?;
    Ok(root.clone())
}
fn available() -> Result<(), &'static str> {
    if trace::snapshot(0).available {
        Ok(())
    } else {
        Err("development_debug_unavailable")
    }
}
fn private_directory(path: &Path) -> std::io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.is_dir() => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "private evidence directory must be a regular directory",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    fs::create_dir_all(path)?;
    if !fs::symlink_metadata(path)?.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "private evidence directory must be a regular directory",
        ));
    }
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
fn open_case_file(directory: &Path, relative: &str, maximum: u64) -> Result<File, &'static str> {
    let path = directory.join(relative);
    let before = fs::symlink_metadata(&path).map_err(|_| "development_case_read_failed")?;
    if !before.is_file() || before.len() > maximum {
        return Err("development_case_invalid");
    }
    let canonical_directory = directory
        .canonicalize()
        .map_err(|_| "development_case_invalid")?;
    if !path
        .canonicalize()
        .map_err(|_| "development_case_invalid")?
        .starts_with(canonical_directory)
    {
        return Err("development_case_invalid");
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        #[cfg(target_os = "macos")]
        options.custom_flags(0x0100 | 0x0004); // O_NOFOLLOW | O_NONBLOCK.
        #[cfg(not(target_os = "macos"))]
        options.custom_flags(0x20000 | 0x0800);
    }
    let file = options
        .open(path)
        .map_err(|_| "development_case_read_failed")?;
    let after = file
        .metadata()
        .map_err(|_| "development_case_read_failed")?;
    if !after.is_file() || after.len() > maximum {
        return Err("development_case_invalid");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.ino() != after.ino() || before.dev() != after.dev() {
            return Err("development_case_invalid");
        }
    }
    Ok(file)
}
fn copy_case_file(
    directory: &Path,
    relative: &str,
    destination: &Path,
    maximum: u64,
) -> Result<bool, &'static str> {
    match fs::symlink_metadata(directory.join(relative)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err("development_export_failed"),
        Ok(_) => {}
    }
    let file = open_case_file(directory, relative, maximum)?;
    let mut destination = private_file(destination).map_err(|_| "development_export_failed")?;
    let bytes = std::io::copy(&mut file.take(maximum + 1), &mut destination)
        .map_err(|_| "development_export_failed")?;
    if bytes > maximum {
        return Err("development_export_failed");
    }
    Ok(true)
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
    if tx.try_send((ContentKind::Snapshot, bytes)).is_err() {
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
    if tx.try_send((ContentKind::Private, bytes)).is_err() {
        case.dropped.fetch_add(1, Ordering::Relaxed);
    }
}
fn trace_loss(case: &EvidenceCase) {
    case.trace_dropped.fetch_add(1, Ordering::Relaxed);
    case.dropped.fetch_add(1, Ordering::Relaxed);
}
fn record_trace_event(case: &EvidenceCase, entry: trace::DebugEntry) {
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
fn seal_case_trace(case: &EvidenceCase, timeout: std::time::Duration) -> Result<(), &'static str> {
    let target = case.trace_sink.lock().unwrap().clone();
    if target.is_some_and(|target| !target.wait_idle(timeout)) {
        return Err("development_evidence_trace_busy");
    }
    case.tx.lock().unwrap().take();
    case.trace_sink.lock().unwrap().take();
    Ok(())
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
    #[serde(default)]
    trace_persistence_enabled: bool,
    #[serde(default)]
    persisted_trace_entries: u64,
    #[serde(default)]
    trace_bytes: u64,
    #[serde(default)]
    trace_dropped: u64,
    #[serde(default)]
    trace_limited: bool,
    #[serde(default)]
    trace_failed: bool,
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
        trace_persistence_enabled: case.is_some_and(|c| c.trace_persistence_enabled),
        persisted_trace_entries: case.map_or(0, |c| c.trace_entries.load(Ordering::Relaxed)),
        trace_bytes: case.map_or(0, |c| c.trace_bytes.load(Ordering::Relaxed)),
        trace_dropped: case.map_or(0, |c| c.trace_dropped.load(Ordering::Relaxed)),
        trace_limited: case.is_some_and(|c| c.trace_limited.load(Ordering::Relaxed)),
        trace_failed: case.is_some_and(|c| c.trace_failed.load(Ordering::Relaxed)),
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
    let relative = match source.file_name().and_then(|name| name.to_str()) {
        Some("system.wav") => "audio-evidence/system.wav",
        Some("microphone.wav") => "audio-evidence/microphone.wav",
        _ => return Err("audio_evidence_invalid"),
    };
    copy_case_file(case_directory, relative, destination, AUDIO_FILE_BYTE_LIMIT)
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
    let root = evidence_root(&state().lock().unwrap())?;
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
    if let Some(case) = state().lock().unwrap().case.as_ref() {
        require_finalized(case)?;
    }
    let root = evidence_root(&state().lock().unwrap())?;
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
            trace_entries: AtomicU64::new(report.persisted_trace_entries),
            trace_bytes: AtomicU64::new(report.trace_bytes),
            trace_dropped: AtomicU64::new(report.trace_dropped),
            trace_limited: AtomicBool::new(report.trace_limited),
            trace_failed: AtomicBool::new(report.trace_failed),
            trace_persistence_enabled: report.trace_persistence_enabled,
            trace_sink: Mutex::new(None),
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
fn read_trace_page(
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
#[tauri::command]
pub async fn development_debug_trace_events(
    case_id: String,
    offset: usize,
    limit: usize,
) -> Result<Vec<trace::DebugEntry>, String> {
    available()?;
    if limit == 0 || limit > 64 {
        return Err("development_debug_batch_limit".into());
    }
    let operation = operation_gate().try_begin()?;
    let case = bound_case(&case_id)?;
    if !case.trace_persistence_enabled {
        return Err("development_trace_not_persisted".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = operation;
        let _gate = case.write_gate.lock().unwrap();
        read_trace_page(&case.directory, offset, limit)
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
    let mut debugger = state().lock().unwrap();
    let root = evidence_root(&debugger)?;
    let route = serde_json::json!({
        "appVersion":app.package_info().version.to_string(), "buildRevision":option_env!("MIMI_DEBUG_REVISION").unwrap_or("unknown"), "buildTreeState":option_env!("MIMI_DEBUG_TREE_STATE").unwrap_or("unknown"), "uiTest":app_state.settings.is_ui_test(),
        "createdAtUnixMs":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64,"clock":"sharedMonotonicEpoch",
        "provider":profile.as_ref().map(|p|p.provider),"textTranslation":profile.as_ref().map(|p|p.text_translation()),
        "sourceLanguage":preferences.source_language,"targetLanguage":preferences.target_language,"translationMode":preferences.translation_mode,
        "audioInput":preferences.audio_input,"subtitleDisplayMode":preferences.subtitle_display_mode,"fontSize":preferences.font_size,
        "blendsWithBackground":preferences.subtitle_blends_with_background,"recordedContent":with_audio,
        "evidenceWorkspace":debugger.workspace.label(),
    });
    if let Some(case) = debugger.case.as_ref() {
        require_finalized(case)?;
        case.tx.lock().unwrap().take();
    }
    if with_audio {
        let case = start_case(root, &route, epoch)?;
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
        trace::set_sink(case.trace_sink.lock().unwrap().clone())?;
        debugger.case = Some(case);
    } else {
        crate::development_audio::configure(PathBuf::new(), false)?;
        debugger.case = None;
        crate::development_content::configure(None);
        trace::set_sink(None)?;
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
    tauri::async_runtime::spawn_blocking(move || {
        // Move the lease into the worker: cancellation of the IPC future must
        // not allow a new case while this worker still stops the old recorder.
        let _operation = operation;
        if let Some(case) = &case {
            seal_case_trace(case, std::time::Duration::from_secs(5))?;
        }
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
                if !copy_case_file(
                    &case.directory,
                    name,
                    &directory.join(name),
                    CONTENT_BYTE_LIMIT,
                )? {
                    return Err("development_export_failed");
                }
            }
            let copied_trace = copy_case_file(
                &case.directory,
                "trace-events.jsonl",
                &directory.join("trace-events.jsonl"),
                CONTENT_BYTE_LIMIT,
            )?;
            if case.trace_persistence_enabled && !copied_trace {
                return Err("development_export_failed");
            }
            if case
                .directory
                .join("audio-evidence/audio-index.jsonl")
                .exists()
            {
                copy_case_file(
                    &case.directory,
                    "audio-evidence/audio-index.jsonl",
                    &directory.join("audio-index.jsonl"),
                    AUDIO_FILE_BYTE_LIMIT,
                )?;
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

    #[test]
    fn evidence_workspace_is_ignored_before_the_exact_dev_gate() {
        let root = tempfile::tempdir().unwrap();
        assert!(initialize_evidence_storage(
            false,
            || panic!("production must not resolve the evidence root"),
            || panic!("production must not read the workspace environment"),
        )
        .unwrap()
        .is_none());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[test]
    fn invalid_evidence_workspace_fails_before_creating_directories() {
        let root = tempfile::tempdir().unwrap();
        for selector in ["../outside", "bad/name", "-batch", "白"] {
            assert_eq!(
                initialize_evidence_storage(
                    true,
                    || Some(root.path().join("app")),
                    || Some(selector.into()),
                ),
                Err("development_evidence_workspace_invalid")
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            assert_eq!(
                initialize_evidence_storage(
                    true,
                    || Some(root.path().join("app")),
                    || Some(std::ffi::OsString::from_vec(vec![0xff])),
                ),
                Err("development_evidence_workspace_invalid")
            );
        }
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[test]
    fn evidence_workspaces_preserve_the_default_and_limit_extra_catalogs() {
        let app = tempfile::tempdir().unwrap();
        let legacy = app.path().join("development-evidence");
        private_directory(&legacy).unwrap();
        fs::write(legacy.join("unchanged.json"), b"synthetic saved evidence").unwrap();
        for selector in [None, Some(""), Some("default")] {
            let (_, root, workspace) = initialize_evidence_storage(
                true,
                || Some(app.path().to_owned()),
                || selector.map(Into::into),
            )
            .unwrap()
            .unwrap();
            assert_eq!(root, legacy);
            assert_eq!(workspace.label(), "default");
        }
        let mut roots = Vec::new();
        for index in 0..EXTRA_WORKSPACE_LIMIT {
            let workspace = EvidenceWorkspace::parse(Some(&format!("batch-{index}"))).unwrap();
            roots.push(select_evidence_root(app.path(), &workspace).unwrap());
        }
        let ninth = EvidenceWorkspace::parse(Some("batch-9")).unwrap();
        assert_eq!(
            select_evidence_root(app.path(), &ninth),
            Err("development_evidence_workspace_limit")
        );
        assert!(!workspace_directory(app.path(), &ninth).exists());
        for (index, root) in roots.iter().enumerate() {
            let workspace = EvidenceWorkspace::parse(Some(&format!("batch-{index}"))).unwrap();
            assert_eq!(select_evidence_root(app.path(), &workspace).unwrap(), *root);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    fs::metadata(root).unwrap().permissions().mode() & 0o777,
                    0o700
                );
            }
        }
        assert_eq!(
            select_evidence_root(app.path(), &EvidenceWorkspace::Default).unwrap(),
            legacy
        );
        assert_eq!(
            fs::read(legacy.join("unchanged.json")).unwrap(),
            b"synthetic saved evidence"
        );
        assert_eq!(
            workspace_count(&app.path().join("development-evidence-workspaces")).unwrap(),
            EXTRA_WORKSPACE_LIMIT
        );
        assert_eq!(
            ALL_CASES_BYTE_LIMIT * (EXTRA_WORKSPACE_LIMIT as u64 + 1),
            1152 * 1024 * 1024
        );
    }

    #[test]
    fn evidence_workspace_ignores_only_bounded_regular_finder_metadata() {
        let container = tempfile::tempdir().unwrap();
        let metadata = container.path().join(".DS_Store");
        let original = vec![0x42; 64 * 1024];
        fs::write(&metadata, &original).unwrap();
        for index in 0..EXTRA_WORKSPACE_LIMIT {
            fs::create_dir(container.path().join(format!("batch-{index}"))).unwrap();
        }
        let evidence = container.path().join("batch-0").join("unchanged.json");
        fs::write(&evidence, b"synthetic immutable case").unwrap();
        assert_eq!(
            workspace_count(container.path()).unwrap(),
            EXTRA_WORKSPACE_LIMIT
        );
        assert_eq!(fs::read(&metadata).unwrap(), original);
        assert_eq!(fs::read(&evidence).unwrap(), b"synthetic immutable case");
        fs::create_dir(container.path().join("ninth")).unwrap();
        assert_eq!(
            workspace_count(container.path()),
            Err("development_evidence_workspace_limit")
        );
    }

    #[test]
    fn evidence_workspace_rejects_oversized_metadata_and_other_entries() {
        let container = tempfile::tempdir().unwrap();
        let metadata = container.path().join(".DS_Store");
        private_file(&metadata)
            .unwrap()
            .set_len(64 * 1024 + 1)
            .unwrap();
        assert_eq!(
            workspace_count(container.path()),
            Err("development_evidence_workspace_invalid")
        );
        assert_eq!(fs::metadata(&metadata).unwrap().len(), 64 * 1024 + 1);
        fs::remove_file(&metadata).unwrap();
        fs::create_dir(&metadata).unwrap();
        assert_eq!(
            workspace_count(container.path()),
            Err("development_evidence_workspace_invalid")
        );
        fs::remove_dir(&metadata).unwrap();
        fs::write(
            container.path().join("other-metadata"),
            b"bounded but unrecognized",
        )
        .unwrap();
        assert_eq!(
            workspace_count(container.path()),
            Err("development_evidence_workspace_invalid")
        );
    }

    #[cfg(unix)]
    #[test]
    fn evidence_workspace_rejects_finder_metadata_symlinks() {
        use std::os::unix::fs::symlink;
        let container = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("metadata");
        fs::write(&target, b"unchanged outside fixture").unwrap();
        let link = container.path().join(".DS_Store");
        symlink(&target, &link).unwrap();
        assert_eq!(
            workspace_count(container.path()),
            Err("development_evidence_workspace_invalid")
        );
        assert_eq!(fs::read(&target).unwrap(), b"unchanged outside fixture");
        fs::remove_file(&link).unwrap();
        symlink(outside.path(), &link).unwrap();
        assert_eq!(
            workspace_count(container.path()),
            Err("development_evidence_workspace_invalid")
        );
    }

    #[test]
    fn evidence_workspace_retains_the_same_per_catalog_case_reservation() {
        let app = tempfile::tempdir().unwrap();
        let legacy = select_evidence_root(app.path(), &EvidenceWorkspace::Default).unwrap();
        private_file(&legacy.join("existing.bin"))
            .unwrap()
            .set_len(ALL_CASES_BYTE_LIMIT - CASE_RESERVED_BYTES + 1)
            .unwrap();
        assert!(matches!(
            start_case(legacy.clone(), &serde_json::json!({}), Instant::now()),
            Err("development_evidence_storage_limit")
        ));
        let workspace = EvidenceWorkspace::parse(Some("second-batch")).unwrap();
        let root = select_evidence_root(app.path(), &workspace).unwrap();
        let case = start_case(root.clone(), &serde_json::json!({}), Instant::now()).unwrap();
        case.tx.lock().unwrap().take();
        case.writer.lock().unwrap().take().unwrap().join().unwrap();
        assert!(case.directory.starts_with(&root));
        assert_eq!(
            stored_bytes(&legacy).unwrap(),
            ALL_CASES_BYTE_LIMIT - CASE_RESERVED_BYTES + 1
        );
        for index in 1..CASE_COUNT_LIMIT {
            fs::create_dir(root.join(format!("occupied-{index}"))).unwrap();
        }
        assert!(matches!(
            start_case(root.clone(), &serde_json::json!({}), Instant::now()),
            Err("development_evidence_storage_limit")
        ));
        assert_eq!(fs::read_dir(root).unwrap().count(), CASE_COUNT_LIMIT);
    }

    #[test]
    fn evidence_workspace_rejects_non_directories_and_changed_roots() {
        let app = tempfile::tempdir().unwrap();
        let workspace = EvidenceWorkspace::parse(Some("batch")).unwrap();
        let root = select_evidence_root(app.path(), &workspace).unwrap();
        let debugger = DebuggerState {
            app_directory: Some(app.path().to_owned()),
            root: Some(root.clone()),
            workspace,
            ..Default::default()
        };
        assert_eq!(evidence_root(&debugger).unwrap(), root);
        fs::remove_dir(&root).unwrap();
        fs::write(&root, b"not a directory").unwrap();
        assert_eq!(
            evidence_root(&debugger),
            Err("development_evidence_workspace_invalid")
        );
        assert_eq!(
            select_evidence_root(app.path(), &debugger.workspace),
            Err("development_evidence_workspace_invalid")
        );
        assert_eq!(fs::read(root).unwrap(), b"not a directory");
    }

    #[cfg(unix)]
    #[test]
    fn evidence_workspace_rejects_symlink_roots_without_touching_targets() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        fs::create_dir(&target).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(target.join("unchanged"), b"outside fixture").unwrap();
        let linked_app = root.path().join("linked-app");
        symlink(&target, &linked_app).unwrap();
        assert_eq!(
            select_evidence_root(&linked_app, &EvidenceWorkspace::Default),
            Err("development_evidence_workspace_invalid")
        );
        for (name, workspace) in [
            ("development-evidence", EvidenceWorkspace::Default),
            (
                "development-evidence-workspaces",
                EvidenceWorkspace::parse(Some("batch")).unwrap(),
            ),
            (
                "development-evidence-workspaces/batch",
                EvidenceWorkspace::parse(Some("batch")).unwrap(),
            ),
        ] {
            let app = tempfile::tempdir().unwrap();
            let link = app.path().join(name);
            fs::create_dir_all(link.parent().unwrap()).unwrap();
            symlink(&target, &link).unwrap();
            assert_eq!(
                select_evidence_root(app.path(), &workspace),
                Err("development_evidence_workspace_invalid")
            );
        }
        assert_eq!(
            fs::read(target.join("unchanged")).unwrap(),
            b"outside fixture"
        );
        assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
        assert_eq!(
            fs::metadata(target).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }

    fn metadata_entry(id: u64) -> trace::DebugEntry {
        trace::DebugEntry {
            id,
            elapsed_ms: id,
            event: trace::DebugEvent::Pipeline {
                label: "capture started".into(),
            },
        }
    }
    #[test]
    fn persisted_metadata_exceeds_the_live_ring_and_pages_exports_every_id() {
        let root = tempfile::tempdir().unwrap();
        let case = start_case(
            root.path().to_owned(),
            &serde_json::json!({}),
            Instant::now(),
        )
        .unwrap();
        for id in 1..=(trace::ENTRY_LIMIT + 1) as u64 {
            let mut entry = metadata_entry(id);
            if id == (trace::ENTRY_LIMIT + 1) as u64 {
                entry.event = trace::DebugEvent::Stopped {
                    unflushed_windows: vec![],
                };
            }
            let mut bytes = serde_json::to_vec(&entry).unwrap();
            bytes.push(b'\n');
            case.tx
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .send((ContentKind::Trace, bytes))
                .unwrap();
        }
        seal_case_trace(&case, std::time::Duration::ZERO).unwrap();
        case.writer.lock().unwrap().take().unwrap().join().unwrap();
        assert_eq!(
            case.trace_entries.load(Ordering::Relaxed),
            (trace::ENTRY_LIMIT + 1) as u64
        );
        assert_eq!(case.trace_dropped.load(Ordering::Relaxed), 0);
        let first = read_trace_page(&case.directory, 0, 64).unwrap();
        assert_eq!(first.len(), 64);
        assert_eq!(first[0].id, 1);
        let last = read_trace_page(&case.directory, trace::ENTRY_LIMIT, 64).unwrap();
        assert_eq!(last.len(), 1);
        assert!(matches!(last[0].event, trace::DebugEvent::Stopped { .. }));
        let destination = root.path().join("export.jsonl");
        assert!(copy_case_file(
            &case.directory,
            "trace-events.jsonl",
            &destination,
            CONTENT_BYTE_LIMIT
        )
        .unwrap());
        let stored = fs::read(case.directory.join("trace-events.jsonl")).unwrap();
        assert_eq!(fs::read(destination).unwrap(), stored);
        assert_eq!(
            case.trace_bytes.load(Ordering::Relaxed),
            stored.len() as u64
        );
        assert_eq!(case.bytes.load(Ordering::Relaxed), stored.len() as u64);
        assert!(read_trace_page(&case.directory, 0, 65).is_err());
        assert!(read_trace_page(&case.directory, 0, 0).is_err());
        assert!(!copy_case_file(
            root.path(),
            "missing-old-case-trace.jsonl",
            &root.path().join("unused.jsonl"),
            CONTENT_BYTE_LIMIT
        )
        .unwrap());
    }
    #[test]
    fn durable_metadata_reports_queue_line_and_shared_disk_budget_loss() {
        let root = tempfile::tempdir().unwrap();
        let case = start_case(
            root.path().to_owned(),
            &serde_json::json!({}),
            Instant::now(),
        )
        .unwrap();
        let gate = case.write_gate.lock().unwrap();
        for id in 1..=80 {
            record_trace_event(&case, metadata_entry(id));
        }
        assert!(case.trace_dropped.load(Ordering::Relaxed) >= 47);
        drop(gate);
        seal_case_trace(&case, std::time::Duration::ZERO).unwrap();
        case.writer.lock().unwrap().take().unwrap().join().unwrap();
        assert_eq!(
            case.trace_entries.load(Ordering::Relaxed) + case.trace_dropped.load(Ordering::Relaxed),
            80
        );
        assert_eq!(
            case.trace_dropped.load(Ordering::Relaxed),
            case.dropped.load(Ordering::Relaxed)
        );

        let limited = start_case(
            root.path().to_owned(),
            &serde_json::json!({}),
            Instant::now(),
        )
        .unwrap();
        let mut huge = metadata_entry(1);
        huge.event = trace::DebugEvent::Pipeline {
            label: "x".repeat(512 * 1024),
        };
        record_trace_event(&limited, huge);
        assert_eq!(limited.trace_dropped.load(Ordering::Relaxed), 1);
        limited.bytes.store(CONTENT_BYTE_LIMIT, Ordering::Relaxed);
        record_trace_event(&limited, metadata_entry(2));
        seal_case_trace(&limited, std::time::Duration::ZERO).unwrap();
        limited
            .writer
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .join()
            .unwrap();
        assert!(limited.trace_limited.load(Ordering::Relaxed));
        assert!(limited.limited.load(Ordering::Relaxed));
        assert_eq!(limited.trace_entries.load(Ordering::Relaxed), 0);
        assert_eq!(limited.trace_dropped.load(Ordering::Relaxed), 2);
    }
    #[test]
    fn timeout_keeps_the_case_unsealed_and_retry_drains_the_stopped_entry() {
        let root = tempfile::tempdir().unwrap();
        let case = start_case(
            root.path().to_owned(),
            &serde_json::json!({}),
            Instant::now(),
        )
        .unwrap();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let release_rx = Mutex::new(release_rx);
        let target_case = case.clone();
        let target = trace::TraceSink::new(move |entry| {
            ready_tx.send(()).unwrap();
            release_rx.lock().unwrap().recv().unwrap();
            record_trace_event(&target_case, entry);
        });
        *case.trace_sink.lock().unwrap() = Some(target.clone());
        let worker = std::thread::spawn(move || {
            target.dispatch_for_test(trace::DebugEntry {
                id: 1,
                elapsed_ms: 0,
                event: trace::DebugEvent::Stopped {
                    unflushed_windows: vec![],
                },
            })
        });
        ready_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert_eq!(
            seal_case_trace(&case, std::time::Duration::from_millis(1)),
            Err("development_evidence_trace_busy")
        );
        assert!(case.tx.lock().unwrap().is_some());
        release_tx.send(()).unwrap();
        worker.join().unwrap();
        seal_case_trace(&case, std::time::Duration::from_secs(1)).unwrap();
        case.writer.lock().unwrap().take().unwrap().join().unwrap();
        assert_eq!(case.trace_entries.load(Ordering::Relaxed), 1);
        assert_eq!(case.trace_dropped.load(Ordering::Relaxed), 0);
        assert!(matches!(
            read_trace_page(&case.directory, 0, 64).unwrap()[0].event,
            trace::DebugEvent::Stopped { .. }
        ));
    }
    #[cfg(unix)]
    #[test]
    fn trace_reads_and_export_refuse_external_symlinks_and_nonregular_files() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("private.txt"), b"private marker").unwrap();
        symlink(
            outside.path().join("private.txt"),
            root.path().join("trace-events.jsonl"),
        )
        .unwrap();
        assert!(read_trace_page(root.path(), 0, 64).is_err());
        let destination = root.path().join("export.jsonl");
        assert!(copy_case_file(
            root.path(),
            "trace-events.jsonl",
            &destination,
            CONTENT_BYTE_LIMIT
        )
        .is_err());
        assert!(!destination.exists());
        fs::remove_file(root.path().join("trace-events.jsonl")).unwrap();
        fs::create_dir(root.path().join("trace-events.jsonl")).unwrap();
        assert!(read_trace_page(root.path(), 0, 64).is_err());
    }

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
            .send((
                ContentKind::Snapshot,
                b"{\"snapshot\":{\"synthetic\":true}}\n".to_vec(),
            ))
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
            trace_persistence_enabled: true,
            persisted_trace_entries: 0,
            trace_bytes: 0,
            trace_dropped: 0,
            trace_limited: false,
            trace_failed: false,
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
        assert_eq!(reopened.route["evidenceWorkspace"], "default");
        let mut legacy = serde_json::to_value(&report).unwrap();
        for name in [
            "tracePersistenceEnabled",
            "persistedTraceEntries",
            "traceBytes",
            "traceDropped",
            "traceLimited",
            "traceFailed",
        ] {
            legacy.as_object_mut().unwrap().remove(name);
        }
        let legacy: DebuggerSnapshot = serde_json::from_value(legacy).unwrap();
        assert!(!legacy.trace_persistence_enabled);
        assert_eq!(legacy.persisted_trace_entries, 0);
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
