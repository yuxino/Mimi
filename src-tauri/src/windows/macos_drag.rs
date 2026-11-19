//! Anchor an AppKit move to the subtitle root, including presses in its child.

use super::{
    panel_drag_target, OverlayControlMode, OverlayControlWindowManager, OverlayState,
    OverlayWindowManager,
};
use crate::commands::AppState;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSEvent, NSEventType, NSWindow};
use objc2_foundation::NSProcessInfo;
use std::sync::{mpsc, Arc, Mutex};
use tauri::{AppHandle, Manager, WebviewWindow};

#[derive(Clone, Copy)]
enum DragSurface {
    Root,
    Panel,
}

pub(crate) fn start_overlay_drag(
    app: &AppHandle,
    source: &WebviewWindow,
    overlay: &Arc<Mutex<OverlayState>>,
) -> Result<(), String> {
    let surface = match source.label() {
        "overlay" => DragSurface::Root,
        "overlay-control" => DragSurface::Panel,
        _ => return Err("overlay_move_not_allowed".into()),
    };
    if let Some(mtm) = MainThreadMarker::new() {
        return start_on_main(app, source, surface, overlay, mtm);
    }
    let app_on_main = app.clone();
    let source = source.clone();
    let overlay = Arc::clone(overlay);
    let (reply, result) = mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        let mtm = MainThreadMarker::new().expect("AppKit drag runs on the main thread");
        let _ = reply.send(start_on_main(&app_on_main, &source, surface, &overlay, mtm));
    })
    .map_err(|_| "Could not start overlay drag.".to_string())?;
    result
        .recv()
        .map_err(|_| "Could not start overlay drag.".to_string())?
}

fn drag_allowed(
    surface: DragSurface,
    mode: OverlayControlMode,
    locked: bool,
    immersive: bool,
    session_error: bool,
) -> bool {
    match surface {
        DragSurface::Root => session_error || (!locked && !immersive),
        DragSurface::Panel => panel_drag_target(mode, locked, immersive).is_some(),
    }
}

fn start_on_main(
    app: &AppHandle,
    source: &WebviewWindow,
    surface: DragSurface,
    overlay: &Arc<Mutex<OverlayState>>,
    _mtm: MainThreadMarker,
) -> Result<(), String> {
    // IPC delivery can outlive the press. Do not prepare an already released
    // request, or retain the renderer's earlier button state.
    if NSEvent::pressedMouseButtons() & 1 == 0 {
        return Ok(());
    }
    let target = app
        .get_webview_window("overlay")
        .ok_or_else(|| "Could not find the subtitle overlay.".to_string())?;
    let source_pointer = source
        .ns_window()
        .map_err(|_| "Could not start overlay drag.".to_string())?;
    let target_pointer = target
        .ns_window()
        .map_err(|_| "Could not start overlay drag.".to_string())?;
    // SAFETY: Both handles refer to app-owned NSWindows. Their visibility,
    // coordinates and drag are accessed in this one main-thread transaction.
    let (source_window, target_window): (&NSWindow, &NSWindow) =
        unsafe { (&*source_pointer.cast(), &*target_pointer.cast()) };
    if !source_window.isVisible() || !target_window.isVisible() {
        return Ok(());
    }

    // WKWebView IPC need not retain the original mouseDown as currentEvent;
    // it may also belong to the child panel. Never give AppKit that event's
    // location/windowNumber while asking it to move the subtitle root.
    let location = target_window.convertPointFromScreen(NSEvent::mouseLocation());
    let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        NSEventType::LeftMouseDown,
        location,
        NSEvent::modifierFlags_class(),
        NSProcessInfo::processInfo().systemUptime(),
        target_window.windowNumber(),
        None,
        0,
        1,
        1.0,
    )
    .ok_or_else(|| "Could not start overlay drag.".to_string())?;

    if NSEvent::pressedMouseButtons() & 1 == 0 || !OverlayWindowManager::move_start(app, overlay) {
        return Ok(());
    }
    // Read presentation eligibility once, at the native execution boundary.
    // Geometry/monitor reads may have spanned a lock, panel change or release;
    // cancel the pending marker instead of moving a superseded request.
    let state = app.state::<AppState>();
    let preferences = state.settings.preferences();
    if !drag_allowed(
        surface,
        OverlayControlWindowManager::mode(app),
        preferences.overlay_locked,
        preferences.subtitle_blends_with_background,
        matches!(surface, DragSurface::Root) && state.session.status_kind() == "error",
    ) || NSEvent::pressedMouseButtons() & 1 == 0
    {
        OverlayWindowManager::move_cancel(overlay);
        return Ok(());
    }
    target_window.performWindowDragWithEvent(&event);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_recovery_only_lifts_the_subtitle_root_drag_lock() {
        for (locked, immersive) in [(true, false), (false, true), (true, true)] {
            assert!(drag_allowed(
                DragSurface::Root,
                OverlayControlMode::Panel,
                locked,
                immersive,
                true
            ));
            assert!(!drag_allowed(
                DragSurface::Panel,
                OverlayControlMode::Panel,
                locked,
                immersive,
                true
            ));
        }
    }
}
