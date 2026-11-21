// Temporary observer for the disposable, signed macOS UI fixture only.
use serde_json::{json, Value};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager, WebviewWindow};

static WRITE: Mutex<()> = Mutex::new(());

fn destination(app: &AppHandle) -> Option<PathBuf> {
    if app.config().identifier != "app.yuxino.mimi.dev"
        || std::env::var("RUNNER_ENVIRONMENT").as_deref() != Ok("github-hosted")
        || std::env::var("MIMI_UI_TEST").as_deref() != Ok("1")
        || std::env::var("MIMI_UI_TEST_LANGUAGE_PROBE").as_deref() != Ok("1")
    {
        return None;
    }
    let scene = PathBuf::from(std::env::var_os("MIMI_UI_TEST_PREFERENCES_DIR")?)
        .canonicalize()
        .ok()?;
    let temp = std::env::temp_dir().canonicalize().ok()?;
    if scene.parent()? != temp
        || !scene
            .file_name()?
            .to_str()?
            .starts_with("mimi-ui-test-hover.")
    {
        return None;
    }
    Some(scene.join("language-surface.jsonl"))
}

pub(crate) fn enabled(app: &AppHandle) -> bool {
    destination(app).is_some()
}

pub(crate) fn native_start(app: &AppHandle) {
    let _ = write_record(app, json!({"kind": "native-creating-child"}));
}

pub(crate) fn record(window: &WebviewWindow, sample: Value) -> Result<(), String> {
    if window.label() != "overlay-control" || !sample.is_object() {
        return Err("unexpected child observer".into());
    }
    write_record(window.app_handle(), sample)
}

fn write_record(app: &AppHandle, sample: Value) -> Result<(), String> {
    let file = destination(app).ok_or("isolated language observer disabled")?;
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "clock")?
        .as_millis() as u64;
    let mode = crate::windows::OverlayControlWindowManager::mode(app);
    let mut line = serde_json::to_vec(&json!({
        "nativeMode": mode, "receivedMs": now_ms, "child": sample
    }))
    .map_err(|_| "observer json")?;
    if line.len() > 16384 {
        return Err("observer record too large".into());
    }
    line.push(b'\n');
    let _lock = WRITE.lock().map_err(|_| "observer lock")?;
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut output = options.open(file).map_err(|_| "observer file")?;
    if output.metadata().map_err(|_| "observer metadata")?.len() >= 768 * 1024 {
        return Err("observer limit".into());
    }
    output
        .write_all(&line)
        .map_err(|_| "observer append".into())
}
