//! Outside-click dismissal for nonactivating panels, which need not become
//! key and therefore cannot reliably produce a focus-lost event.
use super::controls::{OverlayControlMode, OverlayControlWindowManager};
use super::native::attach_overlay_control_parent_macos;
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSEvent, NSEventMask, NSWindow};
use std::{cell::RefCell, ptr::NonNull};
use tauri::{AppHandle, Manager};

struct Monitors(Vec<Retained<AnyObject>>);
impl Drop for Monitors {
    fn drop(&mut self) {
        for monitor in &self.0 {
            // Tokens are created below and only touched on the main thread.
            unsafe { NSEvent::removeMonitor(monitor) };
        }
    }
}
thread_local! {
    static MONITORS: RefCell<Option<Monitors>> = const { RefCell::new(None) };
}

pub fn sync(app: &AppHandle) {
    let app = app.clone();
    let dispatcher = app.clone();
    let _ = dispatcher.run_on_main_thread(move || {
        MONITORS.with(|slot| {
            let mode = OverlayControlWindowManager::mode(&app);
            if mode != OverlayControlMode::Panel {
                // Release monitor storage before native ordering callbacks.
                let monitors = slot.borrow_mut().take();
                drop(monitors);
                // Escape or a second capsule click can collapse the panel
                // without clicking another app. Release its temporary key
                // status so keyboard events do not stay in the passive island.
                // Read the current mode inside this main-thread transaction:
                // a queued dismissal must not affect a newly reopened panel.
                if let Some(pointer) = app
                    .get_webview_window("overlay-control")
                    .and_then(|window| window.ns_window().ok())
                {
                    let window: &NSWindow = unsafe { &*pointer.cast() };
                    release_passive_keyboard(
                        mode,
                        window.isKeyWindow(),
                        || window.orderOut(None),
                        || {
                            // orderOut transfers key status through AppKit's
                            // window ordering and detaches the native parent.
                            // Restore only the passive island, never Hidden.
                            if let Some(parent) = app
                                .get_webview_window("overlay")
                                .and_then(|window| window.ns_window().ok())
                            {
                                let parent: &NSWindow = unsafe { &*parent.cast() };
                                attach_overlay_control_parent_macos(window, parent);
                                window.orderFrontRegardless();
                            }
                        },
                    );
                }
                return;
            }
            let mut slot = slot.borrow_mut();
            if slot.is_some() {
                return;
            }
            let mask = NSEventMask::LeftMouseDown
                | NSEventMask::RightMouseDown
                | NSEventMask::OtherMouseDown;
            let global_app = app.clone();
            let global = RcBlock::new(move |_: NonNull<NSEvent>| {
                OverlayControlWindowManager::dismiss_panel(&global_app);
            });
            let local = RcBlock::new(move |event: NonNull<NSEvent>| {
                let inside = app
                    .get_webview_window("overlay-control")
                    .and_then(|window| window.ns_window().ok())
                    .is_some_and(|pointer| unsafe {
                        event
                            .as_ref()
                            .window(
                                objc2::MainThreadMarker::new()
                                    .expect("AppKit event callback is on the main thread"),
                            )
                            .is_some_and(|window| {
                                Retained::as_ptr(&window).cast::<std::ffi::c_void>()
                                    == pointer.cast_const()
                            })
                    });
                if !inside {
                    OverlayControlWindowManager::dismiss_panel(&app);
                }
                // Observe the click without consuming it or activating Mimi.
                event.as_ptr()
            });
            let mut tokens = Vec::new();
            if let Some(token) =
                NSEvent::addGlobalMonitorForEventsMatchingMask_handler(mask, &global)
            {
                tokens.push(token);
            }
            if let Some(token) =
                unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &local) }
            {
                tokens.push(token);
            }
            *slot = Some(Monitors(tokens));
        });
    });
}

/// Use ordering, not the resignKeyWindow notification hook, to transfer key
/// status. The restored island stays nonactivating and is never made key.
fn release_passive_keyboard(
    mode: OverlayControlMode,
    is_key: bool,
    order_out: impl FnOnce(),
    restore_island: impl FnOnce(),
) {
    if !is_key || mode == OverlayControlMode::Panel {
        return;
    }
    order_out();
    if mode == OverlayControlMode::Island {
        restore_island();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_key_island_is_restored_after_ordering_out() {
        for (mode, is_key, expected) in [
            (OverlayControlMode::Island, true, vec!["out", "restore"]),
            (OverlayControlMode::Hidden, true, vec!["out"]),
            (OverlayControlMode::Panel, true, vec![]),
            (OverlayControlMode::Island, false, vec![]),
            (OverlayControlMode::Hidden, false, vec![]),
            (OverlayControlMode::Panel, false, vec![]),
        ] {
            let calls = RefCell::new(Vec::new());
            release_passive_keyboard(
                mode,
                is_key,
                || calls.borrow_mut().push("out"),
                || calls.borrow_mut().push("restore"),
            );
            assert_eq!(calls.into_inner(), expected, "mode={mode:?} key={is_key}");
        }
    }
}
