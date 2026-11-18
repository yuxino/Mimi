//! Native panel classes, presentation cache and platform window adaptation.

#[cfg(any(target_os = "macos", target_os = "linux"))]
use crate::pipeline_log;
use crate::settings_store::OverlayFrame;
use tauri::{AppHandle, Manager};

#[cfg(target_os = "linux")]
use super::controls::{
    embedded_control_geometry, OverlayControlHost, OverlayControlMode, OverlayControlState,
    OverlayControlWindowManager,
};
#[cfg(target_os = "linux")]
use super::linux_input_region;
#[cfg(target_os = "macos")]
use super::macos_pointer;
#[cfg(target_os = "windows")]
use super::windows_pointer;
#[cfg(target_os = "windows")]
use super::work_area::work_area_for_saved_frame;

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
pub(in crate::windows) enum OverlayPanelKind {
    Subtitles,
    Controls,
}

#[cfg(target_os = "macos")]
pub(in crate::windows) fn convert_overlay_to_panel(
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
pub(in crate::windows) fn convert_overlay_to_panel(
    _window: &tauri::WebviewWindow,
    _kind: OverlayPanelKind,
) -> Result<(), ()> {
    Ok(())
}

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
    pub(in crate::windows) fn apply_click_through_if_changed(
        &self,
        window: &tauri::WebviewWindow,
        enabled: bool,
    ) {
        let mut current = self.0.lock().unwrap();
        if *current == Some(enabled) {
            return;
        }
        if window.set_ignore_cursor_events(enabled).is_ok() {
            *current = Some(enabled);
            #[cfg(target_os = "macos")]
            if enabled {
                macos_pointer::clear_overlay(window.app_handle());
            }
        }
    }

    #[cfg(target_os = "linux")]
    pub(in crate::windows) fn apply_click_through_if_changed(
        &self,
        window: &tauri::WebviewWindow,
        enabled: bool,
    ) {
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
                reconcile_linux_overlay_input(&app, &native, false);
            })
            .is_err()
        {
            pipeline_log!("overlay input region failed label=main_thread_unavailable");
        }
    }

    pub(in crate::windows) fn invalidate(&self) {
        #[cfg(not(target_os = "linux"))]
        {
            *self.0.lock().unwrap() = None;
        }
        #[cfg(target_os = "linux")]
        self.0.lock().unwrap().invalidate();
    }
}

#[cfg(target_os = "linux")]
pub(in crate::windows) fn configure_overlay_for_software_gl(window: &tauri::WebviewWindow) {
    use webkit2gtk::{HardwareAccelerationPolicy, SettingsExt, WebViewExt};

    // On X11 with software GL, WebKit's accelerated path delayed visible
    // subtitles. Keep the change local to the subtitle WebView and require
    // an explicit software-rendering launch on an X11 session.
    if std::env::var("XDG_SESSION_TYPE").as_deref() != Ok("x11")
        || !matches!(
            std::env::var("LIBGL_ALWAYS_SOFTWARE").as_deref(),
            Ok("1" | "true")
        )
    {
        return;
    }

    let _ = window.with_webview(|platform| {
        if let Some(settings) = WebViewExt::settings(&platform.inner()) {
            settings.set_hardware_acceleration_policy(HardwareAccelerationPolicy::Never);
            pipeline_log!(
                "overlay software GL acceleration disabled={}",
                settings.hardware_acceleration_policy() == HardwareAccelerationPolicy::Never
            );
        }
    });
}

/// Reasserts presentation-level window behavior whenever the overlay is
/// created or shown. macOS needs both `CanJoinAllApplications` and
/// `FullScreenAuxiliary` in addition to Tauri's all-workspaces flag to
/// accompany another app's full-screen window instead of remaining on the
/// previous Space. macOS composites true full-screen video above ordinary
/// floating and status windows, so the subtitle surface uses the screen-saver
/// window level recommended for cross-application media overlays.
pub(in crate::windows) fn configure_overlay_window(window: &tauri::WebviewWindow) {
    configure_overlay_window_impl(window, false);
}

/// Orders an overlay onscreen without making it key. Tauri's ordinary macOS
/// `show()` path uses `makeKeyAndOrderFront`, which can activate mimi and pull
/// the user out of the media app's full-screen Space.
pub(in crate::windows) fn show_overlay_window(window: &tauri::WebviewWindow) {
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
pub(in crate::windows) fn show_overlay_control_window(
    app: &AppHandle,
    window: &tauri::WebviewWindow,
) {
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
#[cfg(target_os = "macos")]
pub(in crate::windows) fn reassert_overlay_window_on_active_space(window: &tauri::WebviewWindow) {
    configure_overlay_window_impl(window, true);
}

fn configure_overlay_window_impl(window: &tauri::WebviewWindow, order_front: bool) {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = order_front;
        let _ = window.set_always_on_top(true);
    }

    #[cfg(target_os = "windows")]
    windows_pointer::activate(window);

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
            macos_pointer::install(&window_for_main);
            macos_pointer::refresh_after_window_update(window_for_main.app_handle());
        }
    });
}

/// Main-thread-only parent restoration shared by initial presentation and
/// keyboard release. AppKit detaches a child when orderOut hides it.
#[cfg(target_os = "macos")]
pub(in crate::windows) fn attach_overlay_control_parent_macos(
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

/// Sets a native desktop origin without ever combining global coordinates
/// from monitors that use different DPI scales. Window dimensions remain in
/// logical pixels so the overlay keeps the same visual size on every screen.
pub(in crate::windows) fn set_desktop_position(window: &tauri::WebviewWindow, x: f64, y: f64) {
    #[cfg(target_os = "windows")]
    let _ = window.set_position(tauri::PhysicalPosition::new(
        x.round() as i32,
        y.round() as i32,
    ));

    #[cfg(not(target_os = "windows"))]
    let _ = window.set_position(tauri::LogicalPosition::new(x, y));
}

pub(in crate::windows) fn set_desktop_size(
    window: &tauri::WebviewWindow,
    width: f64,
    height: f64,
    coordinate_scale: f64,
) {
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

pub(in crate::windows) fn desktop_scale_for_frame(
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
pub(in crate::windows) fn overlay_frame_from_window(app: &AppHandle) -> Option<OverlayFrame> {
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
pub(in crate::windows) fn overlay_frame_from_window(app: &AppHandle) -> Option<OverlayFrame> {
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
                let (x, y) = if OverlayControlWindowManager::is_embedded(native_window.app_handle())
                {
                    // Wayland reports a synthetic origin, not the compositor's
                    // placement. Preserve the saved origin while observing size.
                    let app_state = native_window
                        .app_handle()
                        .state::<crate::commands::AppState>();
                    let frame = app_state.overlay.lock().unwrap().effective_frame();
                    (frame.x, frame.y)
                } else {
                    let (x, y) = window.position();
                    (x as f64, y as f64)
                };
                let (width, height) = window.size();
                Some(OverlayFrame {
                    x,
                    y,
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
pub(in crate::windows) fn reconcile_linux_overlay_input(
    app: &AppHandle,
    window: &impl gtk::glib::IsA<gtk::Window>,
    force: bool,
) {
    use gtk::prelude::*;
    let control = if OverlayControlWindowManager::is_embedded(app) {
        let (width, height) = window.as_ref().size();
        app.try_state::<OverlayControlState>().and_then(|state| {
            let (island_width, panel_height, _) = state.dimensions();
            embedded_control_geometry(state.mode(), island_width, panel_height, width, height).map(
                |geometry| {
                    gtk::cairo::RectangleInt::new(
                        geometry.x as i32,
                        geometry.y as i32,
                        geometry.width as i32,
                        geometry.height as i32,
                    )
                },
            )
        })
    } else {
        None
    };
    let popups: Vec<_> = app
        .try_state::<OverlayControlHost>()
        .and_then(|host| {
            (host.embedded && OverlayControlWindowManager::mode(app) == OverlayControlMode::Panel)
                .then(|| *host.popups.lock().unwrap())
        })
        .unwrap_or_default()
        .into_iter()
        .flatten()
        .map(|bounds| {
            let (width, height) = window.as_ref().size();
            let x = bounds.x.floor().clamp(0.0, width as f64);
            let y = bounds.y.floor().clamp(0.0, height as f64);
            let right = (bounds.x + bounds.width).ceil().clamp(x, width as f64);
            let bottom = (bounds.y + bounds.height).ceil().clamp(y, height as f64);
            gtk::cairo::RectangleInt::new(
                x as i32,
                y as i32,
                (right - x) as i32,
                (bottom - y) as i32,
            )
        })
        .collect();
    if let Some(presentation) = app.try_state::<OverlayPresentationState>() {
        presentation.0.lock().unwrap().reconcile(force, |enabled| {
            linux_input_region::apply_with_control(window, enabled, control, &popups);
        });
    }
}

#[cfg(target_os = "linux")]
pub(in crate::windows) fn follow_linux_overlay_on_map(window: &tauri::WebviewWindow) {
    use gtk::prelude::*;
    let native_window = window.clone();
    let app = window.app_handle().clone();
    let _ = window.run_on_main_thread(move || {
        if let Ok(window) = native_window.gtk_window() {
            let input_app = app.clone();
            linux_input_region::restore_on_surface_change(&window, move |window| {
                reconcile_linux_overlay_input(&input_app, window, true);
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

#[cfg(all(test, target_os = "macos"))]
mod tests;
