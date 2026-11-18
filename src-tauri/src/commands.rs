//! Desktop IPC entry point and shared application state.
//!
//! Handlers live with their product concern. `lib.rs` registers their explicit
//! module paths; payload names used by native publishers remain stable here.

pub(crate) mod app;
pub(crate) mod apple_resources;
pub(crate) mod connection;
pub(crate) mod help;
pub(crate) mod overlay;
pub(crate) mod profiles;
pub(crate) mod session;
pub(crate) mod settings;
#[cfg(test)]
mod tests;
#[cfg(target_os = "windows")]
mod windows_hover_fixture;

pub use app::SettingsNavigationTarget;
pub use settings::SettingsSnapshotPayload;

use crate::session_manager::SessionManager;
use crate::settings_store::SettingsStore;
use crate::windows::OverlayWindowManager;
use std::sync::Arc;
use tauri::{AppHandle, Manager};

pub struct AppState {
    pub settings: Arc<SettingsStore>,
    pub session: Arc<SessionManager>,
    /// Single source of truth for the overlay window geometry.
    pub overlay: Arc<std::sync::Mutex<crate::windows::OverlayState>>,
}

pub(crate) fn sync_overlay_minimum(app: &AppHandle) {
    if let Some(state) = app.try_state::<AppState>() {
        OverlayWindowManager::sync_minimum_height(app, &state.overlay, &state.settings);
    }
}
