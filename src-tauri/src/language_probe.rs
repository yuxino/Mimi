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
    #[cfg(target_os = "macos")]
    if sample.get("kind").and_then(Value::as_str) == Some("react")
        && sample.get("stage").and_then(Value::as_str) == Some("native-listener-ready")
    {
        native(
            window.app_handle(),
            "frontend-native-listener-ready",
            "overlay-control",
            json!({}),
        );
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

/// Read-only AppKit metadata for the two owned synthetic surfaces. No windows
/// are activated, reconfigured, hit-tested by DOM, or given synthetic events.
#[cfg(target_os = "macos")]
pub(crate) fn native(app: &AppHandle, stage: &'static str, surface: &'static str, details: Value) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSEvent, NSWindow};
    use objc2_foundation::NSProcessInfo;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    static LAST_MOVE_US: AtomicU64 = AtomicU64::new(0);
    if !enabled(app) {
        return;
    }
    let app = app.clone();
    let dispatcher = app.clone();
    let observe = move || {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let uptime = NSProcessInfo::processInfo().systemUptime();
        if stage == "control-moved" {
            let us = (uptime * 1_000_000.0) as u64;
            let last = LAST_MOVE_US.load(Ordering::Relaxed);
            if us.saturating_sub(last) < 100_000 {
                return;
            }
            LAST_MOVE_US.store(us, Ordering::Relaxed);
        }
        if COUNT.fetch_add(1, Ordering::Relaxed) >= 512 {
            return;
        }
        let root = app.get_webview_window("overlay");
        let control = app.get_webview_window("overlay-control");
        // The WebviewWindow handles above keep these main-thread pointers live.
        let root_native = root
            .as_ref()
            .and_then(|w| w.ns_window().ok())
            .map(|p| unsafe { &*p.cast::<NSWindow>() });
        let control_native = control
            .as_ref()
            .and_then(|w| w.ns_window().ok())
            .map(|p| unsafe { &*p.cast::<NSWindow>() });
        let current = if surface == "overlay-control" {
            control_native
        } else {
            root_native
        };
        let point = NSEvent::mouseLocation();
        let front = NSWindow::windowNumberAtPoint_belowWindowWithWindowNumber(point, 0, mtm);
        let root_number = root_native.map(|w| w.windowNumber());
        let control_number = control_native.map(|w| w.windowNumber());
        let _ = write_record(
            &app,
            json!({
                "kind": "native-pointer-probe", "stage": stage, "surface": surface,
                "systemUptime": uptime, "appKitPoint": [point.x, point.y],
                "rootWindowNumber": root_number, "controlWindowNumber": control_number,
                "frontMatchesRoot": root_number == Some(front),
                "frontMatchesControl": control_number == Some(front),
                "visible": current.map(|w| w.isVisible()),
                "ignoresMouseEvents": current.map(|w| w.ignoresMouseEvents()),
                "parentIsRoot": control_native.and_then(|w| w.parentWindow())
                    .is_some_and(|w| Some(w.windowNumber()) == root_number),
                "details": details
            }),
        );
    };
    if MainThreadMarker::new().is_some() {
        observe();
    } else {
        let _ = dispatcher.run_on_main_thread(observe);
    }
}
