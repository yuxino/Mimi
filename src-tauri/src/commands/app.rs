//! Application navigation, startup identity and coordinated normal exit.

#[cfg(target_os = "windows")]
use super::windows_hover_fixture;
use super::AppState;
use crate::session_manager::SessionManager;
use crate::windows::OverlayControlWindowManager;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Listener, Manager, State};

const SETTINGS_NAVIGATION_EVENT: &str = "settings-navigate";

const SETTINGS_NAVIGATION_READY_EVENT: &str = "settings-navigation-ready";

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SettingsNavigationTarget {
    Service,
    Voice,
    Export,
    AppleSpeechResources,
    ActiveProfile,
}

/// Lets the frontend select a deterministic updater adapter during native UI
/// tests. The production updater never receives this flag or fixture data.
#[tauri::command]
pub fn app_is_ui_test() -> bool {
    std::env::var("MIMI_UI_TEST").as_deref() == Ok("1")
}

/// A native smoke test must observe a mounted frontend, not just a window title.
/// Neither the destination nor marker contents can be supplied by the frontend.
#[tauri::command]
pub fn app_ui_test_frontend_ready(window: tauri::WebviewWindow) -> Result<(), String> {
    let is_ui_test = app_is_ui_test();
    let directory = std::env::var_os("MIMI_UI_TEST_FRONTEND_READY_DIR");
    write_ui_test_frontend_ready(is_ui_test, window.label(), directory.as_deref())
        .map_err(|_| "Could not write the UI-test readiness marker.".to_string())?;
    #[cfg(target_os = "windows")]
    if is_ui_test {
        windows_hover_fixture::observe_ready(&window, directory.as_deref());
    }
    Ok(())
}

fn write_ui_test_frontend_ready(
    is_ui_test: bool,
    label: &str,
    directory: Option<&std::ffi::OsStr>,
) -> std::io::Result<()> {
    if !is_ui_test || !matches!(label, "settings" | "overlay") {
        return Ok(());
    }
    if let Some(directory) = directory {
        let directory = std::path::Path::new(directory);
        std::fs::create_dir_all(directory)?;
        std::fs::write(directory.join(label), b"ready\n")?;
    }
    Ok(())
}

/// The Windows ZIP carries this marker beside the executable. It identifies
/// an extract-and-run copy before Settings can offer an NSIS update.
#[tauri::command]
pub fn app_is_portable() -> bool {
    #[cfg(target_os = "windows")]
    {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join("mimi.portable")))
            .is_some_and(|marker| marker.is_file())
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

/// Tauri can replace an AppImage, but cannot update a package-manager install.
#[tauri::command]
pub fn app_is_linux_package() -> bool {
    cfg!(target_os = "linux") && std::env::var_os("APPIMAGE").is_none_or(|path| path.is_empty())
}

fn emit_settings_navigation(app: &AppHandle, target: SettingsNavigationTarget) {
    if let Some(window) = app.get_webview_window("settings") {
        let _ = window.emit(SETTINGS_NAVIGATION_EVENT, target);
    }
}

#[tauri::command]
pub fn app_show_settings(
    app: AppHandle,
    target: Option<SettingsNavigationTarget>,
) -> Result<(), String> {
    // Collapse the always-on-top overlay control panel so it cannot obscure Settings.
    OverlayControlWindowManager::dismiss_panel(&app);

    // The settings window normally exists for the whole app lifetime and is
    // merely hidden on close. If its webview crashed, however, recreating it
    // and emitting immediately would race the frontend listener. In that
    // recovery path, wait for SettingsView to announce that its navigation
    // listener is installed before delivering the one-shot intent.
    let settings_window_exists = app.get_webview_window("settings").is_some();
    if !settings_window_exists {
        if let Some(target) = target {
            let ready_app = app.clone();
            app.once(SETTINGS_NAVIGATION_READY_EVENT, move |_| {
                emit_settings_navigation(&ready_app, target);
            });
        }
    }

    crate::windows::ensure_settings_window(&app);
    if settings_window_exists {
        if let Some(target) = target {
            emit_settings_navigation(&app, target);
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn app_quit(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    quit_application(app, Arc::clone(&state.session)).await
}

/// Every normal exit uses the same stop/finalize boundary. A failed save keeps
/// the pending archive and app alive so the user can retry rather than lose it.
pub async fn quit_application(app: AppHandle, session: Arc<SessionManager>) -> Result<(), String> {
    let Some(request) = NORMAL_QUIT_GATE.begin() else {
        return Ok(());
    };
    session.stop().await;
    #[cfg(any(test, feature = "development-debugger"))]
    crate::development_debugger::finish_on_quit(app.clone()).await?;
    finish_quit(session.persist_current_history(), || app.exit(0))?;
    request.exit_requested();
    Ok(())
}

static NORMAL_QUIT_GATE: QuitRequestGate = QuitRequestGate(AtomicBool::new(false));

/// Share one stop/save across simultaneous Dock, menu, tray and IPC requests.
/// Failed or cancelled futures release the gate; successful exit keeps it shut.
struct QuitRequestGate(AtomicBool);

impl QuitRequestGate {
    fn begin(&self) -> Option<QuitRequest<'_>> {
        self.0
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| QuitRequest {
                gate: self,
                exiting: false,
            })
    }
}

struct QuitRequest<'a> {
    gate: &'a QuitRequestGate,
    exiting: bool,
}

impl QuitRequest<'_> {
    fn exit_requested(mut self) {
        self.exiting = true;
    }
}

impl Drop for QuitRequest<'_> {
    fn drop(&mut self) {
        if !self.exiting {
            self.gate.0.store(false, Ordering::Release);
        }
    }
}

fn finish_quit(finalization: std::io::Result<()>, exit: impl FnOnce()) -> Result<(), String> {
    finalization.map_err(|_| "Could not save session history before quitting.".to_string())?;
    exit();
    Ok(())
}

#[cfg(test)]
mod tests;
