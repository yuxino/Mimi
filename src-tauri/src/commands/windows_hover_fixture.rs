//! One bounded, content-free snapshot of each overlay WebView for Windows UI smoke.

use std::{
    ffi::OsStr,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, WebviewWindow};

use super::AppState;

const OPT_IN: &str = "windows-hover-sample-enabled";
const REQUEST: &str = "windows-hover-sample-request";
const MAX_SAMPLE_BYTES: usize = 2048;
const SMOKE_LIFETIME: Duration = Duration::from_secs(125);
static STARTED: AtomicBool = AtomicBool::new(false);

// Run the same fixed expression in both known windows. It reads presentation
// state only; it does not install hooks, query text or dispatch input.
const SNAPSHOT: &str = r#"(() => {
    const overlay = document.querySelector('.overlay-surface');
    const control = document.querySelector('.overlay-control-detached');
    const surface = overlay || control;
    if (!surface) return null;
    const has = name => surface.hasAttribute(name);
    const panel = control?.querySelector('.overlay-control-panel');
    const island = control?.querySelector('.overlay-control-island:not(.overlay-control-island-measure *)');
    const mode = overlay
      ? (overlay.querySelector('.overlay-compact-canvas') ? 'collapsed' : 'expanded')
      : (panel ? 'panel' : island ? 'island' : 'hidden');
    const sample = overlay ? overlay.querySelector('.overlay-action-strip')
      : (panel ? panel.querySelector('.overlay-control-header') : island);
    return {
        ownHover: has('data-overlay-hover'),
        peerHover: has('data-overlay-peer-hover'),
        cssHover: surface.matches(':hover'),
        pointerTracked: has('data-overlay-pointer-tracked'),
        focusVisible: surface.querySelector(':focus-visible') !== null,
        documentFocus: document.hasFocus(),
        chromePinned: has('data-overlay-chrome-pinned'),
        islandPinned: has('data-overlay-island-pinned'),
        quietIsland: has('data-quiet-island'),
        mode,
        innerWidth,
        innerHeight,
        devicePixelRatio,
        sampleOpacityPercent: sample ? Math.round(Number(getComputedStyle(sample).opacity) * 100) : null,
    };
})()"#;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Mode {
    Expanded,
    Collapsed,
    Island,
    Panel,
    Hidden,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HoverSnapshot {
    own_hover: bool,
    peer_hover: bool,
    css_hover: bool,
    pointer_tracked: bool,
    focus_visible: bool,
    document_focus: bool,
    chrome_pinned: bool,
    island_pinned: bool,
    quiet_island: bool,
    mode: Mode,
    inner_width: u16,
    inner_height: u16,
    device_pixel_ratio: f64,
    sample_opacity_percent: Option<u8>,
}

fn checked_directory(configured: &Path, temp: &Path) -> Option<PathBuf> {
    let name = configured.file_name()?.to_str()?;
    let suffix = name.strip_prefix("mimi-frontend-")?;
    if suffix.len() != 32 || !suffix.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let directory = configured.canonicalize().ok()?;
    let temp = temp.canonicalize().ok()?;
    (directory.is_dir()
        && directory.parent() == Some(temp.as_path())
        && directory.file_name() == configured.file_name())
    .then_some(directory)
}

fn marker_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|marker| marker.is_file() && marker.len() == 0)
}

pub(super) fn record_drag_dispatch(ui_test: bool, stage: u8) {
    if !ui_test {
        return;
    }
    let Some(directory) = std::env::var_os("MIMI_UI_TEST_FRONTEND_READY_DIR")
        .and_then(|path| checked_directory(Path::new(&path), &std::env::temp_dir()))
    else {
        return;
    };
    let marker = directory.join("windows-caption-drag-dispatch");
    if !fs::symlink_metadata(&marker).is_ok_and(|file| file.is_file() && file.len() < 32) {
        return;
    }
    if let Ok(mut file) = OpenOptions::new().append(true).open(marker) {
        let _ = file.write_all(&[stage]);
    }
}

// This temporary receiver runs only for the opted-in caption-drag process.
// Compare signed screen coordinates without keeping them or any window payload.
unsafe extern "system" fn observe_drag_message(
    hwnd: windows_sys::Win32::Foundation::HWND,
    message: u32,
    wparam: usize,
    lparam: isize,
    id: usize,
    _: usize,
) -> isize {
    use windows_sys::Win32::UI::{Shell, WindowsAndMessaging as wm};
    match message {
        wm::WM_NCLBUTTONDOWN if wparam == wm::HTCAPTION as usize => {
            let mut point = windows_sys::Win32::Foundation::POINT::default();
            let matches = unsafe { wm::GetCursorPos(&mut point) } != 0
                && i32::from(lparam as i16) == point.x
                && i32::from((lparam >> 16) as i16) == point.y;
            record_drag_dispatch(true, if matches { b'C' } else { b'X' });
        }
        wm::WM_ENTERSIZEMOVE => record_drag_dispatch(true, b'I'),
        wm::WM_EXITSIZEMOVE => record_drag_dispatch(true, b'O'),
        wm::WM_NCDESTROY => unsafe {
            Shell::RemoveWindowSubclass(hwnd, Some(observe_drag_message), id);
        },
        _ => {}
    }
    unsafe { Shell::DefSubclassProc(hwnd, message, wparam, lparam) }
}

pub(super) fn observe_ready(window: &WebviewWindow, directory: Option<&OsStr>) {
    if window.label() != "overlay" {
        return;
    }
    let app = window.app_handle();
    if !app
        .try_state::<AppState>()
        .is_some_and(|state| state.settings.is_ui_test())
    {
        return;
    }
    let Some(directory) =
        directory.and_then(|path| checked_directory(Path::new(path), &std::env::temp_dir()))
    else {
        return;
    };
    if marker_exists(&directory.join("windows-caption-drag-dispatch")) {
        let root = window.clone();
        let _ = window.run_on_main_thread(move || {
            if let Ok(hwnd) = root.hwnd() {
                let installed = unsafe {
                    windows_sys::Win32::UI::Shell::SetWindowSubclass(
                        hwnd.0,
                        Some(observe_drag_message),
                        0x4d44,
                        0,
                    )
                };
                record_drag_dispatch(true, if installed != 0 { b'S' } else { b'U' });
            }
        });
    }
    let opt_in = directory.join(OPT_IN);
    if !marker_exists(&opt_in) || STARTED.swap(true, Ordering::Relaxed) {
        return;
    }

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let request = directory.join(REQUEST);
        let deadline = Instant::now() + SMOKE_LIFETIME;
        while Instant::now() < deadline && marker_exists(&opt_in) {
            if marker_exists(&request) {
                capture(&app, &directory);
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    });
}

fn capture(app: &AppHandle, directory: &Path) {
    for (label, marker) in [
        ("overlay", "windows-hover-sample-overlay.json"),
        (
            "overlay-control",
            "windows-hover-sample-overlay-control.json",
        ),
    ] {
        if let Some(window) = app.get_webview_window(label) {
            let destination = directory.join(marker);
            let _ = window.eval_with_callback(SNAPSHOT, move |json| {
                let _ = write_snapshot(&destination, &json);
            });
        }
    }
}

fn write_snapshot(destination: &Path, json: &str) -> io::Result<()> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "invalid overlay hover sample");
    if json.len() > MAX_SAMPLE_BYTES {
        return Err(invalid());
    }
    let sample: HoverSnapshot = serde_json::from_str(json).map_err(|_| invalid())?;
    if !(0.25..=16.0).contains(&sample.device_pixel_ratio)
        || sample
            .sample_opacity_percent
            .is_some_and(|opacity| opacity > 100)
    {
        return Err(invalid());
    }
    let serialized = serde_json::to_vec(&sample)?;
    // Publish only after the complete validated sample is available.
    let temporary = destination.with_extension("tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result = file.write_all(&serialized);
    drop(file);
    let result = result.and_then(|()| fs::rename(&temporary, destination));
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
