//! Overlay presentation and window-gesture IPC, with native scope checks.

use super::settings::emit_settings_snapshot;
use super::{sync_overlay_minimum, AppState};
use crate::settings_store::SettingsStore;
#[cfg(any(not(target_os = "macos"), test))]
use crate::windows::panel_drag_target;
use crate::windows::{OverlayControlMode, OverlayControlWindowManager, OverlayWindowManager};
use tauri::{AppHandle, Manager, State};

/// Native recovery remains available while locked overlay controls are hidden.
pub(crate) async fn toggle_overlay_lock(app: &AppHandle) -> Result<(), String> {
    let state = app
        .try_state::<AppState>()
        .ok_or_else(|| "Application state is unavailable.".to_string())?;
    let _lifecycle = state.session.settings_mutation_guard(false).await?;
    apply_overlay_lock(app, &state, None)
}

#[tauri::command]
pub fn overlay_set_collapsed(
    app: AppHandle,
    state: State<'_, AppState>,
    collapsed: bool,
) -> Result<(), String> {
    sync_overlay_minimum(&app);
    let preferences = state.settings.preferences();
    let collapsed =
        normalize_overlay_collapsed(collapsed, preferences.subtitle_blends_with_background);
    state.session.set_overlay_collapsed(collapsed);
    OverlayWindowManager::set_collapsed(&app, &state.overlay, &state.settings, collapsed);
    OverlayWindowManager::sync_presentation(
        &app,
        state.session.should_show_overlay(),
        collapsed,
        preferences.overlay_locked || preferences.subtitle_blends_with_background,
        preferences.subtitle_blends_with_background,
    );
    // The frontend's collapse UI state only updates through the
    // session-state event; without this the overlay renders the wrong
    // layout after collapsing/expanding.
    state.session.publish_state();
    Ok(())
}

fn normalize_overlay_collapsed(requested: bool, background_blend: bool) -> bool {
    requested && !background_blend
}

#[tauri::command]
pub fn overlay_set_locked(
    app: AppHandle,
    state: State<'_, AppState>,
    locked: bool,
) -> Result<(), String> {
    apply_overlay_lock(&app, &state, Some(locked))
}

fn persist_overlay_lock(settings: &SettingsStore, requested: Option<bool>) -> Result<(), String> {
    // Read the toggle under the store's write lock, alongside an explicit
    // Settings/overlay update. A failed durable write keeps the old value.
    settings.save_preferences(|prefs| {
        prefs.overlay_locked = requested.unwrap_or(!prefs.overlay_locked);
    })
}

fn apply_overlay_lock(
    app: &AppHandle,
    state: &AppState,
    requested: Option<bool>,
) -> Result<(), String> {
    persist_overlay_lock(&state.settings, requested)?;
    let preferences = state.settings.preferences();
    OverlayWindowManager::sync_presentation(
        app,
        state.session.should_show_overlay(),
        state.session.is_overlay_collapsed(),
        preferences.overlay_locked || preferences.subtitle_blends_with_background,
        preferences.subtitle_blends_with_background,
    );
    // A successful overlay preference write must not turn an unreadable
    // service catalog into an apparently valid empty settings snapshot.
    let _ = emit_settings_snapshot(app, &state.settings);
    Ok(())
}

fn ensure_overlay_pointer_cursor_window(label: &str) -> Result<(), String> {
    if label == "overlay" {
        Ok(())
    } else {
        Err("overlay_pointer_cursor_not_allowed".into())
    }
}

/// Applies a renderer hit-test result for the last native hover sample only.
/// The inactive macOS WebView cannot rely on normal cursor-update tracking.
#[tauri::command]
pub async fn overlay_set_pointer_cursor(
    app: AppHandle,
    window: tauri::WebviewWindow,
    x: f64,
    y: f64,
    pointing: bool,
) -> Result<bool, String> {
    ensure_overlay_pointer_cursor_window(window.label())?;
    #[cfg(target_os = "macos")]
    {
        crate::windows::set_overlay_pointer_cursor(
            &app,
            crate::core::ui::overlay_pointer::OverlayPointerPosition { x, y },
            pointing,
        )
        .await
    }
    #[cfg(not(target_os = "macos"))]
    {
        // Other platforms use their native WebView cursor handling and do
        // not install the inactive-panel pointer relay.
        let _ = (app, x, y, pointing);
        Ok(false)
    }
}

/// Records a user drag and moves the subtitle root from either overlay surface.
#[tauri::command]
pub fn overlay_move_start(
    app: AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        crate::windows::start_overlay_drag(&app, &window, &state.overlay)
    }
    #[cfg(not(target_os = "macos"))]
    {
        #[cfg(target_os = "windows")]
        let from_panel = window.label() == "overlay-control";
        #[cfg(target_os = "windows")]
        if from_panel && !crate::windows::panel_drag_button_down() {
            // A queued DOM press must not start an OS move after its release.
            return Ok(());
        }
        let target = match window.label() {
            "overlay" => window,
            "overlay-control" => {
                let preferences = state.settings.preferences();
                let Some(label) = panel_drag_target(
                    OverlayControlWindowManager::mode(&app),
                    preferences.overlay_locked,
                    preferences.subtitle_blends_with_background,
                ) else {
                    return Ok(());
                };
                app.get_webview_window(label)
                    .ok_or_else(|| "Could not find the subtitle overlay.".to_string())?
            }
            _ => return Err("overlay_move_not_allowed".into()),
        };
        if !OverlayWindowManager::move_start(&app, &state.overlay) {
            return Ok(());
        }
        #[cfg(target_os = "windows")]
        let panel_drag = {
            // A new native move intent cannot inherit a previous gesture's up
            // or deferred focus restoration, even while Panel mode stays open.
            let retired = OverlayControlWindowManager::retire_panel_drag(&app);
            if retired && !from_panel {
                OverlayControlWindowManager::schedule_dismiss(&app);
            }
            if from_panel {
                match crate::windows::prepare_panel_drag(&app, &target) {
                    Ok(Some(drag)) => Some(drag),
                    Ok(None) => {
                        OverlayWindowManager::move_cancel(&state.overlay);
                        return Ok(());
                    }
                    Err(()) => {
                        OverlayWindowManager::move_cancel(&state.overlay);
                        return Err("Could not start overlay drag.".to_string());
                    }
                }
            } else {
                None
            }
        };
        if target.start_dragging().is_err() {
            #[cfg(target_os = "windows")]
            if let Some(drag) = panel_drag {
                drag.cancel(&app);
            }
            OverlayWindowManager::move_cancel(&state.overlay);
            return Err("Could not start overlay drag.".to_string());
        }
        #[cfg(target_os = "windows")]
        if let Some(drag) = panel_drag {
            drag.observe(&app);
        }
        Ok(())
    }
}

/// Toggles the child overlay control between its compact island and expanded
/// panel. The legacy command name remains part of the IPC contract.
#[tauri::command]
pub fn overlay_popover_toggle(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    OverlayControlWindowManager::toggle_panel(&app);
    // Refresh snapshots after a hidden/reloaded WebView so its checkmarks and
    // lifecycle guards never depend solely on an older broadcast.
    state.session.publish_settings();
    state.session.publish_state();
    Ok(())
}

/// Returns the expanded panel to its compact island. The legacy command name
/// is kept for backwards-compatible frontend bundles.
#[tauri::command]
pub fn overlay_popover_hide(app: AppHandle) -> Result<(), String> {
    OverlayControlWindowManager::dismiss_panel(&app);
    Ok(())
}

/// Current native mode for initial control-window hydration after a reload.
#[tauri::command]
pub fn overlay_control_state(app: AppHandle) -> Result<OverlayControlMode, String> {
    Ok(OverlayControlWindowManager::mode(&app))
}

/// Pickers and notifications remain interactive when Wayland subtitles are locked.
#[tauri::command]
pub fn overlay_control_set_popup_bounds(
    app: AppHandle,
    bounds: Option<crate::windows::OverlayControlGeometry>,
    notification: Option<crate::windows::OverlayControlGeometry>,
) {
    OverlayControlWindowManager::set_popup_bounds(&app, bounds, notification);
}

/// Applies a tightly-fitted panel height measured by the control WebView.
#[tauri::command]
pub fn overlay_control_set_panel_height(
    app: AppHandle,
    height: f64,
    pixel_ratio: f64,
) -> Result<(), String> {
    OverlayControlWindowManager::set_panel_height(&app, height, pixel_ratio);
    Ok(())
}

/// Applies the collapsed capsule's content width independently of its panel.
#[tauri::command]
pub fn overlay_control_set_island_width(
    app: AppHandle,
    width: f64,
    pixel_ratio: f64,
) -> Result<(), String> {
    OverlayControlWindowManager::set_island_width(&app, width, pixel_ratio);
    Ok(())
}

/// Begins an overlay resize drag. `region` is one of topLeft/top/topRight/
/// left/right/bottomLeft/bottom/bottomRight; `x`/`y` are the pointer position
/// in screen (logical) coordinates. Ignored unless the overlay is expanded.
#[tauri::command]
pub fn resize_start(
    app: AppHandle,
    state: State<'_, AppState>,
    region: String,
    x: f64,
    y: f64,
) -> Result<(), String> {
    OverlayWindowManager::resize_start(&app, &state.overlay, &region, x, y)
}

/// Continues a resize drag with the current pointer position (screen logical
/// px). The dragged edge/corner stays anchored, sizes clamp to min/max, and
/// the frame keeps at least 48 px on screen.
#[tauri::command]
pub fn resize_move(
    app: AppHandle,
    state: State<'_, AppState>,
    x: f64,
    y: f64,
) -> Result<(), String> {
    OverlayWindowManager::resize_move(&app, &state.overlay, x, y);
    Ok(())
}

/// Ends a resize drag and commits the final frame.
#[tauri::command]
pub fn resize_end(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    OverlayWindowManager::resize_end(&app, &state.overlay, &state.settings);
    Ok(())
}

#[cfg(test)]
mod tests;
