//! Overlay, child-control, settings, and tray-panel window management.
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

/// True when this is a dev build: a debug build (`tauri::is_dev`) or the
/// bundled dev wrapper (bundle id `app.yuxino.mimi.dev`, which is a release
/// build so macOS renders its Dock icon with the standard mask).
pub fn is_dev_build() -> bool {
    if tauri::is_dev() {
        return true;
    }
    #[cfg(target_os = "macos")]
    {
        use objc2::msg_send;
        use objc2::runtime::{AnyClass, AnyObject};
        use std::ffi::{c_char, CStr};
        unsafe {
            let Some(bundle_class) = AnyClass::get(c"NSBundle") else {
                return false;
            };
            let main_bundle: *mut AnyObject = msg_send![bundle_class, mainBundle];
            if main_bundle.is_null() {
                return false;
            }
            let identifier: *mut AnyObject = msg_send![main_bundle, bundleIdentifier];
            if identifier.is_null() {
                return false;
            }
            let ptr: *const c_char = msg_send![identifier, UTF8String];
            if ptr.is_null() {
                return false;
            }
            CStr::from_ptr(ptr).to_str() == Ok("app.yuxino.mimi.dev")
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Window and tooltip title for the current build: dev builds get a "(dev)"
/// marker so they can be told apart from the installed release app at a
/// glance.
pub fn dev_title(base: &str) -> String {
    if is_dev_build() {
        format!("{base} (dev)")
    } else {
        base.to_string()
    }
}

use crate::core::overlay_layout::{minimum_overlay_height, BASE_MINIMUM_HEIGHT};
use crate::pipeline_log;
use crate::settings_store::{OverlayFrame, Preferences, SettingsStore};
use crate::windows::resize::{apply_drag, ResizeRegion};
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

#[cfg(target_os = "linux")]
mod linux_input_region;

#[cfg(target_os = "macos")]
mod macos_control_dismiss;

#[cfg(target_os = "macos")]
mod macos_pointer;

#[cfg(target_os = "macos")]
pub fn remove_overlay_pointer_tracking(app: &AppHandle) {
    macos_pointer::clear(app, true);
}

#[cfg(target_os = "macos")]
pub fn clear_overlay_pointer_hover(app: &AppHandle) {
    macos_pointer::clear(app, false);
}

#[cfg(target_os = "macos")]
pub async fn set_overlay_pointer_cursor(
    app: &AppHandle,
    point: crate::core::overlay_pointer::OverlayPointerPosition,
    pointing: bool,
) -> Result<bool, String> {
    macos_pointer::set_pointer_cursor(app, point, pointing).await
}

#[cfg(target_os = "windows")]
mod windows_workspace;

#[cfg(target_os = "windows")]
pub use windows_workspace::WindowsWorkspaceFollower;

#[cfg(target_os = "windows")]
pub fn install_windows_workspace_follower(app: &AppHandle) -> WindowsWorkspaceFollower {
    windows_workspace::install(app)
}

#[cfg(target_os = "macos")]
tauri_nspanel::tauri_panel! {
    panel!(SubtitleOverlayPanel {
        config: {
            // Permit keyboard reading only after a click needs the WebView's
            // responder. NonactivatingPanel and orderFrontRegardless still
            // show/refresh subtitles without activating Mimi or stealing key.
            can_become_key_window: true,
            can_become_main_window: false,
            is_floating_panel: true,
            hides_on_deactivate: false,
            becomes_key_only_if_needed: true
        }
    })

    panel!(SubtitleControlPanel {
        config: {
            can_become_key_window: true,
            can_become_main_window: false,
            is_floating_panel: true,
            hides_on_deactivate: false,
            // HTML inputs live inside WKWebView, not an AppKit text field
            // whose hit view opts into needsPanelToBecomeKey. Let an explicit
            // click give this control panel keyboard input. NonactivatingPanel
            // and passive ordering still leave the media application active.
            becomes_key_only_if_needed: false
        }
    })
}

#[derive(Clone, Copy)]
enum OverlayPanelKind {
    Subtitles,
    Controls,
}

#[cfg(target_os = "macos")]
fn convert_overlay_to_panel(
    window: &tauri::WebviewWindow,
    kind: OverlayPanelKind,
) -> Result<(), ()> {
    use tauri_nspanel::WebviewWindowExt;

    let result = match kind {
        OverlayPanelKind::Subtitles => window.to_panel::<SubtitleOverlayPanel>(),
        OverlayPanelKind::Controls => window.to_panel::<SubtitleControlPanel>(),
    };
    result.map(|_| ()).map_err(|_| ())
}

#[cfg(not(target_os = "macos"))]
fn convert_overlay_to_panel(
    _window: &tauri::WebviewWindow,
    _kind: OverlayPanelKind,
) -> Result<(), ()> {
    Ok(())
}

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
    pub const REFERENCE_WIDTH: f64 = 659.0;
    pub const REFERENCE_HEIGHT: f64 = 328.0;
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
    /// Current subtitle requirement; part of every queued geometry transaction.
    minimum_height: f64,
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

/// The lightweight control surface that floats above the subtitle canvas.
/// It reuses the former language-popover WebView instead of creating another
/// renderer: the same child/owned window is either a compact status island or
/// an expanded control panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OverlayControlMode {
    Hidden,
    Island,
    Panel,
}

#[derive(Debug)]
struct OverlayControlStateInner {
    mode: OverlayControlMode,
    island_width: f64,
    panel_height: f64,
    generation: u64,
}

/// Tauri-managed state for the overlay control child window. Keeping this
/// separate from [`OverlayState`] ensures transient panel resizing can never
/// mutate or persist the user's subtitle frame.
#[derive(Debug)]
pub struct OverlayControlState(std::sync::Mutex<OverlayControlStateInner>);

/// Cached native presentation state. Session snapshots can arrive many times
/// per second while subtitles stream; the cache keeps identical snapshots
/// from repeatedly crossing into the OS window API.
#[cfg(not(target_os = "linux"))]
#[derive(Debug, Default)]
pub struct OverlayPresentationState(std::sync::Mutex<Option<bool>>);

#[cfg(target_os = "linux")]
#[derive(Debug, Default)]
pub struct OverlayPresentationState(std::sync::Mutex<linux_input_region::InputRegionState>);

impl OverlayPresentationState {
    #[cfg(not(target_os = "linux"))]
    fn apply_click_through_if_changed(&self, window: &tauri::WebviewWindow, enabled: bool) {
        let mut current = self.0.lock().unwrap();
        if *current == Some(enabled) {
            return;
        }
        if window.set_ignore_cursor_events(enabled).is_ok() {
            *current = Some(enabled);
            #[cfg(target_os = "macos")]
            if enabled {
                macos_pointer::clear(window.app_handle(), false);
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn apply_click_through_if_changed(&self, window: &tauri::WebviewWindow, enabled: bool) {
        // Record intent before dispatch, without holding the mutex while GTK
        // waits for the main loop. Older queued requests read the latest intent
        // instead of re-locking a window after a newer explicit unlock.
        self.0.lock().unwrap().request(enabled);
        let app = window.app_handle().clone();
        let label = window.label().to_string();
        if window
            .run_on_main_thread(move || {
                // Resolve the current instance: a queued request may outlive
                // the native window it was originally sent through.
                let Some(window) = app.get_webview_window(&label) else {
                    return;
                };
                let Ok(native) = window.gtk_window() else {
                    return;
                };
                if let Some(presentation) = app.try_state::<OverlayPresentationState>() {
                    presentation.0.lock().unwrap().reconcile(false, |enabled| {
                        linux_input_region::apply(&native, enabled);
                    });
                }
            })
            .is_err()
        {
            pipeline_log!("overlay input region failed label=main_thread_unavailable");
        }
    }

    fn invalidate(&self) {
        #[cfg(not(target_os = "linux"))]
        {
            *self.0.lock().unwrap() = None;
        }
        #[cfg(target_os = "linux")]
        self.0.lock().unwrap().invalidate();
    }
}

impl Default for OverlayControlState {
    fn default() -> Self {
        Self(std::sync::Mutex::new(OverlayControlStateInner {
            mode: OverlayControlMode::Hidden,
            island_width: OverlayControlWindowManager::DEFAULT_ISLAND_WIDTH,
            panel_height: OverlayControlWindowManager::DEFAULT_PANEL_HEIGHT,
            generation: 0,
        }))
    }
}

impl OverlayControlState {
    pub fn mode(&self) -> OverlayControlMode {
        self.0.lock().unwrap().mode
    }

    fn snapshot(&self) -> (OverlayControlMode, f64, u64) {
        let state = self.0.lock().unwrap();
        (state.mode, state.panel_height, state.generation)
    }

    fn set_mode(&self, mode: OverlayControlMode) -> bool {
        let mut state = self.0.lock().unwrap();
        if state.mode == mode {
            return false;
        }
        state.mode = mode;
        state.generation = state.generation.wrapping_add(1);
        true
    }

    fn set_panel_height(&self, height: f64) -> bool {
        let mut state = self.0.lock().unwrap();
        if (state.panel_height - height).abs() < 0.5 {
            return false;
        }
        state.panel_height = height;
        true
    }

    fn dimensions(&self) -> (f64, f64) {
        let state = self.0.lock().unwrap();
        (state.island_width, state.panel_height)
    }

    fn set_island_width(&self, width: f64) -> bool {
        let mut state = self.0.lock().unwrap();
        if (state.island_width - width).abs() < 0.5 {
            return false;
        }
        state.island_width = width;
        true
    }

    fn cancel_scheduled_dismiss(&self) {
        let mut state = self.0.lock().unwrap();
        state.generation = state.generation.wrapping_add(1);
    }
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
            .map_or(SubtitleOverlayMetrics::REFERENCE_WIDTH, |f| f.width);
        let height = trusted
            .as_ref()
            .map_or(SubtitleOverlayMetrics::REFERENCE_HEIGHT, |f| f.height)
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
            minimum_height,
            user_frame,
            presentation_frame: None,
            native_drag_start: None,
            resize_drag: None,
            resize_start: None,
            last_geometry_event: None,
            geometry_task_pending: false,
            resize_log_at: None,
        }
    }

    fn effective_frame(&self) -> OverlayFrame {
        self.presentation_frame.unwrap_or(self.user_frame)
    }

    fn update_minimum_height(&mut self, minimum_height: f64) -> bool {
        if self.minimum_height == minimum_height {
            return false;
        }
        self.minimum_height = minimum_height;
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
        let expected = {
            let mut state = overlay_state.lock().unwrap();
            let minimum = minimum_height_for_preferences(&settings.preferences());
            if !state.update_minimum_height(minimum) {
                return;
            }
            OverlayApplySnapshot::from(&*state)
        };
        Self::apply_if_geometry_current(app, overlay_state, expected, false);
    }

    pub fn ensure_overlay(app: &AppHandle, state: &Arc<std::sync::Mutex<OverlayState>>) {
        if app.get_webview_window("overlay").is_some() {
            return;
        }
        let (frame, minimum_height) = {
            let state = state.lock().unwrap();
            (state.user_frame, state.minimum_height)
        };
        let builder =
            WebviewWindowBuilder::new(app, "overlay", WebviewUrl::App("index.html".into()))
                .title(dev_title("mimi Subtitles"))
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
                .resizable(true)
                .visible(false);
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
                #[cfg(target_os = "macos")]
                macos_pointer::install(&window);
                #[cfg(target_os = "linux")]
                follow_linux_overlay_on_map(&window);
                pipeline_log!("overlay window created");
            }
            Err(_) => pipeline_log!("overlay window failed label=create_failed"),
        }
    }

    /// Applies the complete native overlay presentation in one place:
    /// visibility, click-through behavior, and the child control surface.
    /// The control island is visible only for an active, expanded,
    /// non-immersive overlay; an already-open panel remains open across
    /// ordinary state broadcasts.
    pub fn sync_presentation(
        app: &AppHandle,
        is_active: bool,
        is_collapsed: bool,
        click_through: bool,
        is_immersive: bool,
    ) {
        if is_active {
            if click_through {
                // Ordinary position locking keeps the independent control
                // island as an unlock escape hatch. Immersive Mode hides all
                // overlay chrome and is exited via shortcut, tray, or settings.
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
                    is_immersive,
                );
                #[cfg(not(target_os = "linux"))]
                Self::update_locked(app, true);
            } else {
                // Restore canvas interaction before changing either visible
                // surface, so unlocking never leaves an inert frame behind.
                Self::update_locked(app, false);
                Self::sync_overlay_visibility(app, true);
                OverlayControlWindowManager::sync_presentation(app, true, is_collapsed, false);
            }
        } else {
            // Hide the child first so it cannot linger for a frame after its
            // subtitle parent disappears.
            OverlayControlWindowManager::sync_presentation(app, false, is_collapsed, is_immersive);
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
        } else if !is_active && visible {
            #[cfg(target_os = "macos")]
            macos_pointer::clear(app, false);
            let _ = window.hide();
        }
    }

    /// Restores already-visible overlay surfaces to the active Space after an
    /// explicit show request. This is deliberately separate from ordinary
    /// streaming state broadcasts so subtitle updates never reorder windows or
    /// disturb focus.
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
    pub(super) fn follow_active_space_on_main(
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

    #[cfg(not(target_os = "macos"))]
    pub fn follow_active_space(
        app: &AppHandle,
        _overlay_state: &Arc<std::sync::Mutex<OverlayState>>,
        _settings: &Arc<SettingsStore>,
    ) {
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
                    (state.mode, state.effective_frame(), state.minimum_height)
                };
                let animation = animate.then(|| GeometryAnimationGuard {
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
        let manual_drag =
            reconcile_native_drag(&mut state, observed_frame, &work_areas, primary_work_area);
        let frame_to_persist = manual_drag.promoted.then_some(state.user_frame);
        let new_mode = if collapsed {
            OverlayMode::Collapsed
        } else {
            OverlayMode::Expanded
        };
        if state.mode == new_mode {
            let expected_state = OverlayApplySnapshot::from(&*state);
            drop(state);
            if let Some(frame) = frame_to_persist {
                persist_user_frame_if_current(overlay_state, settings, frame);
            }
            if manual_drag.fit_changed {
                Self::apply_if_geometry_current(app, overlay_state, expected_state, false);
            }
            return;
        }
        if collapsed {
            // Adopt the exact current window frame before shrinking so an
            // expand later restores precisely what the user saw. A temporary
            // active-Space frame is already exact but must not become saved
            // merely because the user collapsed the surface.
            if should_sync_before_collapse(manual_drag, &state) {
                if let Some(observed_frame) = observed_frame {
                    state.user_frame = observed_frame;
                    state.user_frame.height = state.user_frame.height.max(state.minimum_height);
                }
            }
        } else {
            // The screen configuration may have changed while collapsed;
            // keep the expanded frame on the visible screen.
            let minimum_height = state.minimum_height;
            if let Some(frame) = state.presentation_frame.as_mut() {
                fit_user_frame_to_work_areas(frame, &work_areas, primary_work_area, minimum_height);
            } else {
                fit_user_frame_to_work_areas(
                    &mut state.user_frame,
                    &work_areas,
                    primary_work_area,
                    minimum_height,
                );
            }
        }
        state.mode = new_mode;
        let expected_state = OverlayApplySnapshot::from(&*state);
        drop(state);
        if let Some(frame) = frame_to_persist {
            persist_user_frame_if_current(overlay_state, settings, frame);
        }
        Self::apply_if_geometry_current(app, overlay_state, expected_state, true);
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
                let minimum_height = state.minimum_height;
                let geometry_changed = match state.mode {
                    OverlayMode::Expanded => {
                        fit_user_frame_to_work_areas(
                            &mut state.user_frame,
                            &work_areas,
                            primary_work_area,
                            minimum_height,
                        ) || !frames_approximately_equal(&state.user_frame, &observed_frame)
                    }
                    OverlayMode::Collapsed => {
                        // Only the position is meaningful while collapsed; the
                        // remembered expanded size must survive.
                        fit_collapsed_position_to_work_areas(
                            &mut state.user_frame,
                            &work_areas,
                            primary_work_area,
                        )
                    }
                };
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
    pub fn move_start(app: &AppHandle, overlay_state: &Arc<std::sync::Mutex<OverlayState>>) {
        let (mut state, observed, _, _) =
            lock_overlay_after_native_geometry_read(app, overlay_state);
        let Some(observed) = observed else {
            return;
        };
        state.native_drag_start = Some((observed.x, observed.y));
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
        if state.mode != OverlayMode::Expanded {
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
        if state.mode != OverlayMode::Expanded {
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
        state.resize_drag = None;
        state.resize_start = None;
        if state.mode != OverlayMode::Expanded {
            return;
        }
        state.presentation_frame = None;
        state.native_drag_start = None;
        let minimum_height = state.minimum_height;
        fit_user_frame_to_work_areas(
            &mut state.user_frame,
            &work_areas,
            primary_work_area,
            minimum_height,
        );
        let frame_to_persist = state.user_frame;
        let expected_state = OverlayApplySnapshot::from(&*state);
        drop(state);
        persist_user_frame_if_current(overlay_state, settings, frame_to_persist);
        Self::apply_if_geometry_current(app, overlay_state, expected_state, false);
        tracing::info!("resize end");
    }
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

/// Reasserts presentation-level window behavior whenever the overlay is
/// created or shown. macOS needs both `CanJoinAllApplications` and
/// `FullScreenAuxiliary` in addition to Tauri's all-workspaces flag to
/// accompany another app's full-screen window instead of remaining on the
/// previous Space. macOS composites true full-screen video above ordinary
/// floating and status windows, so the subtitle surface uses the screen-saver
/// window level recommended for cross-application media overlays.
fn configure_overlay_window(window: &tauri::WebviewWindow) {
    configure_overlay_window_impl(window, false);
}

/// Orders an overlay onscreen without making it key. Tauri's ordinary macOS
/// `show()` path uses `makeKeyAndOrderFront`, which can activate mimi and pull
/// the user out of the media app's full-screen Space.
fn show_overlay_window(window: &tauri::WebviewWindow) {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window.show();
        configure_overlay_window(window);
    }

    #[cfg(target_os = "macos")]
    configure_overlay_window_impl(window, true);
}

/// Shows the control surface and restores its native parent first. AppKit
/// automatically detaches a child window whenever `orderOut:` hides it, so the
/// relationship created by `WebviewWindowBuilder::parent` is not persistent
/// across inactive, collapsed, or immersive transitions. Reattaching in the
/// same main-thread transaction as `orderFrontRegardless` keeps the control
/// moving synchronously with the subtitle panel during the next drag.
fn show_overlay_control_window(app: &AppHandle, window: &tauri::WebviewWindow) {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        show_overlay_window(window);
    }

    #[cfg(target_os = "macos")]
    {
        let Some(parent) = app.get_webview_window("overlay") else {
            pipeline_log!("overlay control show failed label=missing_parent");
            return;
        };
        configure_overlay_window_impl_macos(window, true, Some(parent));
    }
}

/// Reapplies the native behavior for an explicit user-requested show. AppKit's
/// `isVisible` remains true for an ordered-in window on another Space, so a
/// plain Tauri `show()` would otherwise be skipped without restoring it to the
/// active full-screen Space.
fn reassert_overlay_window_on_active_space(window: &tauri::WebviewWindow) {
    configure_overlay_window_impl(window, true);
}

/// Only an explicit request to open controls grants keyboard input. On macOS,
/// use AppKit's key-window operation directly: Tao's `set_focus` also activates
/// the application and can pull the media app out of its full-screen Space.
/// A WKWebView click alone does not reliably make a nonactivating panel key.
fn focus_overlay_control(window: &tauri::WebviewWindow) {
    #[cfg(not(target_os = "macos"))]
    let _ = window.set_focus();

    #[cfg(target_os = "macos")]
    {
        let current_window = window.clone();
        let _ = window.with_webview(move |webview| {
            use objc2_app_kit::{NSView, NSWindow};

            if OverlayControlWindowManager::mode(current_window.app_handle())
                != OverlayControlMode::Panel
            {
                return;
            }
            let Ok(pointer) = current_window.ns_window() else {
                return;
            };
            let view_pointer = webview.inner();
            if view_pointer.is_null() {
                return;
            }
            // SAFETY: with_webview runs on the main thread and keeps the
            // app-owned WKWebView/window alive for this callback. contentView
            // is a Wry wrapper, so use the actual webview for keyboard input.
            let panel: &NSWindow = unsafe { &*pointer.cast() };
            let view: &NSView = unsafe { &*view_pointer.cast() };
            if !panel.isVisible() {
                return;
            }
            let responder_accepted = panel.makeFirstResponder(Some(view));
            panel.makeKeyWindow();
            pipeline_log!(
                "overlay control keyboard key={} main={} responder_accepted={}",
                panel.isKeyWindow(),
                panel.isMainWindow(),
                responder_accepted
            );
        });
    }
}

fn configure_overlay_window_impl(window: &tauri::WebviewWindow, order_front: bool) {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = order_front;
        let _ = window.set_always_on_top(true);
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = window.set_visible_on_all_workspaces(true);
    }

    #[cfg(target_os = "macos")]
    configure_overlay_window_impl_macos(window, order_front, None);
}

#[cfg(target_os = "macos")]
fn configure_overlay_window_impl_macos(
    window: &tauri::WebviewWindow,
    order_front: bool,
    parent: Option<tauri::WebviewWindow>,
) {
    // Session transitions may call this function from a tokio worker. Raw
    // AppKit access is main-thread-only and macOS traps immediately if native
    // window state is mutated from that worker, so presentation, optional
    // parent restoration, and ordering happen in one dispatched transaction.
    let window_for_main = window.clone();
    let _ = window.run_on_main_thread(move || unsafe {
        use objc2_app_kit::{NSWindow, NSWindowStyleMask};

        let Ok(pointer) = window_for_main.ns_window() else {
            return;
        };
        let ns_window: &NSWindow = &*pointer.cast();
        // `to_panel` changes the native class while retaining Tauri's existing
        // borderless/resizable style bits. Add the one panel style required to
        // avoid activating mimi over another app's full-screen video.
        ns_window.setStyleMask(ns_window.styleMask() | NSWindowStyleMask::NonactivatingPanel);
        let behavior = overlay_collection_behavior(ns_window.collectionBehavior());
        ns_window.setCollectionBehavior(behavior);
        ns_window.setHidesOnDeactivate(false);
        ns_window.setLevel(overlay_window_level());

        if let Some(parent_window) = parent {
            match parent_window.ns_window() {
                Ok(parent_pointer) => {
                    let parent_ns_window: &NSWindow = &*parent_pointer.cast();
                    attach_overlay_control_parent_macos(ns_window, parent_ns_window);
                }
                Err(_) => pipeline_log!("overlay control show failed label=parent_unavailable"),
            }
        }

        if order_front {
            ns_window.orderFrontRegardless();
        }
    });
}

/// Main-thread-only parent restoration shared by initial presentation and
/// keyboard release. AppKit detaches a child when orderOut hides it.
#[cfg(target_os = "macos")]
fn attach_overlay_control_parent_macos(
    window: &objc2_app_kit::NSWindow,
    parent: &objc2_app_kit::NSWindow,
) {
    use objc2_app_kit::NSWindowOrderingMode;

    let current_parent = window.parentWindow();
    if current_parent
        .as_deref()
        .is_some_and(|current| std::ptr::eq(current, parent))
    {
        return;
    }
    if let Some(current) = current_parent {
        current.removeChildWindow(window);
    }
    // SAFETY: Both are distinct app-owned windows, accessed on the main thread;
    // the child is detached from any previous parent before attaching above.
    unsafe { parent.addChildWindow_ordered(window, NSWindowOrderingMode::Above) };
}

#[cfg(target_os = "macos")]
fn overlay_window_level() -> objc2_app_kit::NSWindowLevel {
    objc2_app_kit::NSScreenSaverWindowLevel
}

#[cfg(target_os = "macos")]
fn overlay_collection_behavior(
    current: objc2_app_kit::NSWindowCollectionBehavior,
) -> objc2_app_kit::NSWindowCollectionBehavior {
    use objc2_app_kit::NSWindowCollectionBehavior;

    // AppKit permits only one member from each of these behavior groups.
    // Remove incompatible defaults before declaring the overlay as eligible
    // for other applications' full-screen Spaces.
    let incompatible = NSWindowCollectionBehavior::MoveToActiveSpace
        | NSWindowCollectionBehavior::Managed
        | NSWindowCollectionBehavior::Transient
        | NSWindowCollectionBehavior::ParticipatesInCycle
        | NSWindowCollectionBehavior::Primary
        | NSWindowCollectionBehavior::Auxiliary
        | NSWindowCollectionBehavior::FullScreenPrimary
        | NSWindowCollectionBehavior::FullScreenNone;
    (current & !incompatible)
        | NSWindowCollectionBehavior::CanJoinAllSpaces
        | NSWindowCollectionBehavior::Stationary
        | NSWindowCollectionBehavior::IgnoresCycle
        | NSWindowCollectionBehavior::CanJoinAllApplications
        | NSWindowCollectionBehavior::FullScreenAuxiliary
}

// ---------------------------------------------------------------------------
// Geometry helpers
// ---------------------------------------------------------------------------

/// Logical work area of the monitor the overlay currently sits on. The
/// desktop-space origin is preserved, including negative coordinates for
/// monitors positioned to the left of or above the primary display.
fn current_work_area_logical(app: &AppHandle, window: &tauri::WebviewWindow) -> LogicalWorkArea {
    window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| app.primary_monitor().ok().flatten())
        .map(|monitor| logical_work_area(&monitor))
        .unwrap_or(LogicalWorkArea {
            x: 0.0,
            y: 0.0,
            width: 1440.0,
            height: 900.0,
            coordinate_scale: 1.0,
        })
}

/// Sets a native desktop origin without ever combining global coordinates
/// from monitors that use different DPI scales. Window dimensions remain in
/// logical pixels so the overlay keeps the same visual size on every screen.
fn set_desktop_position(window: &tauri::WebviewWindow, x: f64, y: f64) {
    #[cfg(target_os = "windows")]
    let _ = window.set_position(tauri::PhysicalPosition::new(
        x.round() as i32,
        y.round() as i32,
    ));

    #[cfg(not(target_os = "windows"))]
    let _ = window.set_position(tauri::LogicalPosition::new(x, y));
}

fn set_desktop_size(window: &tauri::WebviewWindow, width: f64, height: f64, coordinate_scale: f64) {
    #[cfg(target_os = "windows")]
    let _ = window.set_size(tauri::PhysicalSize::new(
        (width * coordinate_scale).round() as u32,
        (height * coordinate_scale).round() as u32,
    ));

    #[cfg(not(target_os = "windows"))]
    {
        let _ = coordinate_scale;
        let _ = window.set_size(tauri::LogicalSize::new(width, height));
    }
}

fn desktop_scale_for_frame(
    app: &AppHandle,
    window: &tauri::WebviewWindow,
    frame: &OverlayFrame,
) -> f64 {
    #[cfg(target_os = "windows")]
    if let Some(area) = work_area_for_saved_frame(app, frame) {
        return area.coordinate_scale;
    }

    let _ = (app, frame);
    window.scale_factor().unwrap_or(1.0).max(1.0)
}

#[cfg(not(target_os = "linux"))]
fn overlay_frame_from_window(app: &AppHandle) -> Option<OverlayFrame> {
    let window = app.get_webview_window("overlay")?;
    let scale = window.scale_factor().unwrap_or(1.0).max(1.0);
    let (size, position) = (window.inner_size().ok()?, window.outer_position().ok()?);
    #[cfg(target_os = "windows")]
    let (x, y) = (position.x as f64, position.y as f64);
    #[cfg(not(target_os = "windows"))]
    let (x, y) = (position.x as f64 / scale, position.y as f64 / scale);
    Some(OverlayFrame {
        x,
        y,
        width: size.width as f64 / scale,
        height: size.height as f64 / scale,
    })
}

/// Tao's cached ConfigureNotify geometry can still be (0, 0) when a hidden
/// GTK window is first mapped. Read the mapped native window on GTK's thread;
/// never persist the hidden placeholder or anchor controls to that cache.
#[cfg(target_os = "linux")]
fn overlay_frame_from_window(app: &AppHandle) -> Option<OverlayFrame> {
    use gtk::prelude::*;
    let window = app.get_webview_window("overlay")?;
    let native_window = window.clone();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    window
        .run_on_main_thread(move || {
            let frame = native_window.gtk_window().ok().and_then(|window| {
                if !window.is_mapped() {
                    return None;
                }
                let (x, y) = window.position();
                let (width, height) = window.size();
                Some(OverlayFrame {
                    x: x as f64,
                    y: y as f64,
                    width: width as f64,
                    height: height as f64,
                })
            });
            let _ = tx.send(frame);
        })
        .ok()?;
    rx.recv().ok().flatten()
}

#[cfg(target_os = "linux")]
fn follow_linux_overlay_on_map(window: &tauri::WebviewWindow) {
    use gtk::prelude::*;
    let native_window = window.clone();
    let app = window.app_handle().clone();
    let _ = window.run_on_main_thread(move || {
        if let Ok(window) = native_window.gtk_window() {
            let input_app = app.clone();
            linux_input_region::restore_on_surface_change(&window, move |window| {
                if let Some(presentation) = input_app.try_state::<OverlayPresentationState>() {
                    presentation.0.lock().unwrap().reconcile(true, |enabled| {
                        linux_input_region::apply(window, enabled);
                    });
                }
            });
            window.connect_map_event(move |_, _| {
                // Wait until GTK has processed the mapping event, without a
                // timer or a polling loop. Re-read mode so a stop wins races.
                let app = app.clone();
                gtk::glib::idle_add_local_once(move || {
                    OverlayControlWindowManager::follow_overlay(&app);
                });
                gtk::glib::Propagation::Proceed
            });
        }
    });
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
    state.presentation_frame = None;
    state.native_drag_start = None;
    match state.mode {
        OverlayMode::Expanded => {
            state.user_frame = observed;
            state.user_frame.height = state.user_frame.height.max(state.minimum_height);
        }
        OverlayMode::Collapsed => {
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
    state.native_drag_start = None;
    if !native_drag_changed_position(start, &observed) {
        return NativeDragReconcile::default();
    }
    promote_observed_user_frame(state, observed);
    let minimum_height = state.minimum_height;
    let fit_changed = match state.mode {
        OverlayMode::Expanded => fit_user_frame_to_work_areas(
            &mut state.user_frame,
            work_areas,
            primary_work_area,
            minimum_height,
        ),
        OverlayMode::Collapsed => fit_collapsed_position_to_work_areas(
            &mut state.user_frame,
            work_areas,
            primary_work_area,
        ),
    };
    NativeDragReconcile {
        promoted: true,
        fit_changed,
    }
}

fn positions_approximately_equal(left: (f64, f64), right: (f64, f64)) -> bool {
    (left.0 - right.0).abs() < 1.0 && (left.1 - right.1).abs() < 1.0
}

fn native_drag_changed_position(start: (f64, f64), observed: &OverlayFrame) -> bool {
    !positions_approximately_equal(start, (observed.x, observed.y))
}

fn should_sync_before_collapse(reconciliation: NativeDragReconcile, state: &OverlayState) -> bool {
    !reconciliation.promoted && state.presentation_frame.is_none()
}

fn frames_approximately_equal(left: &OverlayFrame, right: &OverlayFrame) -> bool {
    (left.x - right.x).abs() < 1.0
        && (left.y - right.y).abs() < 1.0
        && (left.width - right.width).abs() < 1.0
        && (left.height - right.height).abs() < 1.0
}

#[cfg(any(target_os = "macos", test))]
fn active_space_follow_allowed(state: &OverlayState) -> bool {
    state.resize_drag.is_none()
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

/// Fits the complete expanded frame inside the work area that contains most of
/// it. This is intentionally called only on restore or after native movement
/// settles; live dragging stays entirely in the window server.
fn fit_user_frame_to_screen(
    app: &AppHandle,
    frame: &mut OverlayFrame,
    minimum_height: f64,
) -> bool {
    let (areas, primary) = available_work_areas(app);
    fit_user_frame_to_work_areas(frame, &areas, primary, minimum_height)
}

fn fit_user_frame_to_work_areas(
    frame: &mut OverlayFrame,
    areas: &[LogicalWorkArea],
    primary: Option<LogicalWorkArea>,
    minimum_height: f64,
) -> bool {
    let Some(work_area) = choose_work_area_for_frame(frame, areas.iter().copied(), primary) else {
        return false;
    };
    fit_frame_to_work_area(frame, work_area, minimum_height)
}

fn fit_collapsed_position_to_work_areas(
    frame: &mut OverlayFrame,
    areas: &[LogicalWorkArea],
    primary: Option<LogicalWorkArea>,
) -> bool {
    let mut collapsed = OverlayFrame {
        x: frame.x,
        y: frame.y,
        width: SubtitleOverlayMetrics::COLLAPSED_WIDTH,
        height: SubtitleOverlayMetrics::COLLAPSED_HEIGHT,
    };
    let Some(work_area) = choose_work_area_for_frame(&collapsed, areas.iter().copied(), primary)
    else {
        return false;
    };
    clamp_frame_origin_to_work_area(&mut collapsed, work_area);
    let changed = collapsed.x != frame.x || collapsed.y != frame.y;
    frame.x = collapsed.x;
    frame.y = collapsed.y;
    changed
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

/// Origin for a saved frame (clamped into the visible screen) or the default
/// bottom-center placement 72 logical px above the bottom edge.
fn default_overlay_origin(
    app: &AppHandle,
    width: f64,
    height: f64,
    saved: &Option<OverlayFrame>,
) -> (f64, f64) {
    if let Some(saved) = saved {
        // Prefer the work area that contains most of the saved frame. This
        // preserves deliberate placement on a left/upper secondary monitor;
        // if that display disappeared, fall back to the primary work area.
        if let Some(work_area) = work_area_for_saved_frame(app, saved) {
            let mut restored = *saved;
            clamp_frame_origin_to_work_area(&mut restored, work_area);
            return (restored.x, restored.y);
        }
        return (saved.x, saved.y);
    }
    // Default to the bottom-center of the primary monitor.
    if let Some(monitor) = app.primary_monitor().ok().flatten() {
        let work_area = logical_work_area(&monitor);
        return default_overlay_origin_in_work_area(width, height, work_area);
    }
    (0.0, 0.0)
}

/// A monitor work area with logical dimensions and a desktop-space origin.
/// Windows desktop origins are physical pixels and `coordinate_scale` maps
/// logical distances into that coordinate space. Other platforms keep the
/// former all-logical representation with a scale of 1.
#[derive(Debug, Clone, Copy, PartialEq)]
struct LogicalWorkArea {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    coordinate_scale: f64,
}

impl LogicalWorkArea {
    fn coordinate_width(self) -> f64 {
        self.width * self.coordinate_scale
    }

    fn coordinate_height(self) -> f64 {
        self.height * self.coordinate_scale
    }

    fn coordinate_distance(self, logical: f64) -> f64 {
        logical * self.coordinate_scale
    }
}

fn logical_work_area(monitor: &tauri::Monitor) -> LogicalWorkArea {
    let scale = monitor.scale_factor().max(1.0);
    let area = monitor.work_area();
    #[cfg(target_os = "windows")]
    let (x, y, coordinate_scale) = (area.position.x as f64, area.position.y as f64, scale);
    #[cfg(not(target_os = "windows"))]
    let (x, y, coordinate_scale) = (
        area.position.x as f64 / scale,
        area.position.y as f64 / scale,
        1.0,
    );
    LogicalWorkArea {
        x,
        y,
        width: area.size.width as f64 / scale,
        height: area.size.height as f64 / scale,
        coordinate_scale,
    }
}

fn available_work_areas(app: &AppHandle) -> (Vec<LogicalWorkArea>, Option<LogicalWorkArea>) {
    let primary = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|monitor| logical_work_area(&monitor));
    let areas = app
        .available_monitors()
        .unwrap_or_default()
        .into_iter()
        .map(|monitor| logical_work_area(&monitor))
        .collect();
    (areas, primary)
}

fn work_area_for_saved_frame(app: &AppHandle, frame: &OverlayFrame) -> Option<LogicalWorkArea> {
    let (areas, primary) = available_work_areas(app);
    choose_work_area_for_frame(frame, areas, primary)
}

fn choose_work_area_for_frame(
    frame: &OverlayFrame,
    areas: impl IntoIterator<Item = LogicalWorkArea>,
    fallback: Option<LogicalWorkArea>,
) -> Option<LogicalWorkArea> {
    let areas = areas.into_iter().collect::<Vec<_>>();
    // A saved frame's origin is always written in its owning monitor's
    // desktop coordinate space. Prefer that unambiguous signal before
    // estimating intersection: the same logical width has different physical
    // extents on neighbouring mixed-DPI displays.
    if let Some(area) = areas
        .iter()
        .copied()
        .find(|area| frame_origin_is_inside_work_area(frame, *area))
    {
        return Some(area);
    }
    areas
        .into_iter()
        .map(|area| (frame_intersection_area(frame, area), area))
        .filter(|(intersection, _)| *intersection > 0.0)
        .max_by(|left, right| left.0.total_cmp(&right.0))
        .map(|(_, area)| area)
        .or(fallback)
}

fn frame_origin_is_inside_work_area(frame: &OverlayFrame, area: LogicalWorkArea) -> bool {
    frame.x >= area.x
        && frame.x < area.x + area.coordinate_width()
        && frame.y >= area.y
        && frame.y < area.y + area.coordinate_height()
}

/// Maps a saved frame into another display's work area without changing its
/// size or relative placement. Vertical placement preserves the bottom inset,
/// which keeps subtitles near the video controls across differently sized
/// displays; the final fit handles smaller destinations safely.
#[cfg(any(target_os = "macos", test))]
fn map_frame_between_work_areas(
    frame: OverlayFrame,
    source: LogicalWorkArea,
    target: LogicalWorkArea,
    minimum_height: f64,
) -> OverlayFrame {
    let mut mapped = if work_areas_approximately_equal(source, target) {
        frame
    } else {
        let horizontal_range = (source.width - frame.width).max(0.0);
        let horizontal_fraction = if horizontal_range > 0.0 {
            ((frame.x - source.x) / source.coordinate_scale / horizontal_range).clamp(0.0, 1.0)
        } else {
            0.5
        };
        let bottom_inset = ((source.y + source.coordinate_height() - frame.y)
            / source.coordinate_scale
            - frame.height)
            .max(0.0);
        let target_horizontal_range = (target.width - frame.width).max(0.0);
        OverlayFrame {
            x: target.x + target.coordinate_distance(horizontal_fraction * target_horizontal_range),
            y: target.y + target.coordinate_distance(target.height - frame.height - bottom_inset),
            ..frame
        }
    };
    fit_frame_to_work_area(&mut mapped, target, minimum_height);
    mapped
}

#[cfg(any(target_os = "macos", test))]
fn work_areas_approximately_equal(left: LogicalWorkArea, right: LogicalWorkArea) -> bool {
    (left.x - right.x).abs() < 1.0
        && (left.y - right.y).abs() < 1.0
        && (left.width - right.width).abs() < 1.0
        && (left.height - right.height).abs() < 1.0
        && (left.coordinate_scale - right.coordinate_scale).abs() < 0.001
}

fn frame_intersection_area(frame: &OverlayFrame, area: LogicalWorkArea) -> f64 {
    let width = (frame.x + area.coordinate_distance(frame.width))
        .min(area.x + area.coordinate_width())
        - frame.x.max(area.x);
    let height = (frame.y + area.coordinate_distance(frame.height))
        .min(area.y + area.coordinate_height())
        - frame.y.max(area.y);
    width.max(0.0) * height.max(0.0)
}

fn clamp_frame_origin_to_work_area(frame: &mut OverlayFrame, area: LogicalWorkArea) {
    let max_x = (area.x + area.coordinate_distance(area.width - frame.width)).max(area.x);
    let max_y = (area.y + area.coordinate_distance(area.height - frame.height)).max(area.y);
    frame.x = frame.x.clamp(area.x, max_x);
    frame.y = frame.y.clamp(area.y, max_y);
}

/// Normalizes dimensions first, then origin. Origin-only clamping cannot make
/// an oversized frame fully visible after a display or resolution change.
fn fit_frame_to_work_area(
    frame: &mut OverlayFrame,
    area: LogicalWorkArea,
    minimum_height: f64,
) -> bool {
    let previous = *frame;
    let maximum_width = area.width.clamp(1.0, SubtitleOverlayMetrics::MAXIMUM_WIDTH);
    let maximum_height = area
        .height
        .clamp(1.0, SubtitleOverlayMetrics::MAXIMUM_HEIGHT);
    let minimum_width = SubtitleOverlayMetrics::MINIMUM_WIDTH.min(maximum_width);
    let minimum_height = minimum_height.min(maximum_height);
    frame.width = frame.width.clamp(minimum_width, maximum_width);
    frame.height = frame.height.clamp(minimum_height, maximum_height);
    clamp_frame_origin_to_work_area(frame, area);
    *frame != previous
}

fn default_overlay_origin_in_work_area(
    width: f64,
    height: f64,
    area: LogicalWorkArea,
) -> (f64, f64) {
    let mut frame = OverlayFrame {
        x: area.x + area.coordinate_distance((area.width - width) / 2.0),
        y: area.y + area.coordinate_distance(area.height - height - 72.0),
        width,
        height,
    };
    clamp_frame_origin_to_work_area(&mut frame, area);
    (frame.x, frame.y)
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct OverlayControlGeometry {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

const OVERLAY_CONTROL_MODE_EVENT: &str = "overlay-control-mode";

/// Child/owned control surface for the subtitle canvas. The native bounds
/// always match the visible surface, so no transparent shadow margin steals
/// clicks from the video underneath.
pub struct OverlayControlWindowManager;

impl OverlayControlWindowManager {
    pub const DEFAULT_ISLAND_WIDTH: f64 = 200.0;
    pub const ISLAND_HEIGHT: f64 = 30.0;
    const MIN_ISLAND_WIDTH: f64 = 80.0;
    const MAX_ISLAND_WIDTH: f64 = 512.0;
    pub const PANEL_WIDTH: f64 = 280.0;
    // Matches the compact control panel on first open; React
    // immediately replaces it with the measured provider/locale-specific
    // height, but this default avoids a visible clipped first frame.
    pub const DEFAULT_PANEL_HEIGHT: f64 = 270.0;
    const MIN_PANEL_HEIGHT: f64 = 132.0;
    const MAX_PANEL_HEIGHT: f64 = 520.0;
    const ANCHOR_OFFSET_X: f64 = 18.0;
    const ANCHOR_OFFSET_Y: f64 = 16.0;
    const WORK_AREA_MARGIN: f64 = 8.0;

    pub fn ensure(app: &AppHandle) {
        if app.get_webview_window("overlay-control").is_some() {
            return;
        }
        let Some(overlay) = app.get_webview_window("overlay") else {
            pipeline_log!("overlay control failed label=missing_parent");
            return;
        };
        let builder =
            WebviewWindowBuilder::new(app, "overlay-control", WebviewUrl::App("index.html".into()))
                .title(dev_title("mimi"))
                .inner_size(Self::DEFAULT_ISLAND_WIDTH, Self::ISLAND_HEIGHT)
                .resizable(false)
                .transparent(true)
                .decorations(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .shadow(false)
                .focused(false)
                .visible(false);
        #[cfg(not(target_os = "windows"))]
        let builder = builder.visible_on_all_workspaces(true);
        #[cfg(target_os = "windows")]
        let builder = builder.focusable(false);
        #[cfg(target_os = "macos")]
        let builder = builder.accept_first_mouse(true);
        let builder = match builder.parent(&overlay) {
            Ok(builder) => builder,
            Err(_) => {
                pipeline_log!("overlay control failed label=parent_unavailable");
                return;
            }
        };

        match builder.build() {
            Ok(window) => {
                if convert_overlay_to_panel(&window, OverlayPanelKind::Controls).is_err() {
                    let _ = window.destroy();
                    pipeline_log!("overlay control failed label=panel_conversion_failed");
                    return;
                }
                configure_overlay_window(&window);
                pipeline_log!("overlay control created");
            }
            Err(_) => pipeline_log!("overlay control failed label=create_failed"),
        }
    }

    pub fn mode(app: &AppHandle) -> OverlayControlMode {
        app.try_state::<OverlayControlState>()
            .map(|state| state.mode())
            .unwrap_or(OverlayControlMode::Hidden)
    }

    /// Derives the persistent island visibility from the overlay lifecycle.
    /// An open panel survives ordinary subtitle/status broadcasts, but any
    /// inactive, collapsed, or immersive transition hides the entire child
    /// surface.
    pub fn sync_presentation(
        app: &AppHandle,
        is_active: bool,
        is_collapsed: bool,
        is_immersive: bool,
    ) {
        let Some(state) = app.try_state::<OverlayControlState>() else {
            return;
        };
        let current = state.mode();
        let next = control_mode_for_presentation(current, is_active, is_collapsed, is_immersive);
        let mode_changed = state.set_mode(next);
        let visible_surface_needs_restore = !mode_changed
            && next != OverlayControlMode::Hidden
            && app
                .get_webview_window("overlay-control")
                .and_then(|window| window.is_visible().ok())
                != Some(true);
        if mode_changed || visible_surface_needs_restore {
            Self::apply_mode(app, next, false);
        }
    }

    pub fn toggle_panel(app: &AppHandle) {
        let Some(state) = app.try_state::<OverlayControlState>() else {
            return;
        };
        let next = match state.mode() {
            OverlayControlMode::Island => OverlayControlMode::Panel,
            OverlayControlMode::Panel => OverlayControlMode::Island,
            OverlayControlMode::Hidden => return,
        };
        if state.set_mode(next) {
            Self::apply_mode(app, next, next == OverlayControlMode::Panel);
        }
    }

    /// Returns an expanded panel to its compact island. Hidden surfaces stay
    /// hidden, so a delayed focus-loss task can never resurrect an inactive
    /// session.
    pub fn dismiss_panel(app: &AppHandle) {
        let Some(state) = app.try_state::<OverlayControlState>() else {
            return;
        };
        if state.mode() == OverlayControlMode::Panel && state.set_mode(OverlayControlMode::Island) {
            Self::apply_mode(app, OverlayControlMode::Island, false);
        }
    }

    /// Delayed focus-loss handling avoids a hide/show race when a click inside
    /// the control WebView changes focus during the island/panel resize.
    pub fn schedule_dismiss(app: &AppHandle) {
        let Some(state) = app.try_state::<OverlayControlState>() else {
            return;
        };
        let (_, _, generation) = state.snapshot();
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(180)).await;
            let Some(state) = app.try_state::<OverlayControlState>() else {
                return;
            };
            let (mode, _, current_generation) = state.snapshot();
            if current_generation != generation || mode != OverlayControlMode::Panel {
                return;
            }
            if state.set_mode(OverlayControlMode::Island) {
                Self::apply_mode(&app, OverlayControlMode::Island, false);
            }
        });
    }

    /// Cancels a delayed focus-loss dismissal when focus returns during the
    /// island-to-panel native resize.
    pub fn cancel_scheduled_dismiss(app: &AppHandle) {
        let Some(state) = app.try_state::<OverlayControlState>() else {
            return;
        };
        if state.mode() == OverlayControlMode::Panel {
            state.cancel_scheduled_dismiss();
        }
    }

    /// Updates the tightly-fitted panel height measured by React. The value is
    /// clamped before any native window mutation.
    pub fn set_panel_height(app: &AppHandle, height: f64) {
        if !height.is_finite() {
            return;
        }
        let height = height.clamp(Self::MIN_PANEL_HEIGHT, Self::MAX_PANEL_HEIGHT);
        let Some(state) = app.try_state::<OverlayControlState>() else {
            return;
        };
        if state.set_panel_height(height) && state.mode() == OverlayControlMode::Panel {
            Self::apply_geometry(app, OverlayControlMode::Panel);
        }
    }

    /// Caches the collapsed capsule's intrinsic width independently of the
    /// panel. Hidden and expanded surfaces only remember this measurement;
    /// a late report must neither reveal a hidden window nor shrink a panel.
    pub fn set_island_width(app: &AppHandle, width: f64) {
        let Some(width) = measured_island_width(width) else {
            return;
        };
        let current_app = app.clone();
        let _ = app.run_on_main_thread(move || {
            let Some(state) = current_app.try_state::<OverlayControlState>() else {
                return;
            };
            if state.set_island_width(width) && state.mode() == OverlayControlMode::Island {
                Self::apply_geometry(&current_app, OverlayControlMode::Island);
            }
        });
    }

    /// Keeps the child surface attached to the overlay on platforms where an
    /// owned/transient window does not automatically move with its owner, and
    /// performs the final cross-platform clamp after movement settles. macOS
    /// intentionally does not call this for every Moved event because AppKit
    /// already moves native child windows synchronously with their parent.
    pub fn follow_overlay(app: &AppHandle) {
        let mode = Self::mode(app);
        if mode != OverlayControlMode::Hidden {
            #[cfg(target_os = "linux")]
            Self::apply_mode(app, mode, false);
            #[cfg(not(target_os = "linux"))]
            Self::apply_geometry(app, mode);
        }
    }

    fn apply_mode(app: &AppHandle, mode: OverlayControlMode, focus: bool) {
        #[cfg(target_os = "linux")]
        {
            let current_app = app.clone();
            let _ = app.run_on_main_thread(move || {
                // A geometry debounce or map callback may outlive Stop.
                // Re-read and apply in one main-thread transaction so stale
                // follow work cannot show a control after it was hidden.
                let current = Self::mode(&current_app);
                Self::apply_mode_now(&current_app, current, focus && current == mode);
            });
        }
        #[cfg(not(target_os = "linux"))]
        Self::apply_mode_now(app, mode, focus);
    }

    fn apply_mode_now(app: &AppHandle, mode: OverlayControlMode, focus: bool) {
        #[cfg(target_os = "macos")]
        macos_control_dismiss::sync(app);
        let Some(window) = app.get_webview_window("overlay-control") else {
            return;
        };
        #[cfg(target_os = "windows")]
        let focus_policy = windows_control_focus_policy(mode, focus);
        #[cfg(target_os = "windows")]
        let _ = window.set_focusable(focus_policy.focusable_before_show);
        if mode == OverlayControlMode::Hidden {
            let _ = window.hide();
            let _ = window.emit(OVERLAY_CONTROL_MODE_EVENT, mode);
            return;
        }

        let _ = window.emit(OVERLAY_CONTROL_MODE_EVENT, mode);
        #[cfg(target_os = "linux")]
        if !Self::apply_geometry(app, mode) {
            return;
        }
        #[cfg(not(target_os = "linux"))]
        Self::apply_geometry(app, mode);
        if !window.is_visible().unwrap_or(false) {
            show_overlay_control_window(app, &window);
        }
        #[cfg(target_os = "windows")]
        let _ = window.set_focusable(focus_policy.focusable_after_show);
        #[cfg(target_os = "windows")]
        let should_focus = focus_policy.focus;
        #[cfg(not(target_os = "windows"))]
        let should_focus = focus;
        if should_focus {
            focus_overlay_control(&window);
        }
    }

    fn apply_geometry(app: &AppHandle, mode: OverlayControlMode) -> bool {
        let Some(window) = app.get_webview_window("overlay-control") else {
            return false;
        };
        let Some((anchor_x, anchor_y, work_area)) = Self::overlay_anchor(app) else {
            return false;
        };
        let (island_width, panel_height) = app
            .try_state::<OverlayControlState>()
            .map(|state| state.dimensions())
            .unwrap_or((Self::DEFAULT_ISLAND_WIDTH, Self::DEFAULT_PANEL_HEIGHT));
        let geometry = overlay_control_geometry(
            mode,
            anchor_x,
            anchor_y,
            island_width,
            panel_height,
            work_area,
        );
        #[cfg(target_os = "linux")]
        {
            use gtk::prelude::*;
            let native_window = window.clone();
            let _ = window.run_on_main_thread(move || {
                if let Ok(window) = native_window.gtk_window() {
                    // A non-resizable GTK window otherwise retains its
                    // natural 200px height and intercepts invisible clicks.
                    window.set_size_request(geometry.width as i32, geometry.height as i32);
                    window.resize(geometry.width as i32, geometry.height as i32);
                    window.move_(geometry.x.round() as i32, geometry.y.round() as i32);
                }
            });
        }
        #[cfg(not(target_os = "linux"))]
        {
            set_desktop_size(
                &window,
                geometry.width,
                geometry.height,
                work_area.coordinate_scale,
            );
            set_desktop_position(&window, geometry.x, geometry.y);
        }
        true
    }

    fn overlay_anchor(app: &AppHandle) -> Option<(f64, f64, LogicalWorkArea)> {
        let overlay = app.get_webview_window("overlay")?;
        #[cfg(not(target_os = "linux"))]
        let position = overlay.outer_position().ok()?;
        let monitor = overlay
            .current_monitor()
            .ok()
            .flatten()
            .or_else(|| app.primary_monitor().ok().flatten())?;
        let work_area = logical_work_area(&monitor);
        #[cfg(target_os = "windows")]
        let (x, y) = (position.x as f64, position.y as f64);
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        let (x, y) = {
            let scale = overlay.scale_factor().unwrap_or(1.0).max(1.0);
            (position.x as f64 / scale, position.y as f64 / scale)
        };
        #[cfg(target_os = "linux")]
        let (x, y) = {
            let frame = overlay_frame_from_window(app)?;
            (frame.x, frame.y)
        };
        Some((
            x + work_area.coordinate_distance(Self::ANCHOR_OFFSET_X),
            y + work_area.coordinate_distance(Self::ANCHOR_OFFSET_Y),
            work_area,
        ))
    }
}

#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WindowsControlFocusPolicy {
    focusable_before_show: bool,
    focusable_after_show: bool,
    focus: bool,
}

/// Windows surfaces are nonactivating while shown. Only an explicit click
/// that expands the full panel may make it focusable and request focus.
#[cfg(any(target_os = "windows", test))]
fn windows_control_focus_policy(
    mode: OverlayControlMode,
    explicit_user_action: bool,
) -> WindowsControlFocusPolicy {
    match mode {
        OverlayControlMode::Hidden | OverlayControlMode::Island => WindowsControlFocusPolicy {
            focusable_before_show: false,
            focusable_after_show: false,
            focus: false,
        },
        OverlayControlMode::Panel if explicit_user_action => WindowsControlFocusPolicy {
            focusable_before_show: true,
            focusable_after_show: true,
            focus: true,
        },
        OverlayControlMode::Panel => WindowsControlFocusPolicy {
            focusable_before_show: false,
            focusable_after_show: true,
            focus: false,
        },
    }
}

fn control_mode_for_presentation(
    current: OverlayControlMode,
    is_active: bool,
    is_collapsed: bool,
    is_immersive: bool,
) -> OverlayControlMode {
    if !is_active || is_collapsed || is_immersive {
        OverlayControlMode::Hidden
    } else if current == OverlayControlMode::Panel {
        OverlayControlMode::Panel
    } else {
        OverlayControlMode::Island
    }
}

fn measured_island_width(width: f64) -> Option<f64> {
    width.is_finite().then(|| {
        width.ceil().clamp(
            OverlayControlWindowManager::MIN_ISLAND_WIDTH,
            OverlayControlWindowManager::MAX_ISLAND_WIDTH,
        )
    })
}

fn overlay_control_geometry(
    mode: OverlayControlMode,
    anchor_x: f64,
    anchor_y: f64,
    island_width: f64,
    panel_height: f64,
    work_area: LogicalWorkArea,
) -> OverlayControlGeometry {
    let margin = OverlayControlWindowManager::WORK_AREA_MARGIN;
    let (width, requested_height) = match mode {
        OverlayControlMode::Panel => (OverlayControlWindowManager::PANEL_WIDTH, panel_height),
        OverlayControlMode::Hidden | OverlayControlMode::Island => {
            (island_width, OverlayControlWindowManager::ISLAND_HEIGHT)
        }
    };
    let width = width.min((work_area.width - margin * 2.0).max(1.0));
    let available_height = (work_area.height - margin * 2.0).max(1.0);
    let height = requested_height.min(available_height);
    let min_x = work_area.x + work_area.coordinate_distance(margin);
    let max_x =
        (work_area.x + work_area.coordinate_distance(work_area.width - width - margin)).max(min_x);
    let x = anchor_x.clamp(min_x, max_x);
    let min_y = work_area.y + work_area.coordinate_distance(margin);
    let max_y = (work_area.y + work_area.coordinate_distance(work_area.height - height - margin))
        .max(min_y);
    let preferred_y = if mode == OverlayControlMode::Panel && anchor_y > max_y {
        // Grow upward while keeping the panel's bottom aligned with the
        // compact island's bottom.
        anchor_y
            + work_area.coordinate_distance(OverlayControlWindowManager::ISLAND_HEIGHT - height)
    } else {
        anchor_y
    };
    let y = preferred_y.clamp(min_y, max_y);
    OverlayControlGeometry {
        x,
        y,
        width,
        height,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PhysicalScreenArea {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl PhysicalScreenArea {
    fn right(self) -> f64 {
        self.x + self.width
    }

    fn bottom(self) -> f64 {
        self.y + self.height
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayScreenEdge {
    Top,
    Bottom,
    Left,
    Right,
}

fn nearest_tray_screen_edge(
    tray: PhysicalScreenArea,
    monitor: PhysicalScreenArea,
) -> TrayScreenEdge {
    let candidates = [
        (TrayScreenEdge::Top, (tray.y - monitor.y).abs()),
        (
            TrayScreenEdge::Bottom,
            (monitor.bottom() - tray.bottom()).abs(),
        ),
        (TrayScreenEdge::Left, (tray.x - monitor.x).abs()),
        (
            TrayScreenEdge::Right,
            (monitor.right() - tray.right()).abs(),
        ),
    ];

    candidates
        .into_iter()
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .map(|candidate| candidate.0)
        .unwrap_or(TrayScreenEdge::Bottom)
}

fn tray_panel_origin(
    tray: PhysicalScreenArea,
    panel_width: f64,
    panel_height: f64,
    monitor: PhysicalScreenArea,
    work_area: PhysicalScreenArea,
    margin: f64,
) -> (f64, f64) {
    let tray_center_x = tray.x + tray.width / 2.0;
    let tray_center_y = tray.y + tray.height / 2.0;
    let (preferred_x, preferred_y) = match nearest_tray_screen_edge(tray, monitor) {
        TrayScreenEdge::Top => (tray_center_x - panel_width / 2.0, tray.bottom() + margin),
        TrayScreenEdge::Bottom => (
            tray_center_x - panel_width / 2.0,
            tray.y - panel_height - margin,
        ),
        TrayScreenEdge::Left => (tray.right() + margin, tray_center_y - panel_height / 2.0),
        TrayScreenEdge::Right => (
            tray.x - panel_width - margin,
            tray_center_y - panel_height / 2.0,
        ),
    };

    let min_x = work_area.x + margin;
    let max_x = (work_area.right() - panel_width - margin).max(min_x);
    let min_y = work_area.y + margin;
    let max_y = (work_area.bottom() - panel_height - margin).max(min_y);
    (
        preferred_x.clamp(min_x, max_x),
        preferred_y.clamp(min_y, max_y),
    )
}

/// Tray popup panel: a compact frameless always-on-top control surface shown
/// inward from the tray edge on both desktop platforms.
pub struct TrayPanelManager;

impl TrayPanelManager {
    pub fn ensure(app: &AppHandle) {
        if app.get_webview_window("tray-panel").is_some() {
            return;
        }
        let builder =
            WebviewWindowBuilder::new(app, "tray-panel", WebviewUrl::App("index.html".into()))
                .title(dev_title("mimi"))
                .inner_size(320.0, 410.0)
                .resizable(false)
                .transparent(true)
                .decorations(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .shadow(false)
                .visible(false);

        match builder.build() {
            Ok(_) => pipeline_log!("tray panel created"),
            Err(_) => pipeline_log!("tray panel failed label=create_failed"),
        }
    }

    pub fn toggle(app: &AppHandle, tray_rect: &tauri::Rect) {
        let Some(window) = app.get_webview_window("tray-panel") else {
            return;
        };
        let is_visible = window.is_visible().unwrap_or(false);
        if is_visible {
            let _ = window.hide();
            return;
        }
        use tauri_plugin_positioner::{Position, WindowExt};

        // Compute from the click event instead of assuming that every tray is
        // on the top edge. Windows normally puts the taskbar at the bottom,
        // and can also place it on another edge or a negative-origin monitor.
        // The positioner remains a fallback for platforms with incomplete
        // tray rectangles.
        if !Self::position_from_tray_rect(&window, tray_rect)
            && window
                .move_window_constrained(Position::TrayCenter)
                .is_err()
        {
            tracing::debug!("tray panel position unavailable label=initial_position_failed");
            Self::position_in_work_area(&window);
        }
        if let Ok(position) = window.outer_position() {
            tracing::debug!("tray panel: positioned at {position:?}");
        }
        let _ = window.show();
        let _ = window.set_focus();

        // The first move can run before the window reports its final size.
        // Re-apply the same edge-aware calculation after it becomes visible.
        let window = window.clone();
        let tray_rect = *tray_rect;
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(60)).await;
            if !Self::position_from_tray_rect(&window, &tray_rect)
                && window
                    .move_window_constrained(Position::TrayCenter)
                    .is_err()
            {
                tracing::debug!("tray panel position unavailable label=settle_position_failed");
                Self::position_in_work_area(&window);
            }
            if let Ok(position) = window.outer_position() {
                tracing::debug!("tray panel: re-positioned at {position:?}");
            }
        });
    }

    fn position_from_tray_rect(window: &tauri::WebviewWindow, tray_rect: &tauri::Rect) -> bool {
        let tray_position: tauri::PhysicalPosition<f64> = tray_rect.position.to_physical(1.0);
        let tray_size: tauri::PhysicalSize<f64> = tray_rect.size.to_physical(1.0);
        let tray = PhysicalScreenArea {
            x: tray_position.x,
            y: tray_position.y,
            width: tray_size.width,
            height: tray_size.height,
        };
        let Some(monitor) = window
            .monitor_from_point(tray.x + tray.width / 2.0, tray.y + tray.height / 2.0)
            .ok()
            .flatten()
        else {
            return false;
        };
        let Ok(window_size) = window.outer_size() else {
            return false;
        };
        let monitor_area = PhysicalScreenArea {
            x: monitor.position().x as f64,
            y: monitor.position().y as f64,
            width: monitor.size().width as f64,
            height: monitor.size().height as f64,
        };
        let native_work_area = monitor.work_area();
        let work_area = PhysicalScreenArea {
            x: native_work_area.position.x as f64,
            y: native_work_area.position.y as f64,
            width: native_work_area.size.width as f64,
            height: native_work_area.size.height as f64,
        };
        let margin = 8.0 * monitor.scale_factor().max(1.0);
        let (x, y) = tray_panel_origin(
            tray,
            window_size.width as f64,
            window_size.height as f64,
            monitor_area,
            work_area,
            margin,
        );
        window
            .set_position(tauri::PhysicalPosition::new(
                x.round() as i32,
                y.round() as i32,
            ))
            .is_ok()
    }

    /// Fallback placement inside the current monitor work area. Windows uses
    /// the bottom-right corner; macOS uses the top-right corner.
    fn position_in_work_area(window: &tauri::WebviewWindow) {
        let Some(monitor) = window.current_monitor().ok().flatten() else {
            return;
        };
        let work_area = monitor.work_area();
        let window_size = window.outer_size().unwrap_or_default();
        let margin = 8.0 * monitor.scale_factor().max(1.0);
        let x = (work_area.position.x as f64 + work_area.size.width as f64
            - window_size.width as f64
            - margin)
            .max(work_area.position.x as f64);
        #[cfg(target_os = "windows")]
        let y = (work_area.position.y as f64 + work_area.size.height as f64
            - window_size.height as f64
            - margin)
            .max(work_area.position.y as f64);
        #[cfg(not(target_os = "windows"))]
        let y = work_area.position.y as f64 + margin;
        let _ = window.set_position(tauri::PhysicalPosition::new(x as i32, y as i32));
    }

    pub fn hide(app: &AppHandle) {
        if let Some(window) = app.get_webview_window("tray-panel") {
            let _ = window.hide();
        }
    }
}

/// Shows the settings window, creating it first if it does not exist (the
/// window is hidden on close — see the CloseRequested handler in lib.rs — but
/// a stale or crashed webview could leave it missing). Mirrors the window
/// declared in tauri.conf.json.
pub fn ensure_settings_window(app: &AppHandle) {
    // The tray panel is always-on-top. Hide it before restoring Settings so
    // the requested window cannot reopen underneath the popup.
    TrayPanelManager::hide(app);
    let Some(window) = app.get_webview_window("settings") else {
        let builder =
            WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("index.html".into()))
                .title(dev_title("mimi"))
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

#[cfg(target_os = "macos")]
mod macos_space;
pub mod resize;

#[cfg(target_os = "macos")]
pub fn install_active_space_observer(
    app: &AppHandle,
    overlay: &Arc<std::sync::Mutex<OverlayState>>,
    settings: &Arc<SettingsStore>,
) {
    macos_space::install(app, overlay, settings);
    // Seed the runtime presentation frame as well as observing later changes;
    // Mimi may start while another application is already full-screen.
    OverlayWindowManager::follow_active_space(app, overlay, settings);
}

#[cfg(test)]
mod geometry_tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn control_panel_declares_key_eligibility_without_main_window_eligibility() {
        use objc2::{msg_send, ClassType};

        // This checks declared policy, not runtime keyboard delivery. Native
        // typing still requires the explicit makeKeyWindow path and real UI
        // verification; class selectors do not prove a live first responder.
        let control = RawSubtitleControlPanel::class();
        let can_become_key: bool = unsafe { msg_send![control, canBecomeKeyWindow] };
        let can_become_main: bool = unsafe { msg_send![control, canBecomeMainWindow] };
        let key_only_if_needed: bool = unsafe { msg_send![control, becomesKeyOnlyIfNeeded] };
        assert!(can_become_key);
        assert!(!can_become_main);
        assert!(!key_only_if_needed);

        // Reading subtitles keeps the more restrictive existing key policy.
        let subtitles = RawSubtitleOverlayPanel::class();
        let key_only_if_needed: bool = unsafe { msg_send![subtitles, becomesKeyOnlyIfNeeded] };
        assert!(key_only_if_needed);
        let can_become_main: bool = unsafe { msg_send![subtitles, canBecomeMainWindow] };
        assert!(!can_become_main);
    }

    fn frame(x: f64, y: f64, w: f64, h: f64) -> OverlayFrame {
        OverlayFrame {
            x,
            y,
            width: w,
            height: h,
        }
    }

    const WORK_AREA: LogicalWorkArea = LogicalWorkArea {
        x: 0.0,
        y: 24.0,
        width: 1512.0,
        height: 958.0,
        coordinate_scale: 1.0,
    };

    fn state_with_frame(user_frame: OverlayFrame) -> OverlayState {
        OverlayState {
            mode: OverlayMode::Expanded,
            minimum_height: SubtitleOverlayMetrics::MINIMUM_HEIGHT,
            user_frame,
            presentation_frame: None,
            native_drag_start: None,
            resize_drag: None,
            resize_start: None,
            last_geometry_event: None,
            geometry_task_pending: false,
            resize_log_at: None,
        }
    }

    #[test]
    fn expanded_passes_user_frame_through() {
        let geometry = geometry_for(
            OverlayMode::Expanded,
            &frame(400.0, 300.0, 640.0, 136.0),
            136.0,
        );
        assert_eq!(
            (geometry.x, geometry.y, geometry.width, geometry.height),
            (400.0, 300.0, 640.0, 136.0)
        );
        assert_eq!(geometry.min, (360.0, 136.0));
        assert_eq!(geometry.max, (1200.0, 600.0));
    }

    #[test]
    fn source_display_and_font_changes_raise_only_the_needed_height() {
        let mut state = state_with_frame(frame(410.0, 300.0, 730.0, 136.0));
        let mut preferences = Preferences {
            subtitle_display_mode: crate::core::models::SubtitleDisplayMode::Bilingual,
            font_size: 18.0,
            ..Preferences::default()
        };
        assert!(state.update_minimum_height(minimum_height_for_preferences(&preferences)));
        assert_eq!(state.user_frame, frame(410.0, 300.0, 730.0, 156.0));
        preferences.audio_input = crate::core::audio_input::AudioInput::Both;
        assert!(state.update_minimum_height(minimum_height_for_preferences(&preferences)));
        assert_eq!(state.user_frame, frame(410.0, 300.0, 730.0, 252.0));
        preferences.font_size = 20.0;
        assert!(state.update_minimum_height(minimum_height_for_preferences(&preferences)));
        assert_eq!(state.user_frame, frame(410.0, 300.0, 730.0, 264.0));
        preferences.subtitle_display_mode = crate::core::models::SubtitleDisplayMode::Original;
        assert!(state.update_minimum_height(minimum_height_for_preferences(&preferences)));
        assert_eq!(state.minimum_height, 216.0);
        assert_eq!(state.user_frame, frame(410.0, 300.0, 730.0, 264.0));
        preferences.audio_input = crate::core::audio_input::AudioInput::System;
        assert!(state.update_minimum_height(minimum_height_for_preferences(&preferences)));
        assert_eq!(state.minimum_height, 156.0);
        assert_eq!(state.user_frame, frame(410.0, 300.0, 730.0, 264.0));
        preferences.show_subtitle_timestamps = true;
        assert!(state.update_minimum_height(minimum_height_for_preferences(&preferences)));
        assert_eq!(state.minimum_height, 180.0);
        assert_eq!(state.user_frame, frame(410.0, 300.0, 730.0, 264.0));
        preferences.audio_input = crate::core::audio_input::AudioInput::Microphone;
        assert!(!state.update_minimum_height(minimum_height_for_preferences(&preferences)));
        assert_eq!(state.minimum_height, 180.0);
        assert_eq!(state.user_frame, frame(410.0, 300.0, 730.0, 264.0));
        preferences.show_subtitle_timestamps = false;
        assert!(state.update_minimum_height(minimum_height_for_preferences(&preferences)));
        assert_eq!(state.minimum_height, 156.0);
        assert_eq!(state.user_frame, frame(410.0, 300.0, 730.0, 264.0));
    }

    #[test]
    fn system_timestamp_toggle_expands_a_compact_frame_without_losing_user_geometry() {
        let mut state = state_with_frame(frame(410.0, 300.0, 730.0, 136.0));
        let mut preferences = Preferences {
            show_subtitle_timestamps: true,
            ..Preferences::default()
        };
        assert!(state.update_minimum_height(minimum_height_for_preferences(&preferences)));
        assert_eq!(state.minimum_height, 180.0);
        assert_eq!(state.user_frame, frame(410.0, 300.0, 730.0, 180.0));

        preferences.show_subtitle_timestamps = false;
        assert!(state.update_minimum_height(minimum_height_for_preferences(&preferences)));
        assert_eq!(state.minimum_height, 156.0);
        assert_eq!(state.user_frame, frame(410.0, 300.0, 730.0, 180.0));
    }

    #[test]
    fn requirement_changes_invalidate_native_reads_and_queued_writes_without_frame_changes() {
        let mut state = state_with_frame(frame(400.0, 300.0, 640.0, 482.0));
        let pending_read = OverlayGeometrySnapshot::from(&state);
        let pending_resize_or_animation = OverlayApplySnapshot::from(&state);
        let before = state.user_frame;
        assert!(state.update_minimum_height(200.0));
        assert_eq!(state.user_frame, before);
        assert!(!geometry_snapshot_is_current(&state, pending_read));
        assert!(!apply_snapshot_is_current(
            &state,
            pending_resize_or_animation
        ));
    }

    #[test]
    fn collapsed_requirement_changes_keep_the_fixed_bar_and_remember_expanded_geometry() {
        let mut state = state_with_frame(frame(400.0, 300.0, 730.0, 136.0));
        state.mode = OverlayMode::Collapsed;
        state.update_minimum_height(200.0);
        let collapsed = geometry_for(state.mode, &state.effective_frame(), state.minimum_height);
        assert_eq!((collapsed.width, collapsed.height), (280.0, 54.0));
        assert_eq!(collapsed.min, collapsed.max);
        assert_eq!((collapsed.x, collapsed.y), (400.0, 300.0));
        state.mode = OverlayMode::Expanded;
        let expanded = geometry_for(state.mode, &state.effective_frame(), state.minimum_height);
        assert_eq!((expanded.width, expanded.height), (730.0, 200.0));
        assert_eq!(expanded.min, (360.0, 200.0));
    }

    #[test]
    fn followed_and_saved_frames_grow_independently_without_changing_their_origins() {
        let mut state = state_with_frame(frame(400.0, 300.0, 730.0, 136.0));
        state.presentation_frame = Some(frame(-1200.0, 200.0, 600.0, 144.0));
        state.update_minimum_height(200.0);
        assert_eq!(state.user_frame, frame(400.0, 300.0, 730.0, 200.0));
        assert_eq!(state.effective_frame(), frame(-1200.0, 200.0, 600.0, 200.0));
        assert!(presentation_frame_matches_observed(
            &state,
            &state.effective_frame()
        ));
        assert!(!adopt_observed_frame(
            &mut state,
            frame(-1200.0, 200.0, 600.0, 200.0)
        ));
        assert_eq!(state.user_frame, frame(400.0, 300.0, 730.0, 200.0));
    }

    #[test]
    fn native_resize_uses_the_current_minimum_and_keeps_the_opposite_edge_anchored() {
        let start = ResizeGesture {
            pointer: (400.0, 300.0),
            frame: frame(400.0, 300.0, 640.0, 482.0),
            work_area: WORK_AREA,
        };
        let resized = resized_frame_for_pointer(ResizeRegion::Top, start, (400.0, 900.0), 200.0);
        assert_eq!(resized.height, 200.0);
        assert_eq!(
            resized.y + resized.height,
            start.frame.y + start.frame.height
        );
        assert_eq!((resized.x, resized.width), (400.0, 640.0));
    }

    #[test]
    fn fitting_and_native_observation_cannot_restore_a_height_below_the_current_requirement() {
        let mut state = state_with_frame(frame(400.0, 300.0, 640.0, 136.0));
        state.update_minimum_height(200.0);
        promote_observed_user_frame(&mut state, frame(400.0, 300.0, 640.0, 136.0));
        assert_eq!(state.user_frame.height, 200.0);
        let mut fitted = frame(400.0, 300.0, 640.0, 136.0);
        assert!(fit_frame_to_work_area(
            &mut fitted,
            WORK_AREA,
            state.minimum_height
        ));
        assert_eq!(fitted, state.user_frame);
    }

    #[test]
    fn collapsed_is_fixed_size_at_user_origin() {
        let geometry = geometry_for(
            OverlayMode::Collapsed,
            &frame(400.0, 300.0, 640.0, 136.0),
            136.0,
        );
        assert_eq!(
            (geometry.x, geometry.y, geometry.width, geometry.height),
            (400.0, 300.0, 280.0, 54.0)
        );
        // min == max: the OS cannot resize the collapsed bar.
        assert_eq!(geometry.min, (280.0, 54.0));
        assert_eq!(geometry.max, (280.0, 54.0));
    }

    #[test]
    fn active_expanded_overlay_keeps_an_open_panel() {
        assert_eq!(
            control_mode_for_presentation(OverlayControlMode::Panel, true, false, false),
            OverlayControlMode::Panel
        );
        assert_eq!(
            control_mode_for_presentation(OverlayControlMode::Hidden, true, false, false),
            OverlayControlMode::Island
        );
    }

    #[test]
    fn inactive_or_collapsed_overlay_hides_control_surface() {
        assert_eq!(
            control_mode_for_presentation(OverlayControlMode::Panel, false, false, false),
            OverlayControlMode::Hidden
        );
        assert_eq!(
            control_mode_for_presentation(OverlayControlMode::Panel, true, true, false),
            OverlayControlMode::Hidden
        );
        assert_eq!(
            control_mode_for_presentation(OverlayControlMode::Panel, true, false, true),
            OverlayControlMode::Hidden
        );
    }

    #[test]
    fn persisted_layout_requires_an_exact_coordinate_version() {
        assert!(frame_layout_is_current(OVERLAY_FRAME_LAYOUT_VERSION));
        assert!(!frame_layout_is_current(
            OVERLAY_FRAME_LAYOUT_VERSION.saturating_sub(1)
        ));
        assert!(!frame_layout_is_current(OVERLAY_FRAME_LAYOUT_VERSION + 1));
    }

    #[test]
    fn windows_control_focus_policy_never_activates_passive_surfaces() {
        assert_eq!(
            windows_control_focus_policy(OverlayControlMode::Island, false),
            WindowsControlFocusPolicy {
                focusable_before_show: false,
                focusable_after_show: false,
                focus: false,
            }
        );
        assert_eq!(
            windows_control_focus_policy(OverlayControlMode::Panel, false),
            WindowsControlFocusPolicy {
                focusable_before_show: false,
                focusable_after_show: true,
                focus: false,
            }
        );
        assert_eq!(
            windows_control_focus_policy(OverlayControlMode::Panel, true),
            WindowsControlFocusPolicy {
                focusable_before_show: true,
                focusable_after_show: true,
                focus: true,
            }
        );
    }

    #[test]
    fn mixed_dpi_work_areas_keep_distinct_physical_desktop_origins() {
        let primary = LogicalWorkArea {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1040.0,
            coordinate_scale: 1.0,
        };
        let right = LogicalWorkArea {
            x: 1920.0,
            y: 0.0,
            width: 2560.0 / 1.5,
            height: 1400.0 / 1.5,
            coordinate_scale: 1.5,
        };
        let on_right = frame(2070.0, 150.0, 640.0, 136.0);

        assert_eq!(
            choose_work_area_for_frame(&on_right, [primary, right], Some(primary)),
            Some(right)
        );
        assert_eq!(
            default_overlay_origin_in_work_area(640.0, 136.0, right),
            (2720.0, 1088.0)
        );
        let straddling_from_primary = frame(1500.0, 150.0, 640.0, 136.0);
        assert_eq!(
            choose_work_area_for_frame(&straddling_from_primary, [primary, right], Some(primary)),
            Some(primary)
        );
    }

    #[test]
    fn mixed_dpi_resize_converts_physical_delta_back_to_logical_size() {
        let work_area = LogicalWorkArea {
            x: 1920.0,
            y: 0.0,
            width: 2560.0 / 1.5,
            height: 1400.0 / 1.5,
            coordinate_scale: 1.5,
        };
        let start = ResizeGesture {
            pointer: (3030.0, 354.0),
            frame: frame(2070.0, 150.0, 640.0, 136.0),
            work_area,
        };

        let resized =
            resized_frame_for_pointer(ResizeRegion::BottomRight, start, (3180.0, 429.0), 136.0);

        assert_eq!(resized, frame(2070.0, 150.0, 740.0, 186.0));
    }

    #[test]
    fn saved_frame_prefers_the_secondary_work_area_it_occupies() {
        let primary = LogicalWorkArea {
            x: 0.0,
            y: 24.0,
            width: 1512.0,
            height: 958.0,
            coordinate_scale: 1.0,
        };
        let left = LogicalWorkArea {
            x: -1728.0,
            y: -120.0,
            width: 1728.0,
            height: 1080.0,
            coordinate_scale: 1.0,
        };
        let saved = frame(-1400.0, 680.0, 640.0, 136.0);

        assert_eq!(
            choose_work_area_for_frame(&saved, [primary, left], Some(primary)),
            Some(left)
        );
    }

    #[test]
    fn active_space_mapping_preserves_center_and_bottom_inset() {
        let target = LogicalWorkArea {
            x: -1728.0,
            y: -120.0,
            width: 1728.0,
            height: 1080.0,
            coordinate_scale: 1.0,
        };
        let saved = frame(436.0, 774.0, 640.0, 136.0);

        let mapped = map_frame_between_work_areas(saved, WORK_AREA, target, 136.0);

        assert_eq!(mapped, frame(-1184.0, 752.0, 640.0, 136.0));
    }

    #[test]
    fn active_space_mapping_fits_a_smaller_target_completely() {
        let target = LogicalWorkArea {
            x: 1600.0,
            y: 100.0,
            width: 520.0,
            height: 200.0,
            coordinate_scale: 1.0,
        };
        let saved = frame(400.0, 400.0, 640.0, 300.0);

        let mapped = map_frame_between_work_areas(saved, WORK_AREA, target, 136.0);

        assert_eq!(mapped, frame(1600.0, 100.0, 520.0, 200.0));
    }

    #[test]
    fn same_screen_mapping_still_fits_a_stale_offscreen_frame() {
        let stale = frame(1400.0, 900.0, 900.0, 400.0);

        let mapped = map_frame_between_work_areas(stale, WORK_AREA, WORK_AREA, 136.0);

        assert_eq!(mapped, frame(612.0, 582.0, 900.0, 400.0));
    }

    #[test]
    fn matching_presentation_geometry_is_not_a_user_move() {
        let saved = frame(400.0, 700.0, 640.0, 136.0);
        let mut state = state_with_frame(saved);
        state.presentation_frame = Some(frame(-1200.0, 700.0, 640.0, 136.0));

        assert!(!adopt_observed_frame(
            &mut state,
            frame(-1200.0, 700.0, 640.0, 136.0)
        ));
        assert_eq!(state.user_frame, saved);
        assert!(state.presentation_frame.is_some());
    }

    #[test]
    fn return_to_saved_screen_clears_the_runtime_override_after_settle() {
        let saved = frame(400.0, 700.0, 640.0, 136.0);
        let mut state = state_with_frame(saved);
        state.presentation_frame = Some(saved);

        assert!(!adopt_observed_frame(&mut state, saved));
        assert_eq!(state.user_frame, saved);
        assert_eq!(state.presentation_frame, None);
    }

    #[test]
    fn single_screen_follow_does_not_create_a_runtime_override() {
        let saved = frame(400.0, 700.0, 640.0, 136.0);
        let mut state = state_with_frame(saved);

        assert!(!update_presentation_frame(&mut state, saved));
        assert_eq!(state.presentation_frame, None);
    }

    #[test]
    fn returning_from_another_screen_waits_for_the_matching_native_event() {
        let saved = frame(400.0, 700.0, 640.0, 136.0);
        let followed = frame(-1200.0, 700.0, 640.0, 136.0);
        let mut state = state_with_frame(saved);
        state.presentation_frame = Some(followed);

        assert!(update_presentation_frame(&mut state, saved));
        assert_eq!(state.presentation_frame, Some(saved));

        assert!(!adopt_observed_frame(&mut state, saved));
        assert_eq!(state.presentation_frame, None);
    }

    #[test]
    fn manual_move_promotes_a_followed_frame_to_user_state() {
        let mut state = state_with_frame(frame(400.0, 700.0, 640.0, 136.0));
        state.presentation_frame = Some(frame(-1200.0, 700.0, 640.0, 136.0));
        let moved = frame(-1160.0, 680.0, 640.0, 136.0);

        assert!(adopt_observed_frame(&mut state, moved));
        assert_eq!(state.user_frame, moved);
        assert_eq!(state.presentation_frame, None);
    }

    #[test]
    fn collapsed_presentation_compares_against_the_visual_bar() {
        let mut state = state_with_frame(frame(400.0, 700.0, 640.0, 136.0));
        state.mode = OverlayMode::Collapsed;
        state.presentation_frame = Some(frame(-1200.0, 700.0, 640.0, 136.0));

        assert!(presentation_frame_matches_observed(
            &state,
            &frame(-1200.0, 700.0, 280.0, 54.0)
        ));
    }

    #[test]
    fn collapsed_manual_move_preserves_the_expanded_size() {
        let mut state = state_with_frame(frame(400.0, 700.0, 640.0, 136.0));
        state.mode = OverlayMode::Collapsed;
        state.presentation_frame = Some(frame(-1200.0, 700.0, 640.0, 136.0));

        assert!(adopt_observed_frame(
            &mut state,
            frame(-1100.0, 680.0, 280.0, 54.0)
        ));
        assert_eq!(state.user_frame, frame(-1100.0, 680.0, 640.0, 136.0));
        assert_eq!(state.presentation_frame, None);
    }

    #[test]
    fn active_space_follow_never_overwrites_a_live_resize() {
        let mut state = state_with_frame(frame(400.0, 700.0, 640.0, 136.0));
        assert!(active_space_follow_allowed(&state));

        state.resize_drag = Some(ResizeRegion::BottomRight);

        assert!(!active_space_follow_allowed(&state));
    }

    #[test]
    fn newer_space_follow_cancels_an_older_resize_animation() {
        let mut state = state_with_frame(frame(400.0, 700.0, 640.0, 136.0));
        let animation = OverlayApplySnapshot::from(&state);

        state.presentation_frame = Some(frame(-1200.0, 700.0, 640.0, 136.0));

        assert!(!apply_snapshot_is_current(&state, animation));
    }

    #[test]
    fn native_drag_marker_does_not_strand_an_animation_mid_size() {
        let mut state = state_with_frame(frame(400.0, 700.0, 640.0, 136.0));
        state.mode = OverlayMode::Collapsed;
        let animation = OverlayApplySnapshot::from(&state);

        state.native_drag_start = Some((400.0, 700.0));

        assert!(apply_snapshot_is_current(&state, animation));
    }

    #[test]
    fn resize_end_cancels_a_queued_move_from_the_previous_gesture() {
        let mut state = state_with_frame(frame(400.0, 700.0, 640.0, 136.0));
        state.resize_drag = Some(ResizeRegion::BottomRight);
        state.resize_start = Some(ResizeGesture {
            pointer: (400.0, 700.0),
            frame: state.user_frame,
            work_area: WORK_AREA,
        });
        let queued_move = OverlayApplySnapshot::from(&state);

        state.resize_drag = None;
        state.resize_start = None;

        assert!(!apply_snapshot_is_current(&state, queued_move));
    }

    #[test]
    fn native_drag_intent_ignores_intermediate_collapse_size() {
        let start = (-1200.0, 700.0);
        let intermediate_animation_frame = frame(-1200.0, 700.0, 430.0, 82.0);

        assert!(!native_drag_changed_position(
            start,
            &intermediate_animation_frame
        ));

        let actually_moved = frame(-1160.0, 680.0, 430.0, 82.0);
        assert!(native_drag_changed_position(start, &actually_moved));
    }

    #[test]
    fn promoted_drag_is_not_resynced_over_its_fitted_user_frame() {
        let state = state_with_frame(frame(872.0, 400.0, 640.0, 136.0));
        let reconciliation = NativeDragReconcile {
            promoted: true,
            fit_changed: true,
        };

        assert!(!should_sync_before_collapse(reconciliation, &state));
    }

    #[test]
    fn frame_clamp_honors_negative_work_area_origin() {
        let left = LogicalWorkArea {
            x: -1728.0,
            y: -120.0,
            width: 1728.0,
            height: 1080.0,
            coordinate_scale: 1.0,
        };
        let mut saved = frame(-2200.0, -300.0, 640.0, 136.0);

        clamp_frame_origin_to_work_area(&mut saved, left);

        assert_eq!(saved.x, -1728.0);
        assert_eq!(saved.y, -120.0);
    }

    #[test]
    fn settled_frame_returns_fully_inside_the_right_edge() {
        let mut moved = frame(1_200.0, 400.0, 640.0, 136.0);

        assert!(fit_frame_to_work_area(&mut moved, WORK_AREA, 136.0));

        assert_eq!(moved, frame(872.0, 400.0, 640.0, 136.0));
    }

    #[test]
    fn settled_oversized_frame_shrinks_before_origin_is_clamped() {
        let narrow = LogicalWorkArea {
            x: 100.0,
            y: 50.0,
            width: 520.0,
            height: 320.0,
            coordinate_scale: 1.0,
        };
        let mut oversized = frame(480.0, 260.0, 1_200.0, 600.0);

        assert!(fit_frame_to_work_area(&mut oversized, narrow, 136.0));

        assert_eq!(oversized, frame(100.0, 50.0, 520.0, 320.0));
    }

    #[test]
    fn settled_frame_respects_a_negative_origin_secondary_display() {
        let left = LogicalWorkArea {
            x: -1728.0,
            y: -120.0,
            width: 1728.0,
            height: 1080.0,
            coordinate_scale: 1.0,
        };
        let mut moved = frame(-400.0, 900.0, 640.0, 136.0);

        assert!(fit_frame_to_work_area(&mut moved, left, 136.0));

        assert_eq!(moved, frame(-640.0, 824.0, 640.0, 136.0));
    }

    #[test]
    fn settled_frame_that_is_already_visible_stays_exactly_unchanged() {
        let mut visible = frame(400.0, 300.0, 640.0, 136.0);

        assert!(!fit_frame_to_work_area(&mut visible, WORK_AREA, 136.0));

        assert_eq!(visible, frame(400.0, 300.0, 640.0, 136.0));
    }

    #[test]
    fn missing_saved_monitor_falls_back_to_primary_work_area() {
        let primary = LogicalWorkArea {
            x: 0.0,
            y: 24.0,
            width: 1512.0,
            height: 958.0,
            coordinate_scale: 1.0,
        };
        let saved = frame(-2200.0, 700.0, 640.0, 136.0);

        assert_eq!(
            choose_work_area_for_frame(&saved, [primary], Some(primary)),
            Some(primary)
        );
    }

    #[test]
    fn default_origin_uses_work_area_origin_and_bottom_inset() {
        let upper = LogicalWorkArea {
            x: -1280.0,
            y: -1024.0,
            width: 1280.0,
            height: 984.0,
            coordinate_scale: 1.0,
        };

        assert_eq!(
            default_overlay_origin_in_work_area(640.0, 136.0, upper),
            (-960.0, -248.0)
        );
    }

    #[test]
    fn panel_grows_upward_when_it_would_overflow_bottom() {
        let geometry = overlay_control_geometry(
            OverlayControlMode::Panel,
            400.0,
            900.0,
            180.0,
            356.0,
            WORK_AREA,
        );
        assert_eq!(geometry.y, 900.0 + 30.0 - 356.0);
        assert_eq!(geometry.width, 280.0);
        assert_eq!(geometry.height, 356.0);
    }

    #[test]
    fn panel_keeps_its_anchor_when_there_is_room_below() {
        let geometry = overlay_control_geometry(
            OverlayControlMode::Panel,
            400.0,
            300.0,
            180.0,
            356.0,
            WORK_AREA,
        );
        assert_eq!(geometry.y, 300.0);
    }

    #[test]
    fn control_geometry_honors_negative_monitor_origins() {
        let work_area = LogicalWorkArea {
            x: -1728.0,
            y: -120.0,
            width: 1728.0,
            height: 1080.0,
            coordinate_scale: 1.0,
        };
        let geometry = overlay_control_geometry(
            OverlayControlMode::Island,
            -1800.0,
            -200.0,
            180.0,
            356.0,
            work_area,
        );
        assert_eq!(geometry.x, -1720.0);
        assert_eq!(geometry.y, -112.0);
    }

    #[test]
    fn panel_clamps_inside_right_edge_and_short_work_area() {
        let work_area = LogicalWorkArea {
            x: 100.0,
            y: 50.0,
            width: 320.0,
            height: 180.0,
            coordinate_scale: 1.0,
        };
        let geometry = overlay_control_geometry(
            OverlayControlMode::Panel,
            400.0,
            160.0,
            180.0,
            520.0,
            work_area,
        );
        assert_eq!(geometry.x, 132.0);
        assert_eq!(geometry.width, 280.0);
        assert_eq!(geometry.y, 58.0);
        assert_eq!(geometry.height, 164.0);
    }

    #[test]
    fn island_hugs_measured_content_without_changing_the_panel_width_or_left_anchor() {
        for width in [148.0, 188.0, 312.0] {
            let island = overlay_control_geometry(
                OverlayControlMode::Island,
                400.0,
                300.0,
                width,
                356.0,
                WORK_AREA,
            );
            let panel = overlay_control_geometry(
                OverlayControlMode::Panel,
                400.0,
                300.0,
                width,
                356.0,
                WORK_AREA,
            );
            assert_eq!((island.x, island.y, island.width), (400.0, 300.0, width));
            assert_eq!((panel.x, panel.y, panel.width), (400.0, 300.0, 280.0));
        }
    }

    #[test]
    fn measured_width_updates_keep_hidden_and_panel_modes_and_dismiss_generation() {
        let state = OverlayControlState::default();
        assert!(state.set_island_width(148.0));
        assert_eq!(state.snapshot(), (OverlayControlMode::Hidden, 270.0, 0));
        assert_eq!(state.dimensions(), (148.0, 270.0));
        assert!(state.set_mode(OverlayControlMode::Panel));
        assert!(state.set_panel_height(356.0));
        assert!(state.set_island_width(312.0));
        assert!(!state.set_island_width(312.0));
        assert_eq!(state.snapshot(), (OverlayControlMode::Panel, 356.0, 1));
        assert_eq!(state.dimensions(), (312.0, 356.0));
    }

    #[test]
    fn island_measurements_are_finite_rounded_and_bounded() {
        assert_eq!(measured_island_width(173.2), Some(174.0));
        assert_eq!(measured_island_width(2.0), Some(80.0));
        assert_eq!(measured_island_width(10000.0), Some(512.0));
        assert_eq!(measured_island_width(f64::NAN), None);
        assert_eq!(measured_island_width(f64::INFINITY), None);
    }

    #[test]
    fn island_width_clamps_to_short_screens_without_using_panel_width() {
        let work_area = LogicalWorkArea {
            x: 100.0,
            y: 50.0,
            width: 320.0,
            height: 180.0,
            coordinate_scale: 1.0,
        };
        let island = overlay_control_geometry(
            OverlayControlMode::Island,
            400.0,
            160.0,
            180.0,
            520.0,
            work_area,
        );
        assert_eq!((island.x, island.y, island.width), (232.0, 160.0, 180.0));
        let narrow = LogicalWorkArea {
            width: 100.0,
            ..work_area
        };
        let island = overlay_control_geometry(
            OverlayControlMode::Island,
            400.0,
            160.0,
            312.0,
            520.0,
            narrow,
        );
        assert_eq!((island.x, island.y, island.width), (108.0, 160.0, 84.0));
    }

    #[test]
    fn windows_bottom_taskbar_opens_tray_panel_above_it() {
        let monitor = PhysicalScreenArea {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        };
        let work_area = PhysicalScreenArea {
            height: 1040.0,
            ..monitor
        };
        let tray = PhysicalScreenArea {
            x: 1730.0,
            y: 1040.0,
            width: 24.0,
            height: 40.0,
        };

        assert_eq!(
            nearest_tray_screen_edge(tray, monitor),
            TrayScreenEdge::Bottom
        );
        assert_eq!(
            tray_panel_origin(tray, 640.0, 820.0, monitor, work_area, 16.0),
            (1264.0, 204.0)
        );
    }

    #[test]
    fn windows_top_taskbar_opens_tray_panel_below_it() {
        let monitor = PhysicalScreenArea {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        };
        let work_area = PhysicalScreenArea {
            y: 40.0,
            height: 1040.0,
            ..monitor
        };
        let tray = PhysicalScreenArea {
            x: 1730.0,
            y: 0.0,
            width: 24.0,
            height: 40.0,
        };

        assert_eq!(nearest_tray_screen_edge(tray, monitor), TrayScreenEdge::Top);
        assert_eq!(
            tray_panel_origin(tray, 640.0, 820.0, monitor, work_area, 16.0),
            (1264.0, 56.0)
        );
    }

    #[test]
    fn windows_vertical_taskbars_open_tray_panel_inward() {
        let monitor = PhysicalScreenArea {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        };
        let left_work_area = PhysicalScreenArea {
            x: 40.0,
            width: 1880.0,
            ..monitor
        };
        let left_tray = PhysicalScreenArea {
            x: 0.0,
            y: 900.0,
            width: 40.0,
            height: 24.0,
        };
        let right_work_area = PhysicalScreenArea {
            width: 1880.0,
            ..monitor
        };
        let right_tray = PhysicalScreenArea {
            x: 1880.0,
            ..left_tray
        };

        assert_eq!(
            tray_panel_origin(left_tray, 640.0, 820.0, monitor, left_work_area, 16.0),
            (56.0, 244.0)
        );
        assert_eq!(
            tray_panel_origin(right_tray, 640.0, 820.0, monitor, right_work_area, 16.0),
            (1224.0, 244.0)
        );
    }

    #[test]
    fn negative_origin_monitor_does_not_invert_bottom_taskbar() {
        let monitor = PhysicalScreenArea {
            x: -1920.0,
            y: -1080.0,
            width: 1920.0,
            height: 1080.0,
        };
        let work_area = PhysicalScreenArea {
            height: 1040.0,
            ..monitor
        };
        let tray = PhysicalScreenArea {
            x: -190.0,
            y: -40.0,
            width: 24.0,
            height: 40.0,
        };

        assert_eq!(
            nearest_tray_screen_edge(tray, monitor),
            TrayScreenEdge::Bottom
        );
        assert_eq!(
            tray_panel_origin(tray, 640.0, 820.0, monitor, work_area, 16.0),
            (-656.0, -876.0)
        );
    }
}

#[cfg(all(test, target_os = "macos"))]
mod macos_presentation_tests {
    use super::{overlay_collection_behavior, overlay_window_level};
    use objc2_app_kit::{
        NSScreenSaverWindowLevel, NSStatusWindowLevel, NSWindowCollectionBehavior,
    };

    #[test]
    fn overlay_level_covers_true_full_screen_media() {
        assert_eq!(overlay_window_level(), NSScreenSaverWindowLevel);
        assert!(overlay_window_level() > NSStatusWindowLevel);
    }

    #[test]
    fn overlay_can_join_other_apps_full_screen_spaces() {
        let current = NSWindowCollectionBehavior::MoveToActiveSpace
            | NSWindowCollectionBehavior::Managed
            | NSWindowCollectionBehavior::ParticipatesInCycle
            | NSWindowCollectionBehavior::Primary
            | NSWindowCollectionBehavior::FullScreenPrimary;

        let behavior = overlay_collection_behavior(current);

        assert!(behavior.contains(NSWindowCollectionBehavior::CanJoinAllSpaces));
        assert!(behavior.contains(NSWindowCollectionBehavior::Stationary));
        assert!(behavior.contains(NSWindowCollectionBehavior::IgnoresCycle));
        assert!(behavior.contains(NSWindowCollectionBehavior::CanJoinAllApplications));
        assert!(behavior.contains(NSWindowCollectionBehavior::FullScreenAuxiliary));
        assert!(!behavior.contains(NSWindowCollectionBehavior::MoveToActiveSpace));
        assert!(!behavior.contains(NSWindowCollectionBehavior::Managed));
        assert!(!behavior.contains(NSWindowCollectionBehavior::Transient));
        assert!(!behavior.contains(NSWindowCollectionBehavior::ParticipatesInCycle));
        assert!(!behavior.contains(NSWindowCollectionBehavior::Primary));
        assert!(!behavior.contains(NSWindowCollectionBehavior::Auxiliary));
        assert!(!behavior.contains(NSWindowCollectionBehavior::FullScreenPrimary));
        assert!(!behavior.contains(NSWindowCollectionBehavior::FullScreenNone));
    }
}
