//! Presence recovery for Windows' nonactivating subtitle WebViews.
//!
//! WebView2 can miss entry after returning to the same physical point even
//! when Windows still hits the same child HWND. While either Mimi surface is
//! visible, sample native ownership at 80 ms and publish only presence changes.
//! The worker parks without querying Windows when both surfaces are hidden.

#[derive(Clone, Copy, Default)]
struct SurfaceTracking {
    handle: Option<isize>,
    published: Option<bool>,
}

impl SurfaceTracking {
    fn set_handle(&mut self, handle: Option<isize>) -> bool {
        if self.handle == handle {
            return false;
        }
        self.handle = handle;
        self.published = None;
        true
    }

    fn needs_publication(&self, inside: bool) -> bool {
        self.published != Some(inside)
    }
}

fn owns_pointer(visible: bool, click_through: bool, window: isize, hit_root: isize) -> bool {
    visible && !click_through && window != 0 && window == hit_root
}

#[derive(Default)]
struct TrackingState {
    surfaces: [SurfaceTracking; 2],
    generation: u64,
    stopped: bool,
}

impl TrackingState {
    fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        for surface in &mut self.surfaces {
            surface.published = None;
        }
    }
}

#[cfg(target_os = "windows")]
mod native {
    use super::{owns_pointer, TrackingState};
    use ::windows::Win32::Foundation::{HWND, POINT};
    use ::windows::Win32::UI::WindowsAndMessaging::{
        GetAncestor, GetCursorPos, GetWindowLongPtrW, IsWindowVisible, WindowFromPoint, GA_ROOT,
        GWL_EXSTYLE, WS_EX_TRANSPARENT,
    };
    use serde::{Deserialize, Serialize};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::Duration;
    use tauri::{AppHandle, Emitter, EventTarget, Listener, Manager, WebviewWindow};

    const EVENT: &str = "overlay-chrome-hover";
    const LABELS: [&str; 2] = ["overlay", "overlay-control"];
    const INTERVAL: Duration = Duration::from_millis(80);

    #[derive(Clone, Serialize)]
    struct Presence {
        kind: &'static str,
        visible: bool,
    }

    #[derive(Deserialize)]
    struct Ready<'a> {
        kind: &'a str,
        #[serde(rename = "visible")]
        _visible: bool,
    }

    pub struct WindowsPointerTracker {
        shared: Arc<(Mutex<TrackingState>, Condvar)>,
        thread: Mutex<Option<std::thread::JoinHandle<()>>>,
        ready_listener: tauri::EventId,
    }

    impl WindowsPointerTracker {
        fn stop(&self) {
            self.shared.0.lock().unwrap().stopped = true;
            self.shared.1.notify_all();
            if let Some(thread) = self.thread.lock().unwrap().take() {
                let _ = thread.join();
            }
        }
    }

    impl Drop for WindowsPointerTracker {
        fn drop(&mut self) {
            self.stop();
        }
    }

    pub(crate) fn install(app: &AppHandle) -> WindowsPointerTracker {
        let shared = Arc::new((Mutex::new(TrackingState::default()), Condvar::new()));
        let ready_state = Arc::clone(&shared);
        // The existing renderer handshake follows listener installation. A
        // reload or a late listener must receive the stationary native owner.
        let ready_listener = app.listen_any(EVENT, move |event| {
            if event.payload().len() > 128 {
                return;
            }
            let Ok(ready) = serde_json::from_str::<Ready<'_>>(event.payload()) else {
                return;
            };
            if ready.kind != "ready" {
                return;
            }
            let mut state = ready_state.0.lock().unwrap();
            state.invalidate();
            ready_state.1.notify_all();
        });
        let app_for_thread = app.clone();
        let shared_for_thread = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("mimi-overlay-hover".into())
            .spawn(move || sample_visible_surfaces(app_for_thread, shared_for_thread))
            .map_err(|_| {
                tracing::warn!("overlay hover unavailable label=thread_start_failed");
            })
            .ok();
        WindowsPointerTracker {
            shared,
            thread: Mutex::new(thread),
            ready_listener,
        }
    }

    fn rearm_failed_publication(
        shared: &(Mutex<TrackingState>, Condvar),
        generation: u64,
        index: usize,
        inside: bool,
    ) {
        let mut state = shared.0.lock().unwrap();
        if state.generation == generation && state.surfaces[index].published == Some(inside) {
            state.surfaces[index].published = None;
        }
    }

    fn publish(
        app: &AppHandle,
        shared: &Arc<(Mutex<TrackingState>, Condvar)>,
        generation: u64,
        index: usize,
        inside: bool,
    ) {
        let app_for_main = app.clone();
        let state_for_main = Arc::clone(shared);
        // Serialize the final lifetime check with native show/hide. Never
        // hold tracking state across emit_to: Rust event callbacks can reenter
        // the ready handshake synchronously.
        if app
            .run_on_main_thread(move || {
                let current = {
                    let state = state_for_main.0.lock().unwrap();
                    !state.stopped
                        && state.generation == generation
                        && state.surfaces[index].published == Some(inside)
                };
                if current
                    && app_for_main
                        .emit_to(
                            EventTarget::webview_window(LABELS[index]),
                            EVENT,
                            Presence {
                                kind: "native-pointer",
                                visible: inside,
                            },
                        )
                        .is_err()
                {
                    rearm_failed_publication(&state_for_main, generation, index, inside);
                }
            })
            .is_err()
        {
            rearm_failed_publication(shared, generation, index, inside);
        }
    }

    fn sample_visible_surfaces(app: AppHandle, shared: Arc<(Mutex<TrackingState>, Condvar)>) {
        loop {
            let mut state = shared.0.lock().unwrap();
            while !state.stopped
                && state
                    .surfaces
                    .iter()
                    .all(|surface| surface.handle.is_none() && surface.published == Some(false))
            {
                state = shared.1.wait(state).unwrap();
            }
            if state.stopped {
                break;
            }
            let handles = state.surfaces.map(|surface| surface.handle);
            let generation = state.generation;
            drop(state);

            let mut point = POINT::default();
            let hit_root = if handles.iter().any(Option::is_some) {
                unsafe {
                    GetCursorPos(&mut point)
                        .ok()
                        .map(|()| GetAncestor(WindowFromPoint(point), GA_ROOT).0 as isize)
                        .unwrap_or(0)
                }
            } else {
                0
            };
            let inside = handles.map(|handle| {
                handle.is_some_and(|handle| unsafe {
                    let window = HWND(handle as *mut _);
                    owns_pointer(
                        IsWindowVisible(window).as_bool(),
                        GetWindowLongPtrW(window, GWL_EXSTYLE) as u32 & WS_EX_TRANSPARENT.0 != 0,
                        handle,
                        hit_root,
                    )
                })
            });

            let mut state = shared.0.lock().unwrap();
            if state.stopped {
                break;
            }
            // Hide/show and HWND replacement can happen during the native
            // reads. Never publish a sample from an earlier window lifetime.
            if state.generation != generation {
                continue;
            }
            let mut changes = [None; 2];
            for (index, inside) in inside.into_iter().enumerate() {
                if state.surfaces[index].needs_publication(inside) {
                    state.surfaces[index].published = Some(inside);
                    changes[index] = Some(inside);
                }
            }
            drop(state);
            for (index, inside) in changes.into_iter().enumerate() {
                if let Some(inside) = inside {
                    publish(&app, &shared, generation, index, inside);
                }
            }
            let state = shared.0.lock().unwrap();
            if !state.stopped {
                drop(shared.1.wait_timeout(state, INTERVAL).unwrap());
            }
        }
    }

    pub(crate) fn activate(window: &WebviewWindow) {
        let Some(index) = LABELS.iter().position(|label| *label == window.label()) else {
            return;
        };
        if !window.is_visible().unwrap_or(false) {
            return;
        }
        let Ok(handle) = window.hwnd() else {
            return;
        };
        let Some(tracker) = window.app_handle().try_state::<WindowsPointerTracker>() else {
            return;
        };
        let mut state = tracker.shared.0.lock().unwrap();
        if state.surfaces[index].set_handle(Some(handle.0 as isize)) {
            state.invalidate();
            tracker.shared.1.notify_all();
        }
    }

    pub(crate) fn remove(app: &AppHandle, label: &str) {
        let Some(index) = LABELS.iter().position(|surface| *surface == label) else {
            return;
        };
        let Some(tracker) = app.try_state::<WindowsPointerTracker>() else {
            return;
        };
        let mut state = tracker.shared.0.lock().unwrap();
        if state.surfaces[index].set_handle(None) {
            state.invalidate();
            tracker.shared.1.notify_all();
        }
    }

    pub(crate) fn remove_all(app: &AppHandle) {
        if let Some(tracker) = app.try_state::<WindowsPointerTracker>() {
            app.unlisten(tracker.ready_listener);
            tracker.stop();
        }
    }
}

#[cfg(target_os = "windows")]
pub use native::WindowsPointerTracker;
#[cfg(target_os = "windows")]
pub(super) use native::{activate, install, remove, remove_all};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_ownership_rejects_other_windows_hidden_and_click_through_surfaces() {
        assert!(owns_pointer(true, false, 12, 12));
        assert!(!owns_pointer(true, false, 12, 13));
        assert!(!owns_pointer(false, false, 12, 12));
        assert!(!owns_pointer(true, true, 12, 12));
        assert!(!owns_pointer(true, false, 0, 0));
    }

    #[test]
    fn same_point_reentry_changes_presence_without_repeating_stationary_samples() {
        let mut surface = SurfaceTracking::default();
        assert!(surface.set_handle(Some(12)));
        assert!(surface.needs_publication(true));
        surface.published = Some(true);
        assert!(!surface.needs_publication(true));
        assert!(!surface.set_handle(Some(12)));
        assert!(!surface.needs_publication(true));
        assert!(surface.needs_publication(false));
        surface.published = Some(false);
        assert!(surface.needs_publication(true));
        surface.published = Some(true);
        assert!(surface.set_handle(None));
        assert!(surface.set_handle(Some(12)));
        assert!(surface.needs_publication(true));
        surface.published = Some(true);
        // A renderer's ready handshake replays its current stationary owner.
        surface.published = None;
        assert!(surface.needs_publication(true));
        assert!(surface.set_handle(Some(13)));
        assert!(surface.needs_publication(false));
    }

    #[test]
    fn ready_and_hidden_window_replacement_invalidate_queued_ownership() {
        let mut state = TrackingState::default();
        assert!(!state.stopped);
        state.surfaces[0].set_handle(Some(12));
        state.surfaces[0].published = Some(true);
        state.surfaces[1].published = Some(false);
        let sampled_generation = state.generation;
        state.surfaces[0].set_handle(None);
        state.invalidate();
        state.surfaces[0].set_handle(Some(12));
        state.invalidate();
        // Even HWND reuse cannot validate a sample from before hide/show.
        assert_ne!(state.generation, sampled_generation);
        assert!(state.surfaces[0].needs_publication(true));
        assert!(state.surfaces[1].needs_publication(false));
        state.surfaces[0].published = Some(true);
        let shown_generation = state.generation;
        // A late renderer's ready handshake forces one current-state replay.
        state.invalidate();
        assert_ne!(state.generation, shown_generation);
        assert!(state.surfaces[0].needs_publication(true));
    }
}
