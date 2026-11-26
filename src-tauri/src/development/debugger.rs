//! Development-only commands. Content evidence is private and explicitly enabled,
//! separate from the content-free diagnostic journal.

#[path = "debugger/case_io.rs"]
mod case_io;
#[path = "debugger/workspace.rs"]
mod workspace;

use crate::commands::AppState;
use crate::core::audio::audio_input::AudioSource;
use crate::core::development::development_debug::{self as trace, FrontendObservation};
use crate::core::development::development_evidence_workspace::EvidenceWorkspace;
#[cfg(test)]
#[allow(unused_imports)]
use crate::core::development::development_evidence_workspace::EXTRA_WORKSPACE_LIMIT;
use crate::session_manager::SessionStateEvent;
#[cfg(test)]
#[allow(unused_imports)]
use case_io::{case_audio_path, record_trace_event, ALL_CASES_BYTE_LIMIT, CASE_RESERVED_BYTES};
use case_io::{
    copy_case_audio, persist_case_report, read_case_audio, read_saved_report, read_trace_page,
    record_private_event, require_finalized, saved_case_directory, seal_case_trace, start_case,
    validate_case_id, ContentKind, EvidenceCase, AUDIO_FILE_BYTE_LIMIT, CASE_COUNT_LIMIT,
    CONTENT_BYTE_LIMIT,
};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[cfg(test)]
#[allow(unused_imports)]
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use workspace::{
    copy_case_file, evidence_root, initialize_evidence_storage, private_directory, private_file,
};
#[cfg(test)]
#[allow(unused_imports)]
use workspace::{select_evidence_root, stored_bytes, workspace_count, workspace_directory};

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

fn available() -> Result<(), &'static str> {
    if trace::snapshot(0).available {
        Ok(())
    } else {
        Err("development_debug_unavailable")
    }
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
            "audioInput":preferences.audio_input,"sourceLanguage":preferences.source_language,"targetLanguage":preferences.target_language,"subtitleDisplayMode":preferences.subtitle_display_mode,"showIntermediateSubtitles":preferences.show_intermediate_subtitles,"showSubtitleTimestamps":preferences.show_subtitle_timestamps,"fontSize":preferences.font_size,
            "subtitleAlignment":preferences.subtitle_alignment,"subtitleColor":preferences.subtitle_color,"microphoneSubtitleColor":preferences.microphone_subtitle_color,"blendsWithBackground":preferences.subtitle_blends_with_background,
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedCase {
    id: String,
    created_at_unix_ms: u64,
    snapshots: u64,
    provider: serde_json::Value,
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
        "audioInput":preferences.audio_input,"subtitleDisplayMode":preferences.subtitle_display_mode,"showIntermediateSubtitles":preferences.show_intermediate_subtitles,"showSubtitleTimestamps":preferences.show_subtitle_timestamps,"fontSize":preferences.font_size,
        "blendsWithBackground":preferences.subtitle_blends_with_background,"microphoneSubtitleColor":preferences.microphone_subtitle_color,"recordedContent":with_audio,
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
                "../../../docs/development/debugger-evidence.md"
            ))
            .map_err(|_| "development_export_failed")?;
        Ok(())
    })
    .await
    .map_err(|_| "development_export_failed")??;
    Ok(true)
}

#[cfg(test)]
#[path = "debugger/tests.rs"]
mod tests;
