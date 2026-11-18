//! Settings-window creation, restoration, focus and UI-test activation evidence.

use crate::pipeline_log;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use super::dev_title;

/// Shows the settings window, creating it first if it does not exist (the
/// window is hidden on close — see the CloseRequested handler in lib.rs — but
/// a stale or crashed webview could leave it missing). Mirrors the window
/// declared in tauri.conf.json.
pub fn ensure_settings_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window("settings") else {
        let builder =
            WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("index.html".into()))
                .title(dev_title("Mimi"))
                .inner_size(760.0, 720.0)
                .min_inner_size(520.0, 560.0)
                .resizable(true)
                .center()
                .visible(true);
        if builder.build().is_err() {
            pipeline_log!("settings window re-create failed label=create_failed");
            return;
        }
        let Some(window) = app.get_webview_window("settings") else {
            return;
        };
        restore_and_focus_settings_window(&window);
        return;
    };
    restore_and_focus_settings_window(&window);
}

fn restore_and_focus_settings_window(window: &tauri::WebviewWindow) {
    let _ = window.unminimize();
    let _ = window.show();
    let focus_requested = window.set_focus().is_ok();
    record_ui_test_settings_activation(focus_requested);
}

fn record_ui_test_settings_activation(focus_requested: bool) {
    if std::env::var("MIMI_UI_TEST").as_deref() != Ok("1") {
        return;
    }
    let Some(path) = std::env::var_os("MIMI_UI_TEST_SETTINGS_ACTIVATION_FILE") else {
        return;
    };
    let state: &[u8] = if focus_requested {
        b"focus-requested"
    } else {
        b"focus-error"
    };
    if let Err(error) = std::fs::write(path, state) {
        tracing::warn!(error = %error, "could not write UI-test settings activation marker");
    }
}
