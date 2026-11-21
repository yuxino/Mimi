//! Disposable hosted Windows synthetic-history observer. No content or general eval.
use super::AppState;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{Manager, WebviewWindow};

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum Stage {
    Hello,
    Ready,
    Method,
    Capture,
    React,
    Limit,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum Method {
    PointerDown,
    PointerMove,
    PointerUp,
    UserIntent,
    EndUserIntent,
    BeginReading,
    ReadFromStart,
    FollowTail,
    FollowIfAtTail,
    Scrolled,
    DisplayChanged,
    ContentChanged,
    Reflow,
    SyncScrollPosition,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Edge {
    Enter,
    Exit,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum InputEvent {
    Pointerdown,
    Pointermove,
    Pointerup,
    Pointercancel,
    Mousedown,
    Mousemove,
    Mouseup,
    Scroll,
    Blur,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Pointer {
    Mouse,
    Pen,
    Touch,
    Other,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Intent {
    Down,
    Pointer,
    Scrollbar,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct HistoryProbeSample {
    stage: Stage,
    method: Option<Method>,
    edge: Option<Edge>,
    event: Option<InputEvent>,
    following: Option<bool>,
    user_kind: Option<Intent>,
    intent_top: Option<f64>,
    intent_range: Option<f64>,
    pointer_type: Option<Pointer>,
    target_timeline: Option<bool>,
    gutter: Option<bool>,
    trusted: Option<bool>,
    x: Option<f64>,
    y: Option<f64>,
    button: Option<i8>,
    buttons: Option<u8>,
    primary: Option<bool>,
    scroll_top: Option<f64>,
    client_height: Option<f64>,
    scroll_height: Option<f64>,
    offset_width: Option<f64>,
    client_width: Option<f64>,
    rect: Option<[f64; 4]>,
    footer: bool,
    sequence: u16,
    perf_ms: f64,
    wall_ms: f64,
}
fn directory(window: &WebviewWindow) -> Option<PathBuf> {
    if !cfg!(target_os = "windows")
        || window.label() != "overlay"
        || std::env::var("RUNNER_ENVIRONMENT").as_deref() != Ok("github-hosted")
        || std::env::var("MIMI_UI_TEST").as_deref() != Ok("1")
        || std::env::var("MIMI_UI_TEST_CAPTION_HISTORY").as_deref() != Ok("1")
        || !window
            .app_handle()
            .try_state::<AppState>()
            .is_some_and(|s| s.settings.is_ui_test())
    {
        return None;
    }
    let temp = std::env::temp_dir().canonicalize().ok()?;
    for (variable, prefix) in [
        ("MIMI_UI_TEST_FRONTEND_READY_DIR", "mimi-frontend-"),
        ("MIMI_UI_TEST_PREFERENCES_DIR", "mimi-ui-test-"),
    ] {
        let configured = PathBuf::from(std::env::var_os(variable)?);
        let suffix = configured.file_name()?.to_str()?.strip_prefix(prefix)?;
        if suffix.len() != 32 || !suffix.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let resolved = configured.canonicalize().ok()?;
        if resolved.parent()? != temp || resolved.file_name() != configured.file_name() {
            return None;
        }
    }
    let path = PathBuf::from(std::env::var_os("MIMI_UI_TEST_FRONTEND_READY_DIR")?)
        .canonicalize()
        .ok()?;
    let opt = fs::symlink_metadata(path.join("history-pointer-probe-enabled")).ok()?;
    (opt.is_file() && opt.len() == 0).then_some(path)
}
pub(crate) fn record(window: &WebviewWindow, sample: HistoryProbeSample) -> Result<(), String> {
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    static WRITE: Mutex<()> = Mutex::new(());
    let directory = directory(window).ok_or("history probe disabled")?;
    if sample.sequence == 0
        || sample.sequence > 2048
        || !sample.perf_ms.is_finite()
        || sample.perf_ms < 0.0
        || !sample.wall_ms.is_finite()
        || !(0.0..1e14).contains(&sample.wall_ms)
        || [
            sample.intent_top,
            sample.intent_range,
            sample.x,
            sample.y,
            sample.scroll_top,
            sample.client_height,
            sample.scroll_height,
            sample.offset_width,
            sample.client_width,
        ]
        .into_iter()
        .flatten()
        .chain(sample.rect.into_iter().flatten())
        .any(|n| !n.is_finite() || n.abs() > 1e7)
    {
        return Err("invalid history probe number".into());
    }
    let ready = matches!(sample.stage, Stage::Ready);
    let received_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "clock")?
        .as_millis();
    let mut bytes =
        serde_json::to_vec(&serde_json::json!({"receivedMs": received_ms, "sample": sample}))
            .map_err(|_| "json")?;
    if bytes.len() > 2048 || COUNT.fetch_add(1, Ordering::Relaxed) >= 2048 {
        return Err("history probe limit".into());
    }
    bytes.push(b'\n');
    let _lock = WRITE.lock().map_err(|_| "lock")?;
    let mut output = OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("history-pointer-probe.jsonl"))
        .map_err(|_| "file")?;
    if output.metadata().map_err(|_| "metadata")?.len() > 2 * 1024 * 1024 {
        return Err("history probe file limit".into());
    }
    output.write_all(&bytes).map_err(|_| "write")?;
    if ready {
        fs::write(directory.join("history-pointer-probe-ready"), b"ready\n")
            .map_err(|_| "ready")?;
    }
    Ok(())
}
