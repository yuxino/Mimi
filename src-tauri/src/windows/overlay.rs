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

use super::resize::{apply_drag, ResizeRegion};
use crate::core::ui::overlay_layout::{minimum_overlay_height, BASE_MINIMUM_HEIGHT};
use crate::pipeline_log;
use crate::settings_store::{OverlayFrame, Preferences, SettingsStore};
use std::sync::Arc;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use super::controls::OverlayControlWindowManager;
use super::dev_title;
#[cfg(target_os = "linux")]
use super::linux_input_region;
#[cfg(target_os = "macos")]
use super::macos_space;
#[cfg(target_os = "linux")]
use super::native::{configure_overlay_for_software_gl, follow_linux_overlay_on_map};
use super::native::{
    configure_overlay_window, convert_overlay_to_panel, desktop_scale_for_frame,
    overlay_frame_from_window, set_desktop_position, set_desktop_size, show_overlay_window,
    OverlayPanelKind, OverlayPresentationState,
};
#[cfg(target_os = "macos")]
use super::native::{reassert_overlay_window_on_active_space, show_overlay_control_window};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use super::remove_overlay_pointer_tracking;
use super::work_area::{
    available_work_areas, current_work_area_logical, default_overlay_origin,
    fit_collapsed_position_to_work_areas, fit_user_frame_to_screen, fit_user_frame_to_work_areas,
    LogicalWorkArea,
};
#[cfg(target_os = "macos")]
use super::work_area::{choose_work_area_for_frame, map_frame_between_work_areas};

/// Persisted overlay-coordinate layout version. macOS/Linux use desktop-wide
/// logical coordinates. Windows v2 uses unambiguous physical desktop origins
/// with logical window dimensions, because independently dividing each
/// monitor's global origin by its DPI creates overlapping coordinate spaces.
#[cfg(target_os = "windows")]
pub const OVERLAY_FRAME_LAYOUT_VERSION: u64 = 2;

#[cfg(not(target_os = "windows"))]
pub const OVERLAY_FRAME_LAYOUT_VERSION: u64 = 1;

fn frame_layout_is_current(version: u64) -> bool {
    version == OVERLAY_FRAME_LAYOUT_VERSION
}

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

#[derive(Debug, Clone, Copy, PartialEq)]
struct OverlayApplySnapshot {
    mode: OverlayMode,
    mode_revision: u64,
    session_error: bool,
    minimum_height: f64,
    user_frame: OverlayFrame,
    presentation_frame: Option<OverlayFrame>,
    resize_drag: Option<ResizeRegion>,
    resize_start: Option<ResizeGesture>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ResizeGesture {
    pointer: (f64, f64),
    frame: OverlayFrame,
    work_area: LogicalWorkArea,
}

impl From<&OverlayState> for OverlayApplySnapshot {
    fn from(state: &OverlayState) -> Self {
        Self {
            mode: state.mode,
            mode_revision: state.mode_revision,
            session_error: state.session_error,
            minimum_height: state.minimum_height,
            user_frame: state.user_frame,
            presentation_frame: state.presentation_frame,
            resize_drag: state.resize_drag,
            resize_start: state.resize_start,
        }
    }
}

/// Native reads also include the drag marker so an origin observed before a
/// programmatic transition can never be attached to the state after it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct OverlayGeometrySnapshot {
    apply: OverlayApplySnapshot,
    native_drag_start: Option<(f64, f64)>,
}

impl From<&OverlayState> for OverlayGeometrySnapshot {
    fn from(state: &OverlayState) -> Self {
        Self {
            apply: OverlayApplySnapshot::from(state),
            native_drag_start: state.native_drag_start,
        }
    }
}

#[derive(Clone)]
struct GeometryAnimationGuard {
    state: Arc<std::sync::Mutex<OverlayState>>,
    expected: OverlayApplySnapshot,
}

fn geometry_snapshot_is_current(state: &OverlayState, expected: OverlayGeometrySnapshot) -> bool {
    OverlayGeometrySnapshot::from(state) == expected
}

fn apply_snapshot_is_current(state: &OverlayState, expected: OverlayApplySnapshot) -> bool {
    OverlayApplySnapshot::from(state) == expected
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

    fn effective_mode(&self) -> OverlayMode {
        if self.session_error {
            OverlayMode::Expanded
        } else {
            self.mode
        }
    }

    fn effective_minimum_height(&self) -> f64 {
        if self.session_error {
            self.minimum_height.max(280.0)
        } else {
            self.minimum_height
        }
    }

    pub(in crate::windows) fn effective_frame(&self) -> OverlayFrame {
        let mut frame = self.presentation_frame.unwrap_or(self.user_frame);
        frame.height = frame.height.max(self.effective_minimum_height());
        frame
    }

    fn set_session_error(&mut self, enabled: bool) -> bool {
        if self.session_error == enabled {
            return false;
        }
        self.session_error = enabled;
        self.native_drag_start = None;
        self.resize_drag = None;
        self.resize_start = None;
        true
    }

    fn update_minimum_height(&mut self, minimum_height: f64) -> bool {
        if self.minimum_height == minimum_height {
            return false;
        }
        self.minimum_height = minimum_height;
        if self.session_error {
            // The resulting native resize/fit is programmatic, even if an
            // earlier manual error drag had already settled.
            self.native_drag_start = None;
        }
        // A smaller requirement lets the user resize later; it never shrinks
        // their chosen frame or resets its position/width automatically.
        self.user_frame.height = self.user_frame.height.max(minimum_height);
        if let Some(frame) = self.presentation_frame.as_mut() {
            frame.height = frame.height.max(minimum_height);
        }
        true
    }
}

fn minimum_height_for_preferences(preferences: &Preferences) -> f64 {
    minimum_overlay_height(
        preferences.audio_input,
        preferences.subtitle_display_mode,
        preferences.target_language,
        preferences.font_size,
        preferences.show_subtitle_timestamps,
    )
}

/// OS window geometry derived from `(mode, user_frame)`; all logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
struct WindowGeometry {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    min: (f64, f64),
    max: (f64, f64),
}

/// Pure derivation of the window geometry from the overlay state. `screen` is
/// the logical size of the screen the overlay currently sits on.
fn geometry_for(
    mode: OverlayMode,
    user_frame: &OverlayFrame,
    minimum_height: f64,
) -> WindowGeometry {
    let min = (SubtitleOverlayMetrics::MINIMUM_WIDTH, minimum_height);
    let max = (
        SubtitleOverlayMetrics::MAXIMUM_WIDTH,
        SubtitleOverlayMetrics::MAXIMUM_HEIGHT,
    );
    match mode {
        OverlayMode::Expanded => WindowGeometry {
            x: user_frame.x,
            y: user_frame.y,
            width: user_frame.width,
            height: user_frame.height.max(minimum_height),
            min,
            max,
        },
        OverlayMode::Collapsed => WindowGeometry {
            x: user_frame.x,
            y: user_frame.y,
            width: SubtitleOverlayMetrics::COLLAPSED_WIDTH,
            height: SubtitleOverlayMetrics::COLLAPSED_HEIGHT,
            // Fixed size: min == max so the OS cannot resize the collapsed bar.
            min: (
                SubtitleOverlayMetrics::COLLAPSED_WIDTH,
                SubtitleOverlayMetrics::COLLAPSED_HEIGHT,
            ),
            max: (
                SubtitleOverlayMetrics::COLLAPSED_WIDTH,
                SubtitleOverlayMetrics::COLLAPSED_HEIGHT,
            ),
        },
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
                #[cfg(target_os = "linux")]
                if uses_x11 {
                    configure_overlay_for_software_gl(&window);
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

    /// The complete writer of OS window geometry. Derives and applies
    /// size/position/min/max, with an optional cancellable animation guard.
    fn apply_frame(
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
    fn apply_if_geometry_current(
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
    fn apply_resize_if_geometry_current(
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
                    if !apply_snapshot_is_current(&state, expected)
                        || state.mode != OverlayMode::Expanded
                        || state.session_error
                    {
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
                if state.resize_drag.is_some() {
                    return;
                }
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

    /// Marks the beginning of Tauri's native window drag. AppKit owns the
    /// gesture, so this origin is the only reliable way to distinguish a
    /// user-authored position from a simultaneous programmatic size animation.
    pub fn move_start(
        app: &AppHandle,
        overlay_state: &Arc<std::sync::Mutex<OverlayState>>,
    ) -> bool {
        let (mut state, observed, _, _) =
            lock_overlay_after_native_geometry_read(app, overlay_state);
        let Some(observed) = observed else {
            return false;
        };
        state.native_drag_start = Some((observed.x, observed.y));
        true
    }

    pub fn move_cancel(state: &Arc<std::sync::Mutex<OverlayState>>) {
        state.lock().unwrap().native_drag_start = None;
    }

    /// Begins an overlay resize drag. `region` is one of topLeft/top/topRight/
    /// left/right/bottomLeft/bottom/bottomRight; `x`/`y` are the pointer
    /// position in the WebView's screen coordinates. Windows replaces those
    /// values with the native physical cursor position.
    pub fn resize_start(
        app: &AppHandle,
        overlay_state: &Arc<std::sync::Mutex<OverlayState>>,
        region_name: &str,
        x: f64,
        y: f64,
    ) -> Result<(), String> {
        let region = ResizeRegion::from_name(region_name)
            .ok_or_else(|| format!("unknown resize region: {region_name}"))?;
        let Some(window) = app.get_webview_window("overlay") else {
            return Ok(());
        };
        let work_area = current_work_area_logical(app, &window);
        let pointer = resize_pointer_position((x, y));
        let (mut state, observed_frame, _, _) =
            lock_overlay_after_native_geometry_read(app, overlay_state);
        // Resizing is meaningless (and corrupting) in any transient mode.
        if state.mode != OverlayMode::Expanded || state.session_error {
            return Ok(());
        }
        state.native_drag_start = None;
        // A resize gesture is explicit user intent. Adopt the currently
        // presented frame before calculating resize anchors, then let the
        // existing resize path persist its final result.
        state.presentation_frame = None;
        if let Some(observed_frame) = observed_frame {
            state.user_frame = observed_frame;
            state.user_frame.height = state.user_frame.height.max(state.minimum_height);
        }
        state.resize_drag = Some(region);
        state.resize_start = Some(ResizeGesture {
            pointer,
            frame: state.user_frame,
            work_area,
        });
        tracing::info!(
            "resize start region={region:?} frame={:?}",
            (
                state.user_frame.x,
                state.user_frame.y,
                state.user_frame.width,
                state.user_frame.height
            )
        );
        Ok(())
    }

    /// Continues a resize drag with the current pointer position. The dragged
    /// edge/corner stays anchored, sizes clamp to min/max, and the frame keeps
    /// at least 48 logical px on screen.
    pub fn resize_move(
        app: &AppHandle,
        overlay_state: &Arc<std::sync::Mutex<OverlayState>>,
        x: f64,
        y: f64,
    ) {
        if app.get_webview_window("overlay").is_none() {
            return;
        }
        let mut state = overlay_state.lock().unwrap();
        if state.mode != OverlayMode::Expanded || state.session_error {
            return;
        }
        let Some(region) = state.resize_drag else {
            return;
        };
        let Some(start) = state.resize_start else {
            return;
        };
        let frame = resized_frame_for_pointer(
            region,
            start,
            resize_pointer_position((x, y)),
            state.minimum_height,
        );
        // Skip sub-pixel jitter: unchanged frames do not need window ops.
        if (frame.x - state.user_frame.x).abs() < 0.5
            && (frame.y - state.user_frame.y).abs() < 0.5
            && (frame.width - state.user_frame.width).abs() < 0.5
            && (frame.height - state.user_frame.height).abs() < 0.5
        {
            return;
        }
        state.user_frame = frame;
        let log_now = std::time::Instant::now();
        let due = state
            .resize_log_at
            .is_none_or(|at| log_now.duration_since(at).as_millis() >= 250);
        if due {
            state.resize_log_at = Some(log_now);
            tracing::info!(
                "resize move frame={:?}",
                (frame.x, frame.y, frame.width, frame.height)
            );
        }
        let expected_state = OverlayApplySnapshot::from(&*state);
        drop(state);
        Self::apply_resize_if_geometry_current(app, overlay_state, expected_state);
    }

    /// Ends a resize drag and persists the final frame.
    pub fn resize_end(
        app: &AppHandle,
        overlay_state: &Arc<std::sync::Mutex<OverlayState>>,
        settings: &SettingsStore,
    ) {
        let (work_areas, primary_work_area) = available_work_areas(app);
        let mut state = overlay_state.lock().unwrap();
        let Some(frame_to_persist) = finish_resize_drag(&mut state, &work_areas, primary_work_area)
        else {
            return;
        };
        let expected_state = OverlayApplySnapshot::from(&*state);
        drop(state);
        persist_user_frame_if_current(overlay_state, settings, frame_to_persist);
        Self::apply_if_geometry_current(app, overlay_state, expected_state, false);
        tracing::info!("resize end");
    }
}

fn finish_resize_drag(
    state: &mut OverlayState,
    work_areas: &[LogicalWorkArea],
    primary_work_area: Option<LogicalWorkArea>,
) -> Option<OverlayFrame> {
    // Error presentation already cancels its resize owner. A later WebView
    // cleanup must not replace the recovered Space frame or a native move.
    state.resize_drag?;
    state.resize_drag = None;
    state.resize_start = None;
    if state.mode != OverlayMode::Expanded || state.session_error {
        return None;
    }
    state.presentation_frame = None;
    state.native_drag_start = None;
    fit_user_frame_to_work_areas(
        &mut state.user_frame,
        work_areas,
        primary_work_area,
        state.minimum_height,
    );
    Some(state.user_frame)
}

/// WebView `screenX`/`screenY` are not a stable global coordinate space on a
/// mixed-DPI Windows desktop. Read the native cursor position there; retain
/// the existing logical event coordinates on other platforms.
fn resize_pointer_position(fallback: (f64, f64)) -> (f64, f64) {
    #[cfg(target_os = "windows")]
    {
        use ::windows::Win32::Foundation::POINT;
        use ::windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

        let mut point = POINT::default();
        if unsafe { GetCursorPos(&mut point) }.is_ok() {
            return (point.x as f64, point.y as f64);
        }
    }
    fallback
}

/// Applies a resize entirely in the work area's desktop coordinate space,
/// then converts only the resulting dimensions back to logical pixels.
fn resized_frame_for_pointer(
    region: ResizeRegion,
    start: ResizeGesture,
    pointer: (f64, f64),
    minimum_height: f64,
) -> OverlayFrame {
    let work_area = start.work_area;
    let scale = work_area.coordinate_scale;
    let local_start_frame = OverlayFrame {
        x: start.frame.x - work_area.x,
        y: start.frame.y - work_area.y,
        width: start.frame.width * scale,
        height: start.frame.height * scale,
    };
    let mut frame = apply_drag(
        region,
        (start.pointer.0 - work_area.x, start.pointer.1 - work_area.y),
        &local_start_frame,
        (pointer.0 - work_area.x, pointer.1 - work_area.y),
        (
            SubtitleOverlayMetrics::MINIMUM_WIDTH * scale,
            minimum_height * scale,
        ),
        (
            SubtitleOverlayMetrics::MAXIMUM_WIDTH * scale,
            SubtitleOverlayMetrics::MAXIMUM_HEIGHT * scale,
        ),
        (work_area.coordinate_width(), work_area.coordinate_height()),
        48.0 * scale,
    );
    frame.x += work_area.x;
    frame.y += work_area.y;
    frame.width /= scale;
    frame.height /= scale;
    frame
}

/// Reads native geometry without holding the overlay mutex, then returns only
/// when the state used to interpret that read is still current. This keeps
/// Tokio command paths from waiting on the main event loop while main is
/// waiting for the same mutex.
fn lock_overlay_after_native_geometry_read<'a>(
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

fn presentation_frame_matches_observed(state: &OverlayState, observed: &OverlayFrame) -> bool {
    let Some(presentation) = state.presentation_frame else {
        return false;
    };
    let geometry = geometry_for(state.mode, &presentation, state.minimum_height);
    let expected = OverlayFrame {
        x: geometry.x,
        y: geometry.y,
        width: geometry.width,
        height: geometry.height,
    };
    frames_approximately_equal(&expected, observed)
}

/// Returns `true` when `observed` is user-authored geometry and promotes it to
/// the canonical frame. A frame that exactly matches the active-Space
/// presentation remains a runtime override and must not reach preferences.
fn adopt_observed_frame(state: &mut OverlayState, observed: OverlayFrame) -> bool {
    if state.session_error {
        if !state
            .native_drag_start
            .is_some_and(|start| native_drag_changed_position(start, &observed))
        {
            return false;
        }
        promote_observed_user_frame(state, observed);
        return true;
    }
    if presentation_frame_matches_observed(state, &observed) {
        if state
            .presentation_frame
            .is_some_and(|frame| frames_approximately_equal(&frame, &state.user_frame))
        {
            // A return-to-saved-display move needs one matching event to
            // suppress persistence, then no longer needs an override.
            state.presentation_frame = None;
        }
        return false;
    }
    promote_observed_user_frame(state, observed);
    true
}

fn promote_observed_user_frame(state: &mut OverlayState, observed: OverlayFrame) {
    if state.session_error {
        if state
            .native_drag_start
            .is_some_and(|start| native_drag_changed_position(start, &observed))
        {
            // Only explicit native movement can replace this temporary
            // presentation. Retain the user's normal expanded dimensions.
            state.user_frame.x = observed.x;
            state.user_frame.y = observed.y;
            state.presentation_frame = None;
            // Native dragging can pause longer than the debounce and then
            // continue. Retain its intent until a programmatic transition;
            // matching events never count as another placement.
            state.native_drag_start = Some((observed.x, observed.y));
        }
        return;
    }
    // start_dragging only moves the window; custom resize gestures own size.
    // Its observed dimensions may still belong to an expansion animation.
    let native_move = state.native_drag_start.take().is_some();
    state.presentation_frame = None;
    match state.mode {
        OverlayMode::Expanded if !native_move => {
            state.user_frame = observed;
            state.user_frame.height = state.user_frame.height.max(state.minimum_height);
        }
        OverlayMode::Expanded | OverlayMode::Collapsed => {
            state.user_frame.x = observed.x;
            state.user_frame.y = observed.y;
        }
    }
}

/// Reconciles an outstanding native drag before another state transition can
/// overwrite its frame. Returns whether fitting the promoted frame requires a
/// corrective OS geometry write; promotion itself is persisted immediately.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct NativeDragReconcile {
    promoted: bool,
    fit_changed: bool,
}

#[derive(Debug)]
struct OverlayModeTransition {
    mode_changed: bool,
    geometry_changed: bool,
    frame_to_persist: Option<OverlayFrame>,
}

/// Native observations reconcile explicit movement only. An unmarked size can
/// be an intermediate animation frame and must never replace the expanded
/// target when the user reverses a transition.
fn transition_overlay_mode(
    state: &mut OverlayState,
    observed: Option<OverlayFrame>,
    work_areas: &[LogicalWorkArea],
    primary_work_area: Option<LogicalWorkArea>,
    collapsed: bool,
) -> OverlayModeTransition {
    let manual_drag = reconcile_native_drag(state, observed, work_areas, primary_work_area);
    let frame_to_persist = manual_drag.promoted.then_some(state.user_frame);
    let new_mode = if collapsed {
        OverlayMode::Collapsed
    } else {
        OverlayMode::Expanded
    };
    let mode_changed = state.mode != new_mode;
    if mode_changed {
        if !collapsed && !state.session_error {
            // The screen configuration may have changed while collapsed;
            // keep the expanded frame on the visible screen.
            let minimum_height = state.minimum_height;
            if let Some(frame) = state.presentation_frame.as_mut() {
                fit_user_frame_to_work_areas(frame, work_areas, primary_work_area, minimum_height);
            } else {
                fit_user_frame_to_work_areas(
                    &mut state.user_frame,
                    work_areas,
                    primary_work_area,
                    minimum_height,
                );
            }
        }
        state.mode = new_mode;
        state.mode_revision = state.mode_revision.wrapping_add(1);
        if state.session_error {
            state.native_drag_start = None;
        }
    }
    OverlayModeTransition {
        mode_changed,
        geometry_changed: mode_changed || manual_drag.fit_changed,
        frame_to_persist,
    }
}

fn reconcile_native_drag(
    state: &mut OverlayState,
    observed: Option<OverlayFrame>,
    work_areas: &[LogicalWorkArea],
    primary_work_area: Option<LogicalWorkArea>,
) -> NativeDragReconcile {
    let Some(start) = state.native_drag_start else {
        return NativeDragReconcile::default();
    };
    let Some(observed) = observed else {
        return NativeDragReconcile::default();
    };
    if !native_drag_changed_position(start, &observed) {
        if !state.session_error {
            state.native_drag_start = None;
        }
        return NativeDragReconcile::default();
    }
    promote_observed_user_frame(state, observed);
    let fit_changed = fit_promoted_user_frame(state, &observed, work_areas, primary_work_area);
    NativeDragReconcile {
        promoted: true,
        fit_changed,
    }
}

/// Fit after confirmed user movement. Error presentation may need a larger
/// temporary height or a smaller screen fit, neither of which is a user resize.
fn fit_promoted_user_frame(
    state: &mut OverlayState,
    observed: &OverlayFrame,
    work_areas: &[LogicalWorkArea],
    primary_work_area: Option<LogicalWorkArea>,
) -> bool {
    if state.session_error {
        let mut shown = state.effective_frame();
        fit_user_frame_to_work_areas(
            &mut shown,
            work_areas,
            primary_work_area,
            state.effective_minimum_height(),
        );
        state.user_frame.x = shown.x;
        state.user_frame.y = shown.y;
        state.native_drag_start = Some((shown.x, shown.y));
        return !frames_approximately_equal(&shown, observed);
    }
    let minimum_height = state.minimum_height;
    match state.mode {
        OverlayMode::Expanded => {
            fit_user_frame_to_work_areas(
                &mut state.user_frame,
                work_areas,
                primary_work_area,
                minimum_height,
            ) || !frames_approximately_equal(&state.user_frame, observed)
        }
        OverlayMode::Collapsed => fit_collapsed_position_to_work_areas(
            &mut state.user_frame,
            work_areas,
            primary_work_area,
        ),
    }
}

fn positions_approximately_equal(left: (f64, f64), right: (f64, f64)) -> bool {
    (left.0 - right.0).abs() < 1.0 && (left.1 - right.1).abs() < 1.0
}

fn native_drag_changed_position(start: (f64, f64), observed: &OverlayFrame) -> bool {
    !positions_approximately_equal(start, (observed.x, observed.y))
}

fn frames_approximately_equal(left: &OverlayFrame, right: &OverlayFrame) -> bool {
    (left.x - right.x).abs() < 1.0
        && (left.y - right.y).abs() < 1.0
        && (left.width - right.width).abs() < 1.0
        && (left.height - right.height).abs() < 1.0
}

#[cfg(any(target_os = "macos", test))]
fn active_space_follow_allowed(state: &OverlayState) -> bool {
    state.resize_drag.is_none() && !state.session_error
}

#[cfg(any(target_os = "macos", test))]
fn update_presentation_frame(state: &mut OverlayState, next: OverlayFrame) -> bool {
    let changed = !frames_approximately_equal(&state.effective_frame(), &next);
    if !changed && frames_approximately_equal(&next, &state.user_frame) {
        state.presentation_frame = None;
    } else {
        state.presentation_frame = Some(next);
    }
    changed
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
fn persist_user_frame_if_current(
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

#[cfg(test)]
mod tests;
