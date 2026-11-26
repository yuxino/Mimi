//! Main-thread overlay geometry writes, lock-free reads, persistence, and easing animation.

use super::geometry::{
    apply_snapshot_is_current, geometry_for, geometry_snapshot_is_current, OverlayApplySnapshot,
    OverlayGeometrySnapshot,
};
use super::{OverlayMode, OverlayState, OverlayWindowManager, OVERLAY_FRAME_LAYOUT_VERSION};
use crate::settings_store::{OverlayFrame, SettingsStore};
use crate::windows::native::{
    desktop_scale_for_frame, overlay_frame_from_window, set_desktop_position, set_desktop_size,
};
use crate::windows::work_area::{available_work_areas, fit_user_frame_to_screen, LogicalWorkArea};
use std::sync::Arc;
use tauri::{AppHandle, Manager};

#[derive(Clone)]
pub(super) struct GeometryAnimationGuard {
    state: Arc<std::sync::Mutex<OverlayState>>,
    expected: OverlayApplySnapshot,
}

impl OverlayWindowManager {
    /// The complete writer of OS window geometry. Derives and applies
    /// size/position/min/max, with an optional cancellable animation guard.
    pub(super) fn apply_frame(
        app: &AppHandle,
        mode: OverlayMode,
        frame: OverlayFrame,
        minimum_height: f64,
        animation: Option<GeometryAnimationGuard>,
    ) {
        let Some(window) = app.get_webview_window("overlay") else {
            return;
        };
        let geometry = geometry_for(mode, &frame, minimum_height);
        let minimum = Some(tauri::LogicalSize::new(geometry.min.0, geometry.min.1));
        let maximum = Some(tauri::LogicalSize::new(geometry.max.0, geometry.max.1));
        // Expanding must lift the collapsed maximum before raising its
        // minimum. Collapsing first lowers the expanded minimum instead.
        match mode {
            OverlayMode::Expanded => {
                let _ = window.set_max_size(maximum);
                let _ = window.set_min_size(minimum);
            }
            OverlayMode::Collapsed => {
                let _ = window.set_min_size(minimum);
                let _ = window.set_max_size(maximum);
            }
        }
        if let Some(animation) = animation {
            let scale = desktop_scale_for_frame(app, &window, &frame);
            set_desktop_position(&window, geometry.x, geometry.y);
            animate_resize(&window, animation, scale, geometry.width, geometry.height);
        } else {
            let scale = desktop_scale_for_frame(app, &window, &frame);
            set_desktop_position(&window, geometry.x, geometry.y);
            set_desktop_size(&window, geometry.width, geometry.height, scale);
        }
    }

    /// Serializes deferred geometry writes through the main event loop and
    /// drops any transaction whose interpreting state has already changed.
    /// The closure re-derives the frame after validation instead of trusting a
    /// worker-thread copy, so a newer Space/collapse transition always wins.
    pub(super) fn apply_if_geometry_current(
        app: &AppHandle,
        state: &Arc<std::sync::Mutex<OverlayState>>,
        expected: OverlayApplySnapshot,
        animate: bool,
    ) {
        let app_for_main = app.clone();
        let state_for_main = Arc::clone(state);
        if app
            .run_on_main_thread(move || {
                let (mode, frame, minimum_height) = {
                    let state = state_for_main.lock().unwrap();
                    if !apply_snapshot_is_current(&state, expected) {
                        return;
                    }
                    (
                        state.effective_mode(),
                        state.effective_frame(),
                        state.effective_minimum_height(),
                    )
                };
                let mut frame = frame;
                if expected.session_error {
                    fit_user_frame_to_screen(&app_for_main, &mut frame, minimum_height);
                }
                let animation =
                    (animate && !expected.session_error).then(|| GeometryAnimationGuard {
                        state: Arc::clone(&state_for_main),
                        expected,
                    });
                Self::apply_frame(&app_for_main, mode, frame, minimum_height, animation);
            })
            .is_err()
        {
            tracing::warn!("overlay geometry unavailable label=main_dispatch_failed");
        }
    }

    /// High-frequency resize writes use the same guarded main-thread ordering
    /// without resetting min/max constraints on every pointer event.
    pub(super) fn apply_resize_if_geometry_current(
        app: &AppHandle,
        state: &Arc<std::sync::Mutex<OverlayState>>,
        expected: OverlayApplySnapshot,
    ) {
        let app_for_main = app.clone();
        let state_for_main = Arc::clone(state);
        if app
            .run_on_main_thread(move || {
                let frame = {
                    let state = state_for_main.lock().unwrap();
                    if !apply_snapshot_is_current(&state, expected) {
                        return;
                    }
                    state.effective_frame()
                };
                let Some(window) = app_for_main.get_webview_window("overlay") else {
                    return;
                };
                let scale = desktop_scale_for_frame(&app_for_main, &window, &frame);
                set_desktop_position(&window, frame.x, frame.y);
                set_desktop_size(&window, frame.width, frame.height, scale);
            })
            .is_err()
        {
            tracing::warn!("overlay resize unavailable label=main_dispatch_failed");
        }
    }
}

/// Reads native geometry without holding the overlay mutex, then returns only
/// when the state used to interpret that read is still current. This keeps
/// Tokio command paths from waiting on the main event loop while main is
/// waiting for the same mutex.
pub(super) fn lock_overlay_after_native_geometry_read<'a>(
    app: &AppHandle,
    state: &'a Arc<std::sync::Mutex<OverlayState>>,
) -> (
    std::sync::MutexGuard<'a, OverlayState>,
    Option<OverlayFrame>,
    Vec<LogicalWorkArea>,
    Option<LogicalWorkArea>,
) {
    loop {
        let (expected_state, expected_event) = {
            let state = state.lock().unwrap();
            (
                OverlayGeometrySnapshot::from(&*state),
                state.last_geometry_event,
            )
        };
        let observed_frame = overlay_frame_from_window(app);
        let (work_areas, primary_work_area) = available_work_areas(app);
        let state = state.lock().unwrap();
        if geometry_snapshot_is_current(&state, expected_state)
            && state.last_geometry_event == expected_event
        {
            return (state, observed_frame, work_areas, primary_work_area);
        }
    }
}

/// Writes the user frame to preferences. Locked overlays are skipped (their
/// position is pinned by design).
fn persist_user_frame(settings: &SettingsStore, frame: &OverlayFrame) {
    if settings.preferences().overlay_locked {
        return;
    }
    let frame = *frame;
    if settings
        .save_preferences(|prefs| {
            prefs.overlay_frame = Some(frame);
            prefs.frame_layout_version = OVERLAY_FRAME_LAYOUT_VERSION;
        })
        .is_err()
    {
        tracing::warn!("preferences unavailable label=overlay_frame_write_failed");
    }
}

/// Commits only the still-current user frame. Holding the overlay mutex during
/// this small preferences transaction serializes competing manual placements;
/// no Tauri/AppKit call is made here, so it cannot participate in the native
/// main-thread lock cycle guarded elsewhere.
pub(super) fn persist_user_frame_if_current(
    state: &Arc<std::sync::Mutex<OverlayState>>,
    settings: &SettingsStore,
    expected_frame: OverlayFrame,
) {
    let state = state.lock().unwrap();
    if state.user_frame != expected_frame {
        return;
    }
    persist_user_frame(settings, &expected_frame);
}

/// Animated 180 ms ease-in-out size transition. Every step re-enters the main
/// queue and validates the geometry snapshot there, so a newer Space or mode
/// transition cancels all remaining writes before they can overwrite it.
fn animate_resize(
    window: &tauri::WebviewWindow,
    guard: GeometryAnimationGuard,
    scale: f64,
    target_width: f64,
    target_height: f64,
) {
    let start = window.inner_size().unwrap_or_else(|_| {
        tauri::PhysicalSize::new(
            (target_width * scale) as u32,
            (target_height * scale) as u32,
        )
    });
    let end = tauri::PhysicalSize::new(
        (target_width * scale).round() as u32,
        (target_height * scale).round() as u32,
    );
    let window = window.clone();
    tauri::async_runtime::spawn(async move {
        const STEPS: usize = 12;
        for step in 1..=STEPS {
            tokio::time::sleep(std::time::Duration::from_millis(15)).await;
            if !apply_snapshot_is_current(&guard.state.lock().unwrap(), guard.expected) {
                return;
            }
            let t = ease_in_out(step as f64 / STEPS as f64);
            let width = start.width as f64 + (end.width as f64 - start.width as f64) * t;
            let height = start.height as f64 + (end.height as f64 - start.height as f64) * t;
            let size = tauri::PhysicalSize::new(width.round() as u32, height.round() as u32);
            let dispatch_window = window.clone();
            let target_window = window.clone();
            let step_guard = guard.clone();
            if dispatch_window
                .run_on_main_thread(move || {
                    if !apply_snapshot_is_current(
                        &step_guard.state.lock().unwrap(),
                        step_guard.expected,
                    ) {
                        return;
                    }
                    let _ = target_window.set_size(size);
                })
                .is_err()
            {
                return;
            }
        }
    });
}

/// Cubic ease-in-out used by the native geometry transition.
fn ease_in_out(t: f64) -> f64 {
    if t < 0.5 {
        2.0 * t * t
    } else {
        1.0 - ((-2.0 * t + 2.0).powi(2)) / 2.0
    }
}
