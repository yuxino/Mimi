//! Transient control-window state, native bounds, keyboard focus and WebView zoom.

use crate::pipeline_log;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use super::dev_title;
use super::native::{
    configure_overlay_window, convert_overlay_to_panel, show_overlay_control_window,
    OverlayPanelKind,
};
#[cfg(target_os = "linux")]
use super::native::{overlay_frame_from_window, reconcile_linux_overlay_input};
#[cfg(not(target_os = "linux"))]
use super::native::{set_desktop_position, set_desktop_size};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use super::remove_overlay_pointer_tracking;
use super::work_area::{logical_work_area, LogicalWorkArea};
#[cfg(target_os = "macos")]
use super::{macos_control_dismiss, macos_pointer};

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
    webview_zoom: f64,
    generation: u64,
}

/// Tauri-managed state for the overlay control child window. Keeping this
/// separate from [`super::overlay::OverlayState`] ensures transient panel resizing can never
/// mutate or persist the user's subtitle frame.
#[derive(Debug)]
pub struct OverlayControlState(std::sync::Mutex<OverlayControlStateInner>);

impl Default for OverlayControlState {
    fn default() -> Self {
        Self(std::sync::Mutex::new(OverlayControlStateInner {
            mode: OverlayControlMode::Hidden,
            island_width: OverlayControlWindowManager::DEFAULT_ISLAND_WIDTH,
            panel_height: OverlayControlWindowManager::DEFAULT_PANEL_HEIGHT,
            webview_zoom: 1.0,
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

    fn set_panel_height(&self, height: f64, zoom: f64) -> bool {
        let mut state = self.0.lock().unwrap();
        if (state.panel_height - height).abs() < 0.5 && (state.webview_zoom - zoom).abs() < 0.0001 {
            return false;
        }
        state.panel_height = height;
        state.webview_zoom = zoom;
        true
    }

    pub(in crate::windows) fn dimensions(&self) -> (f64, f64, f64) {
        let state = self.0.lock().unwrap();
        (state.island_width, state.panel_height, state.webview_zoom)
    }

    fn set_island_width(&self, width: f64, zoom: f64) -> bool {
        let mut state = self.0.lock().unwrap();
        if (state.island_width - width).abs() < 0.5 && (state.webview_zoom - zoom).abs() < 0.0001 {
            return false;
        }
        state.island_width = width;
        state.webview_zoom = zoom;
        true
    }

    fn cancel_scheduled_dismiss(&self) {
        let mut state = self.0.lock().unwrap();
        state.generation = state.generation.wrapping_add(1);
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct OverlayControlGeometry {
    pub(in crate::windows) x: f64,
    pub(in crate::windows) y: f64,
    pub(in crate::windows) width: f64,
    pub(in crate::windows) height: f64,
}

const OVERLAY_CONTROL_MODE_EVENT: &str = "overlay-control-mode";

/// Child/owned control surface for the subtitle canvas. The native bounds
/// always match the visible surface, so no transparent shadow margin steals
/// clicks from the video underneath.
pub struct OverlayControlWindowManager;

/// Native Wayland toplevels have no application-controlled global position.
/// Render controls in the subtitle WebView there, using the actual GDK backend
/// rather than XDG_SESSION_TYPE (a Wayland desktop may run Mimi via XWayland).
pub struct OverlayControlHost {
    pub(in crate::windows) embedded: bool,
    pub(in crate::windows) popups: std::sync::Mutex<[Option<OverlayControlGeometry>; 2]>,
}

impl OverlayControlHost {
    pub fn detect() -> Self {
        #[cfg(target_os = "linux")]
        {
            use gtk::prelude::*;
            Self {
                embedded: gtk::gdk::Display::default()
                    .is_some_and(|display| display.type_().name() == "GdkWaylandDisplay"),
                popups: std::sync::Mutex::new([None; 2]),
            }
        }
        #[cfg(not(target_os = "linux"))]
        Self {
            embedded: false,
            popups: std::sync::Mutex::new([None; 2]),
        }
    }
}

impl OverlayControlWindowManager {
    pub fn is_embedded(app: &AppHandle) -> bool {
        app.try_state::<OverlayControlHost>()
            .is_some_and(|host| host.embedded)
    }

    pub fn set_popup_bounds(
        app: &AppHandle,
        bounds: Option<OverlayControlGeometry>,
        notification: Option<OverlayControlGeometry>,
    ) {
        let Some(host) = app.try_state::<OverlayControlHost>() else {
            return;
        };
        if !host.embedded {
            return;
        }
        *host.popups.lock().unwrap() = [bounds, notification].map(|bounds| {
            bounds.filter(|bounds| {
                Self::mode(app) == OverlayControlMode::Panel
                    && [bounds.x, bounds.y, bounds.width, bounds.height]
                        .into_iter()
                        .all(f64::is_finite)
                    && bounds.width > 0.0
                    && bounds.height > 0.0
            })
        });
        Self::apply_geometry(app, Self::mode(app));
    }

    pub const DEFAULT_ISLAND_WIDTH: f64 = Self::MAX_ISLAND_WIDTH;
    pub const ISLAND_HEIGHT: f64 = 30.0;
    const MIN_ISLAND_WIDTH: f64 = 80.0;
    // Match the CSS capsule cap and leave all six actions usable at 360px.
    const MAX_ISLAND_WIDTH: f64 = 156.0;
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
        if Self::is_embedded(app) {
            return;
        }
        if app.get_webview_window("overlay-control").is_some() {
            return;
        }
        let Some(overlay) = app.get_webview_window("overlay") else {
            pipeline_log!("overlay control failed label=missing_parent");
            return;
        };
        let builder =
            WebviewWindowBuilder::new(app, "overlay-control", WebviewUrl::App("index.html".into()))
                .title(dev_title("Mimi"))
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
    /// surface. Position lock preserves only the compact recovery island.
    pub fn sync_presentation(
        app: &AppHandle,
        is_active: bool,
        is_collapsed: bool,
        click_through: bool,
        is_immersive: bool,
    ) {
        let Some(state) = app.try_state::<OverlayControlState>() else {
            return;
        };
        let current = state.mode();
        let next = control_mode_for_presentation(
            current,
            is_active,
            is_collapsed,
            click_through,
            is_immersive,
        );
        let mode_changed = state.set_mode(next);
        let visible_surface_needs_restore = !mode_changed
            && next != OverlayControlMode::Hidden
            && !Self::is_embedded(app)
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
    pub fn set_panel_height(app: &AppHandle, height: f64, pixel_ratio: f64) {
        if !height.is_finite() {
            return;
        }
        let height = height.clamp(Self::MIN_PANEL_HEIGHT, Self::MAX_PANEL_HEIGHT);
        let Some(zoom) = measured_webview_zoom(app, pixel_ratio) else {
            return;
        };
        let Some(state) = app.try_state::<OverlayControlState>() else {
            return;
        };
        if state.set_panel_height(height, zoom) && state.mode() == OverlayControlMode::Panel {
            Self::apply_geometry(app, OverlayControlMode::Panel);
        }
    }

    /// Caches the collapsed capsule's intrinsic width independently of the
    /// panel. Hidden and expanded surfaces only remember this measurement;
    /// a late report must neither reveal a hidden window nor shrink a panel.
    pub fn set_island_width(app: &AppHandle, width: f64, pixel_ratio: f64) {
        let Some(width) = measured_island_width(width) else {
            return;
        };
        let current_app = app.clone();
        let _ = app.run_on_main_thread(move || {
            let Some(zoom) = measured_webview_zoom(&current_app, pixel_ratio) else {
                return;
            };
            let Some(state) = current_app.try_state::<OverlayControlState>() else {
                return;
            };
            if state.set_island_width(width, zoom) && state.mode() == OverlayControlMode::Island {
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
        if Self::is_embedded(app) {
            if mode != OverlayControlMode::Panel {
                if let Some(host) = app.try_state::<OverlayControlHost>() {
                    *host.popups.lock().unwrap() = [None; 2];
                }
            }
            if let Some(window) = app.get_webview_window("overlay") {
                let _ = window.emit(OVERLAY_CONTROL_MODE_EVENT, mode);
                Self::apply_geometry(app, mode);
            }
            return;
        }
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
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            remove_overlay_pointer_tracking(app, window.label());
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
        #[cfg(target_os = "linux")]
        if Self::is_embedded(app) {
            let current_app = app.clone();
            let _ = app.run_on_main_thread(move || {
                let Some(window) = current_app.get_webview_window("overlay") else {
                    return;
                };
                let Ok(native) = window.gtk_window() else {
                    return;
                };
                reconcile_linux_overlay_input(&current_app, &native, true);
            });
            return true;
        }
        let Some(window) = app.get_webview_window("overlay-control") else {
            return false;
        };
        let Some((anchor_x, anchor_y, work_area)) = Self::overlay_anchor(app) else {
            return false;
        };
        let (island_width, panel_height, zoom) = app
            .try_state::<OverlayControlState>()
            .map(|state| state.dimensions())
            .unwrap_or((Self::DEFAULT_ISLAND_WIDTH, Self::DEFAULT_PANEL_HEIGHT, 1.0));
        #[cfg(target_os = "windows")]
        let work_area = work_area.with_webview_zoom(zoom);
        #[cfg(not(target_os = "windows"))]
        let _ = zoom;
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
            #[cfg(target_os = "macos")]
            macos_pointer::refresh_after_window_update(app);
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
    click_through: bool,
    is_immersive: bool,
) -> OverlayControlMode {
    if !is_active || is_collapsed || is_immersive {
        OverlayControlMode::Hidden
    } else if current == OverlayControlMode::Panel && !click_through {
        OverlayControlMode::Panel
    } else {
        OverlayControlMode::Island
    }
}

fn measured_webview_zoom(app: &AppHandle, pixel_ratio: f64) -> Option<f64> {
    #[cfg(target_os = "windows")]
    {
        let native_scale = app
            .get_webview_window("overlay-control")?
            .scale_factor()
            .ok()?;
        windows_webview_zoom(pixel_ratio, native_scale)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = app;
        Some(unscaled_webview_zoom(pixel_ratio))
    }
}

#[cfg(any(target_os = "windows", test))]
fn windows_webview_zoom(pixel_ratio: f64, native_scale: f64) -> Option<f64> {
    if !pixel_ratio.is_finite()
        || !(0.25..=16.0).contains(&pixel_ratio)
        || !native_scale.is_finite()
        || !(0.25..=16.0).contains(&native_scale)
    {
        return None;
    }
    Some(pixel_ratio / native_scale)
}

/// Only Windows applies WebView text zoom to native control geometry. Ignore
/// the extra measurement entirely elsewhere, preserving their size updates.
#[cfg(any(not(target_os = "windows"), test))]
fn unscaled_webview_zoom(_pixel_ratio: f64) -> f64 {
    1.0
}

fn measured_island_width(width: f64) -> Option<f64> {
    width.is_finite().then(|| {
        width.ceil().clamp(
            OverlayControlWindowManager::MIN_ISLAND_WIDTH,
            OverlayControlWindowManager::MAX_ISLAND_WIDTH,
        )
    })
}

/// Keep the embedded controls within the canvas; Wayland cannot supply a
/// global work-area origin. The panel scrolls inside these bounds.
#[cfg(any(target_os = "linux", test))]
pub(in crate::windows) fn embedded_control_geometry(
    mode: OverlayControlMode,
    island_width: f64,
    panel_height: f64,
    canvas_width: i32,
    canvas_height: i32,
) -> Option<OverlayControlGeometry> {
    if mode == OverlayControlMode::Hidden {
        return None;
    }
    let width = match mode {
        OverlayControlMode::Panel => OverlayControlWindowManager::PANEL_WIDTH,
        _ => island_width.min(OverlayControlWindowManager::PANEL_WIDTH),
    }
    .min((canvas_width as f64 - 36.0).max(0.0));
    let height = match mode {
        OverlayControlMode::Panel => panel_height,
        _ => OverlayControlWindowManager::ISLAND_HEIGHT,
    }
    .min((canvas_height as f64 - 24.0).max(0.0));
    (width > 0.0 && height > 0.0).then_some(OverlayControlGeometry {
        x: OverlayControlWindowManager::ANCHOR_OFFSET_X,
        y: OverlayControlWindowManager::ANCHOR_OFFSET_Y,
        width,
        height,
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

#[cfg(test)]
mod tests;
