use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

const MAX_SNAPSHOTS: u8 = 4;
const MAX_EVENTS: usize = 32;
const MAX_SNAPSHOT_BYTES: usize = 8 * 1024;
#[cfg(target_os = "windows")]
pub(super) const INITIALIZATION_SCRIPT: &str = include_str!("overlay_pointer_fixture.js");

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum EventKind {
    Blur,
    PointerOver,
    PointerEnter,
    PointerLeave,
    PointerMove,
    PointerDown,
    PointerUp,
    MouseOver,
    MouseEnter,
    MouseLeave,
    MouseMove,
    MouseDown,
    MouseUp,
    Click,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
enum HitTarget {
    Pause,
    Resume,
    Settings,
    ErrorSettings,
    Drag,
    OtherButton,
    Surface,
    Outside,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PointerEvent {
    kind: EventKind,
    target: HitTarget,
    local: Option<[i32; 2]>,
    screen_css: Option<[i32; 2]>,
    trusted: bool,
    marked_hover: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ButtonSample {
    bounds: [i32; 4],
    hit: HitTarget,
    opacity_percent: u8,
    visible: bool,
    pointer_events: bool,
    disabled: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PointerSnapshot {
    marked_hover: bool,
    css_hover: bool,
    hover_capable: bool,
    focused: bool,
    device_pixel_ratio: f64,
    target: Option<ButtonSample>,
    events: Vec<PointerEvent>,
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

fn write_snapshot(directory: &Path, id: u8, json: &str) -> io::Result<()> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "invalid overlay pointer sample");
    if !(1..=MAX_SNAPSHOTS).contains(&id) || json.len() > MAX_SNAPSHOT_BYTES {
        return Err(invalid());
    }
    let snapshot: PointerSnapshot = serde_json::from_str(json).map_err(|_| invalid())?;
    if snapshot.events.len() > MAX_EVENTS || !snapshot.device_pixel_ratio.is_finite() {
        return Err(invalid());
    }
    let serialized = serde_json::to_vec(&snapshot).map_err(|_| invalid())?;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join(format!("overlay-pointer-{id}.json")))?
        .write_all(&serialized)
}

#[cfg(target_os = "windows")]
pub(super) fn trace_directory(app: &tauri::AppHandle) -> Option<PathBuf> {
    use tauri::Manager;

    if !app
        .try_state::<crate::commands::AppState>()?
        .settings
        .is_ui_test()
        || std::env::var("MIMI_UI_TEST_OVERLAY_POINTER_TRACE").as_deref() != Ok("1")
    {
        return None;
    }
    let configured = PathBuf::from(std::env::var_os("MIMI_UI_TEST_FRONTEND_READY_DIR")?);
    checked_directory(&configured, &std::env::temp_dir())
}

#[cfg(target_os = "windows")]
pub(super) fn observe(window: &tauri::WebviewWindow, directory: PathBuf) {
    use std::{
        io::Read,
        time::{Duration, Instant},
    };

    if window.label() != "overlay" {
        return;
    }
    let window = window.clone();
    tauri::async_runtime::spawn(async move {
        let request = directory.join("overlay-pointer-request");
        let ready = directory.join("overlay");
        let mut last_id = 0;
        let deadline = Instant::now() + Duration::from_secs(125);
        while Instant::now() < deadline && last_id < MAX_SNAPSHOTS {
            if ready.is_file() {
                let id = fs::File::open(&request).ok().and_then(|file| {
                    let mut contents = String::new();
                    file.take(3).read_to_string(&mut contents).ok()?;
                    contents.trim().parse::<u8>().ok()
                });
                if let Some(id) = id.filter(|id| *id > last_id && *id <= MAX_SNAPSHOTS) {
                    let destination = directory.clone();
                    last_id = id;
                    let _ = window.eval_with_callback(
                        "window.__mimiUiTestOverlayPointer?.() ?? null",
                        move |json| {
                            let _ = write_snapshot(&destination, id, &json);
                        },
                    );
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_the_existing_test_directory_and_validates_the_fixed_sample() {
        let temp = std::env::temp_dir();
        let name = format!("mimi-frontend-{}", uuid::Uuid::new_v4().simple());
        let directory = temp.join(name);
        assert!(checked_directory(&directory, &temp).is_none());
        fs::create_dir(&directory).unwrap();
        let canonical = checked_directory(&directory, &temp).unwrap();
        assert!(checked_directory(&directory, &directory).is_none());
        assert!(checked_directory(&temp, &temp).is_none());

        let sample = serde_json::json!({
            "marked_hover": false,
            "css_hover": false,
            "hover_capable": true,
            "focused": false,
            "device_pixel_ratio": 1.25,
            "target": null,
            "events": [],
        });
        assert!(write_snapshot(&canonical, 0, &sample.to_string()).is_err());
        assert!(write_snapshot(&canonical, 1, "null").is_err());
        let mut unexpected = sample.clone();
        unexpected["text"] = serde_json::json!("canary");
        assert!(write_snapshot(&canonical, 1, &unexpected.to_string()).is_err());
        write_snapshot(&canonical, 1, &sample.to_string()).unwrap();
        assert!(write_snapshot(&canonical, 1, &sample.to_string()).is_err());
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(
                &fs::read(canonical.join("overlay-pointer-1.json")).unwrap()
            )
            .unwrap(),
            sample
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
