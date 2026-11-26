//! Overlay geometry state, transactions, resize and persisted user frames.
//!
//! # Overlay geometry state machine
//!
//! The overlay window has several *visual* shapes but exactly one *user*
//! frame. [`OverlayState`] owns that state:
//!
//! * `user_frame` — the user's chosen expanded size/position. The only frame
//!   that is ever persisted. Mutated exclusively by resize drags and window
//!   moves (debounced), with height clamped when subtitle requirements grow;
//!   the child control and collapse never replace its expanded size.
//! * `presentation_frame` — a runtime-only frame used while following the
//!   display that owns the active macOS Space. Matching native geometry events
//!   are ignored; a different frame is explicit user movement and is promoted
//!   to `user_frame`.
//! * `mode` — [`OverlayMode`]. Collapse is a temporary visual state *derived*
//!   from the user frame; the child control owns separate transient state.
//!
//! The overlay apply helpers are the single path that writes OS window
//! geometry: they derive size/position/min/max from `(mode, user_frame)`. OS
//! `Moved`/`Resized` events never persist directly — they only arm a debounced
//! commit (so native drags are still remembered), and control-panel mode is
//! never persisted at all. This replaces the earlier design where resize,
//! popover enlargement, collapse animation and event persistence mutated the
//! window directly and negotiated via a global `POPOVER_RESIZING` flag, which
//! caused oscillation, stuck drags, height corruption and lost frames.

mod apply;
mod geometry;
mod resize_gesture;
#[cfg(test)]
mod tests;

use self::apply::{lock_overlay_after_native_geometry_read, persist_user_frame_if_current};
#[cfg(target_os = "macos")]
use self::geometry::{active_space_follow_allowed, update_presentation_frame};
use self::geometry::{
    adopt_observed_frame, fit_promoted_user_frame, frame_layout_is_current,
    geometry_snapshot_is_current, minimum_height_for_preferences, reconcile_native_drag,
    transition_overlay_mode, NativeDragReconcile, OverlayApplySnapshot, OverlayGeometrySnapshot,
};
use self::resize_gesture::ResizeGesture;
use super::controls::OverlayControlWindowManager;
use super::dev_title;
#[cfg(target_os = "linux")]
use super::linux_input_region;
#[cfg(target_os = "macos")]
use super::macos_space;
#[cfg(target_os = "linux")]
use super::native::follow_linux_overlay_on_map;
use super::native::{
    configure_overlay_window, convert_overlay_to_panel, overlay_frame_from_window,
    show_overlay_window, OverlayPanelKind, OverlayPresentationState,
};
#[cfg(target_os = "windows")]
use super::native::{desktop_scale_for_frame, set_desktop_position, set_desktop_size};
#[cfg(target_os = "macos")]
use super::native::{reassert_overlay_window_on_active_space, show_overlay_control_window};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use super::remove_overlay_pointer_tracking;
use super::resize::ResizeRegion;
use super::work_area::{available_work_areas, default_overlay_origin, fit_user_frame_to_screen};
#[cfg(target_os = "macos")]
use super::work_area::{choose_work_area_for_frame, map_frame_between_work_areas};
use crate::core::ui::overlay_layout::BASE_MINIMUM_HEIGHT;
use crate::pipeline_log;
use crate::settings_store::{OverlayFrame, SettingsStore};
use std::sync::Arc;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

/// Persisted overlay-coordinate layout version. macOS/Linux use desktop-wide
/// logical coordinates. Windows v2 uses unambiguous physical desktop origins
/// with logical window dimensions, because independently dividing each
/// monitor's global origin by its DPI creates overlapping coordinate spaces.
#[cfg(target_os = "windows")]
pub const OVERLAY_FRAME_LAYOUT_VERSION: u64 = 2;

#[cfg(not(target_os = "windows"))]
pub const OVERLAY_FRAME_LAYOUT_VERSION: u64 = 1;

pub struct SubtitleOverlayMetrics;

impl SubtitleOverlayMetrics {
    pub const DEFAULT_WIDTH: f64 = 580.0;
    pub const DEFAULT_HEIGHT: f64 = 240.0;
    pub const MINIMUM_WIDTH: f64 = 360.0;
    pub const MINIMUM_HEIGHT: f64 = BASE_MINIMUM_HEIGHT;
    pub const MAXIMUM_WIDTH: f64 = 1_200.0;
    pub const MAXIMUM_HEIGHT: f64 = 600.0;
    pub const COLLAPSED_WIDTH: f64 = 280.0;
    pub const COLLAPSED_HEIGHT: f64 = 54.0;
}

/// The overlay's visual state. The language/mode controls live in their own
/// child window (see [`OverlayControlWindowManager`]), so the overlay has no
/// menu-related state and its height is never affected by the panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OverlayMode {
    Expanded,
    Collapsed,
}

/// All overlay geometry state: the single source of truth for the overlay
/// window's size and position.
#[derive(Debug)]
pub struct OverlayState {
    pub mode: OverlayMode,
    /// Distinguishes repeated visits to the same mode, so an interrupted
    /// animation cannot become current again after another collapse/expand.
    mode_revision: u64,
    /// Current subtitle requirement; part of every queued geometry transaction.
    minimum_height: f64,
    /// Temporary readable, interactive presentation; never persisted.
    session_error: bool,
    /// The user's chosen expanded frame; the only persisted frame.
    pub user_frame: OverlayFrame,
    /// Runtime-only frame used to present the overlay on the screen owning the
    /// active macOS Space. Programmatic follow moves never replace
    /// `user_frame`; a later manual move or resize promotes the observed frame
    /// through the ordinary persistence path.
    presentation_frame: Option<OverlayFrame>,
    /// Native titlebar-style drag origin. The explicit mouse-down marker lets
    /// a later collapse or Space transition distinguish a real user move from
    /// an in-flight programmatic resize animation.
    native_drag_start: Option<(f64, f64)>,
    /// Active resize drag, if any (only valid while `mode == Expanded`).
    pub resize_drag: Option<ResizeRegion>,
    /// Pointer position and frame snapshot taken at `resize_start`.
    resize_start: Option<ResizeGesture>,
    /// Timestamp of the most recent Moved/Resized event; debounced commits
    /// compare against it to drop superseded tasks.
    last_geometry_event: Option<std::time::Instant>,
    /// Whether a debounced geometry-commit task is already in flight; avoids
    /// spawning one task per Moved/Resized event during drags (60+/s).
    geometry_task_pending: bool,
    /// Last time a resize-move was logged (throttles the per-event log).
    resize_log_at: Option<std::time::Instant>,
}

impl OverlayState {
    /// Loads the persisted frame (or the default bottom-center placement) and
    /// starts in the expanded mode.
    pub fn load(app: &AppHandle, settings: &SettingsStore) -> Self {
        let prefs = settings.preferences();
        let minimum_height = minimum_height_for_preferences(&prefs);
        let trusted = frame_layout_is_current(prefs.frame_layout_version)
            .then_some(prefs.overlay_frame)
            .flatten()
            .filter(|frame| {
                frame.width >= SubtitleOverlayMetrics::MINIMUM_WIDTH
                    && frame.height >= SubtitleOverlayMetrics::MINIMUM_HEIGHT
            });
        let width = trusted
            .as_ref()
            .map_or(SubtitleOverlayMetrics::DEFAULT_WIDTH, |f| f.width);
        let height = trusted
            .as_ref()
            .map_or(SubtitleOverlayMetrics::DEFAULT_HEIGHT, |f| f.height)
            .max(minimum_height);
        let (x, y) = default_overlay_origin(app, width, height, &trusted);
        let mut user_frame = OverlayFrame {
            x,
            y,
            width,
            height,
        };
        // A saved frame can outgrow the current work area after a monitor is
        // removed or its resolution changes. Fit the complete frame, not just
        // its origin, so restore can never bring back a half-missing overlay.
        fit_user_frame_to_screen(app, &mut user_frame, minimum_height);
        Self {
            mode: OverlayMode::Expanded,
            mode_revision: 0,
            minimum_height,
            user_frame,
            presentation_frame: None,
            session_error: false,
            native_drag_start: None,
            resize_drag: None,
            resize_start: None,
            last_geometry_event: None,
            geometry_task_pending: false,
            resize_log_at: None,
        }
    }
}

pub struct OverlayWindowManager;

impl OverlayWindowManager {
    /// Update constraints before broadcasting new visual/capture preferences.
    /// Read current preferences under the same overlay→settings lock order as
    /// geometry persistence, so an older broadcast cannot apply stale rules.
    pub fn sync_minimum_height(
        app: &AppHandle,
        overlay_state: &Arc<std::sync::Mutex<OverlayState>>,
        settings: &SettingsStore,
    ) {
        {
            let state = overlay_state.lock().unwrap();
            if state.minimum_height == minimum_height_for_preferences(&settings.preferences()) {
                return;
            }
        }
        let (mut state, observed, areas, primary) =
            lock_overlay_after_native_geometry_read(app, overlay_state);
        let manual_drag = if state.session_error {
            reconcile_native_drag(&mut state, observed, &areas, primary)
        } else {
            NativeDragReconcile::default()
        };
        let minimum = minimum_height_for_preferences(&settings.preferences());
        let changed = state.update_minimum_height(minimum);
        let frame_to_persist = manual_drag.promoted.then_some(state.user_frame);
        let expected = OverlayApplySnapshot::from(&*state);
        drop(state);
        if let Some(frame) = frame_to_persist {
            persist_user_frame_if_current(overlay_state, settings, frame);
        }
        if changed || manual_drag.fit_changed {
            Self::apply_if_geometry_current(app, overlay_state, expected, false);
        }
    }

    pub fn ensure_overlay(app: &AppHandle, state: &Arc<std::sync::Mutex<OverlayState>>) {
        if app.get_webview_window("overlay").is_some() {
            return;
        }
        let (frame, minimum_height) = {
            let state = state.lock().unwrap();
            (state.user_frame, state.minimum_height)
        };
        #[cfg(target_os = "linux")]
        let uses_x11 = {
            use gtk::prelude::*;
            gtk::gdk::Display::default()
                .is_some_and(|display| display.type_().name() == "GdkX11Display")
        };
        // GTK's X11 resize grips can take the press meant for an overlay
        // button. Mimi's own handles still resize the window through Tao.
        #[cfg(target_os = "linux")]
        let native_resize = !uses_x11;
        #[cfg(not(target_os = "linux"))]
        let native_resize = true;
        let builder =
            WebviewWindowBuilder::new(app, "overlay", WebviewUrl::App("index.html".into()))
                .title(dev_title("Mimi Subtitles"))
                .inner_size(frame.width, frame.height)
                .min_inner_size(SubtitleOverlayMetrics::MINIMUM_WIDTH, minimum_height)
                .max_inner_size(
                    SubtitleOverlayMetrics::MAXIMUM_WIDTH,
                    SubtitleOverlayMetrics::MAXIMUM_HEIGHT,
                )
                .transparent(true)
                .decorations(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .shadow(false)
                .resizable(native_resize)
                .visible(false);
        let builder = if OverlayControlWindowManager::is_embedded(app) {
            builder.initialization_script("window.__MIMI_EMBEDDED_OVERLAY_CONTROL__ = true;")
        } else {
            builder
        };
        #[cfg(not(target_os = "windows"))]
        let builder = builder
            .position(frame.x, frame.y)
            .visible_on_all_workspaces(true);
        #[cfg(target_os = "windows")]
        let builder = builder.focusable(false).focused(false);
        // A nonactivating panel's first deliberate click must also reach its
        // WebView control, rather than merely focusing the native window.
        #[cfg(target_os = "macos")]
        let builder = builder.accept_first_mouse(true);

        match builder.build() {
            Ok(window) => {
                if convert_overlay_to_panel(&window, OverlayPanelKind::Subtitles).is_err() {
                    let _ = window.destroy();
                    pipeline_log!("overlay window failed label=panel_conversion_failed");
                    return;
                }
                #[cfg(target_os = "windows")]
                {
                    set_desktop_position(&window, frame.x, frame.y);
                    let scale = desktop_scale_for_frame(app, &window, &frame);
                    set_desktop_size(&window, frame.width, frame.height, scale);
                }
                if let Some(presentation) = app.try_state::<OverlayPresentationState>() {
                    presentation.invalidate();
                }
                configure_overlay_window(&window);
                #[cfg(target_os = "linux")]
                follow_linux_overlay_on_map(&window);
                pipeline_log!("overlay window created");
            }
            Err(_) => pipeline_log!("overlay window failed label=create_failed"),
        }
    }

    /// Applies the complete native overlay presentation in one place:
    /// visibility, click-through behavior, and the child control surface.
    /// The control island remains available for an active, expanded,
    /// non-immersive overlay. Position lock returns an open panel to its
    /// island; ordinary unlocked state broadcasts preserve an open panel.
    pub fn sync_presentation(
        app: &AppHandle,
        is_active: bool,
        is_collapsed: bool,
        click_through: bool,
        is_immersive: bool,
    ) {
        let app_for_main = app.clone();
        if app
            .run_on_main_thread(move || {
                Self::sync_presentation_on_main(
                    &app_for_main,
                    is_active,
                    is_collapsed,
                    click_through,
                    is_immersive,
                );
            })
            .is_err()
        {
            tracing::warn!("overlay presentation unavailable label=main_dispatch_failed");
        }
    }

    fn sync_presentation_on_main(
        app: &AppHandle,
        is_active: bool,
        is_collapsed: bool,
        click_through: bool,
        is_immersive: bool,
    ) {
        let has_error = if let Some(app_state) = app.try_state::<crate::commands::AppState>() {
            let has_error = app_state.session.status_kind() == "error";
            let transition = app_state.overlay.lock().unwrap().session_error != has_error;
            let geometry = if transition {
                // A settings repair can retire the error before the 350 ms
                // geometry debounce. Accept explicit movement before resetting
                // its drag marker or restoring the normal presentation.
                let (mut state, observed, areas, primary) =
                    lock_overlay_after_native_geometry_read(app, &app_state.overlay);
                let manual_drag = reconcile_native_drag(&mut state, observed, &areas, primary);
                let frame_to_persist = manual_drag.promoted.then_some(state.user_frame);
                let geometry = state.set_session_error(has_error).then(|| {
                    (
                        state.effective_mode(),
                        state.effective_frame(),
                        state.effective_minimum_height(),
                    )
                });
                drop(state);
                if let Some(frame) = frame_to_persist {
                    persist_user_frame_if_current(&app_state.overlay, &app_state.settings, frame);
                }
                geometry
            } else {
                None
            };
            if let Some((mode, mut frame, minimum_height)) = geometry {
                // This presentation cannot alter the user's saved size or position.
                // Change the interpreting state and native frame together on main.
                if has_error {
                    fit_user_frame_to_screen(app, &mut frame, minimum_height);
                }
                Self::apply_frame(app, mode, frame, minimum_height, None);
            }
            has_error
        } else {
            false
        };
        let is_active = is_active || has_error;
        let is_collapsed = is_collapsed && !has_error;
        let click_through = click_through && !has_error;
        let is_immersive = is_immersive && !has_error;
        if is_active {
            if click_through {
                // Position lock keeps the independent hover-to-unlock island.
                // Immersive mode hides the complete control surface.
                // GTK can store an input shape before the first native map.
                // Record lock intent first so realize/map cannot briefly
                // restore an old unlocked shape when a worker shows a window.
                #[cfg(target_os = "linux")]
                Self::update_locked(app, true);
                Self::sync_overlay_visibility(app, true);
                OverlayControlWindowManager::sync_presentation(
                    app,
                    true,
                    is_collapsed,
                    click_through,
                    is_immersive,
                );
                #[cfg(not(target_os = "linux"))]
                Self::update_locked(app, true);
            } else {
                // Restore canvas interaction before changing either visible
                // surface, so unlocking never leaves an inert frame behind.
                Self::update_locked(app, false);
                Self::sync_overlay_visibility(app, true);
                OverlayControlWindowManager::sync_presentation(
                    app,
                    true,
                    is_collapsed,
                    false,
                    is_immersive,
                );
            }
        } else {
            // Hide the child first so it cannot linger for a frame after its
            // subtitle parent disappears.
            OverlayControlWindowManager::sync_presentation(
                app,
                false,
                is_collapsed,
                click_through,
                is_immersive,
            );
            Self::sync_overlay_visibility(app, false);
            Self::update_locked(app, click_through);
        }
    }

    fn sync_overlay_visibility(app: &AppHandle, is_active: bool) {
        let Some(window) = app.get_webview_window("overlay") else {
            return;
        };
        let visible = window.is_visible().unwrap_or(false);
        // Only act when the visibility actually changes: `show()` on an
        // already-visible window re-activates it and steals focus from the
        // child control on every subtitle update.
        if is_active && !visible {
            show_overlay_window(&window);
        } else if !is_active && visible && window.hide().is_ok() {
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            remove_overlay_pointer_tracking(app, window.label());
        }
    }

    /// Restores already-visible overlay surfaces to the active Space after an
    /// explicit show request. This is deliberately separate from ordinary
    /// streaming state broadcasts so subtitle updates never reorder windows or
    /// disturb focus.
    #[cfg(target_os = "macos")]
    pub fn reassert_on_active_space(app: &AppHandle) {
        let Some(overlay) = app.get_webview_window("overlay") else {
            return;
        };
        if overlay.is_visible().unwrap_or(false) {
            reassert_overlay_window_on_active_space(&overlay);
        }

        let Some(control) = app.get_webview_window("overlay-control") else {
            return;
        };
        if control.is_visible().unwrap_or(false) {
            show_overlay_control_window(app, &control);
        }
    }

    /// Follows the display that owns macOS's active Space without replacing
    /// the user's saved frame. The AppKit screen lookup and the resulting
    /// presentation transaction must run on the main thread.
    #[cfg(target_os = "macos")]
    pub fn follow_active_space(
        app: &AppHandle,
        overlay_state: &Arc<std::sync::Mutex<OverlayState>>,
        settings: &Arc<SettingsStore>,
    ) {
        let Some(overlay) = app.get_webview_window("overlay") else {
            return;
        };
        let app_for_main = app.clone();
        let state_for_main = Arc::clone(overlay_state);
        let settings_for_main = Arc::clone(settings);
        let _ = overlay.run_on_main_thread(move || {
            Self::follow_active_space_on_main(&app_for_main, &state_for_main, &settings_for_main);
        });
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn follow_active_space_on_main(
        app: &AppHandle,
        overlay_state: &Arc<std::sync::Mutex<OverlayState>>,
        settings: &SettingsStore,
    ) {
        let Some(target_area) = macos_space::main_screen_work_area() else {
            Self::reassert_on_active_space(app);
            return;
        };

        let (mut state, observed_frame, work_areas, primary_work_area) =
            lock_overlay_after_native_geometry_read(app, overlay_state);
        let manual_drag =
            reconcile_native_drag(&mut state, observed_frame, &work_areas, primary_work_area);
        let follow_changed = if active_space_follow_allowed(&state) {
            choose_work_area_for_frame(
                &state.user_frame,
                work_areas.iter().copied(),
                primary_work_area,
            )
            .map(|source_area| {
                let next = map_frame_between_work_areas(
                    state.user_frame,
                    source_area,
                    target_area,
                    state.minimum_height,
                );
                update_presentation_frame(&mut state, next)
            })
            .unwrap_or(false)
        } else {
            // Never replace a live resize frame with a Space transition. The
            // user's pointer owns geometry until resize_end.
            false
        };
        let frame_to_persist = manual_drag.promoted.then_some(state.user_frame);
        let should_apply = follow_changed || manual_drag.fit_changed;
        let expected_state = OverlayApplySnapshot::from(&*state);
        drop(state);

        if let Some(frame) = frame_to_persist {
            persist_user_frame_if_current(overlay_state, settings, frame);
        }
        if should_apply {
            Self::apply_if_geometry_current(app, overlay_state, expected_state, false);
        }
        Self::reassert_on_active_space(app);
    }

    /// Locks the overlay: the window ignores mouse events so clicks pass
    /// through to whatever plays underneath.
    pub fn update_locked(app: &AppHandle, locked: bool) {
        let Some(window) = app.get_webview_window("overlay") else {
            return;
        };
        if let Some(presentation) = app.try_state::<OverlayPresentationState>() {
            presentation.apply_click_through_if_changed(&window, locked);
        } else {
            #[cfg(not(target_os = "linux"))]
            let _ = window.set_ignore_cursor_events(locked);
            #[cfg(target_os = "linux")]
            {
                let native_window = window.clone();
                let _ = window.run_on_main_thread(move || {
                    if let Ok(native) = native_window.gtk_window() {
                        linux_input_region::apply(&native, locked);
                    }
                });
            }
        }
    }

    /// Collapses to 280×54 or expands to the remembered frame. The size
    /// change uses the shared 180 ms ease-in-out timing, and expansion clamps
    /// the remembered frame back onto the visible screen.
    pub fn set_collapsed(
        app: &AppHandle,
        overlay_state: &Arc<std::sync::Mutex<OverlayState>>,
        settings: &SettingsStore,
        collapsed: bool,
    ) {
        let (mut state, observed_frame, work_areas, primary_work_area) =
            lock_overlay_after_native_geometry_read(app, overlay_state);
        let transition = transition_overlay_mode(
            &mut state,
            observed_frame,
            &work_areas,
            primary_work_area,
            collapsed,
        );
        let expected_state = OverlayApplySnapshot::from(&*state);
        drop(state);
        if let Some(frame) = transition.frame_to_persist {
            persist_user_frame_if_current(overlay_state, settings, frame);
        }
        if transition.geometry_changed {
            Self::apply_if_geometry_current(
                app,
                overlay_state,
                expected_state,
                transition.mode_changed,
            );
        }
    }

    /// Handles OS Moved/Resized events for the overlay. Never persists
    /// immediately: a debounced task (350 ms after the last event) folds the
    /// final window frame into the user frame, so native drags are remembered
    /// but transient animation steps are not.
    pub fn on_geometry_event(
        app: &AppHandle,
        state: &Arc<std::sync::Mutex<OverlayState>>,
        settings: &Arc<SettingsStore>,
    ) {
        let now = std::time::Instant::now();
        {
            let mut state = state.lock().unwrap();
            state.last_geometry_event = Some(now);
            // One debounced commit task is enough: the in-flight task tails
            // newer timestamps until events settle, so no commit is lost and
            // no task is spawned per event.
            if state.geometry_task_pending {
                return;
            }
            state.geometry_task_pending = true;
        }
        let app = app.clone();
        let overlay_state = Arc::clone(state);
        let settings = Arc::clone(settings);
        tauri::async_runtime::spawn(async move {
            let mut expected = now;
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(350)).await;
                let expected_state = {
                    let mut state = overlay_state.lock().unwrap();
                    if state.last_geometry_event != Some(expected) {
                        // Newer events arrived while we slept; tail them
                        // instead of dropping their commit (the single-flight
                        // flag made them skip spawning their own task).
                        expected = state.last_geometry_event.expect("checked as Some");
                        continue;
                    }
                    if state.resize_drag.is_some() {
                        // A stationary pointer can leave a live resize quiet
                        // for longer than the debounce. `resize_end` owns the
                        // final fit so the window never snaps before mouse-up.
                        state.geometry_task_pending = false;
                        return;
                    }
                    OverlayGeometrySnapshot::from(&*state)
                };

                // Tauri's window/monitor getters can synchronously rendezvous
                // with the main event loop. Never call them while holding the
                // overlay mutex: the active-Space callback runs on main and
                // also needs this state, so doing both would invert the locks.
                let observed_frame = overlay_frame_from_window(&app);
                let (work_areas, primary_work_area) = available_work_areas(&app);

                let mut state = overlay_state.lock().unwrap();
                if state.last_geometry_event != Some(expected)
                    || !geometry_snapshot_is_current(&state, expected_state)
                {
                    // Geometry or the state used to interpret it changed while
                    // the native frame was being read. Re-read after the newer
                    // event/state transition has settled.
                    expected = state.last_geometry_event.expect("checked as Some");
                    continue;
                }
                state.geometry_task_pending = false;
                let Some(observed_frame) = observed_frame else {
                    drop(state);
                    OverlayControlWindowManager::follow_overlay(&app);
                    return;
                };
                if !adopt_observed_frame(&mut state, observed_frame) {
                    // This is the exact Moved/Resized event produced by an
                    // active-Space follow (or its collapse/expand geometry).
                    // Keep the canonical user frame and preferences untouched.
                    drop(state);
                    OverlayControlWindowManager::follow_overlay(&app);
                    return;
                }
                // A different native frame means the user moved the followed
                // overlay. Promote that intentional placement to canonical
                // state and resume the ordinary persistence path.
                let geometry_changed = fit_promoted_user_frame(
                    &mut state,
                    &observed_frame,
                    &work_areas,
                    primary_work_area,
                );
                let frame_to_persist = state.user_frame;
                let expected_state = OverlayApplySnapshot::from(&*state);
                drop(state);
                persist_user_frame_if_current(&overlay_state, &settings, frame_to_persist);
                if geometry_changed {
                    // Native movement remains untouched while the pointer is
                    // down. Once it settles, perform at most one corrective
                    // write so the whole overlay returns to the work area
                    // without making the control island trail during drag.
                    Self::apply_if_geometry_current(&app, &overlay_state, expected_state, false);
                }
                // AppKit owns live child movement on macOS, avoiding a
                // one-event-late manual correction during the drag. Once the
                // parent settles, re-derive the child geometry on every
                // platform so work-area and scale changes are still clamped.
                OverlayControlWindowManager::follow_overlay(&app);
                return;
            }
        });
    }
}
