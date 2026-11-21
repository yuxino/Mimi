//! One release point for a panel-authored Windows gesture. WebView2's input
//! leaf is in another process, so the app's own window queue can miss its up.
//! The gesture owner disarms at native capture, cancellation or startup timeout;
//! a first up removes the registration before reading its owned screen point.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ScreenPoint {
    pub x: i32,
    pub y: i32,
}

#[derive(Default)]
struct FirstRelease {
    stopped: bool,
    point: Option<ScreenPoint>,
}

impl FirstRelease {
    fn capture(&mut self, point: ScreenPoint) {
        if !self.stopped {
            self.point = Some(point);
            self.stopped = true;
        }
    }

    fn cancel(&mut self) {
        self.stopped = true;
        self.point = None;
    }
}

#[cfg(target_os = "windows")]
mod native {
    use super::{FirstRelease, ScreenPoint};
    use crate::windows::OverlayControlState;
    use ::windows::core::PCWSTR;
    use ::windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use ::windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use ::windows::Win32::System::Threading::GetCurrentThreadId;
    use ::windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
    use ::windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetForegroundWindow, GetWindowThreadProcessId, SetWindowsHookExW,
        UnhookWindowsHookEx, HHOOK, MSLLHOOKSTRUCT, WH_MOUSE_LL, WM_LBUTTONUP,
    };
    use std::cell::RefCell;
    use std::sync::{Arc, Mutex};
    use tauri::{AppHandle, Manager};

    struct Registration {
        token: u64,
        hook: HHOOK,
        root: HWND,
        control: HWND,
        app: AppHandle,
        point: Arc<Mutex<FirstRelease>>,
    }

    impl Drop for Registration {
        fn drop(&mut self) {
            let _ = unsafe { UnhookWindowsHookEx(self.hook) };
        }
    }

    thread_local! {
        static ACTIVE: RefCell<Option<Registration>> = const { RefCell::new(None) };
    }

    fn clear_registration(token: Option<u64>) {
        let old = ACTIVE.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot
                .as_ref()
                .is_some_and(|current| token.is_none_or(|id| current.token == id))
            {
                slot.take()
            } else {
                None
            }
        });
        if let Some(old) = old {
            old.point.lock().unwrap().cancel();
            drop(old);
        }
    }

    fn capture_release(data: LPARAM) {
        // Remove before touching Win32 or application state: callbacks and
        // cleanup never borrow the registry across a native call.
        let Some(current) = ACTIVE.with(|slot| slot.borrow_mut().take()) else {
            return;
        };
        let point = current.point.clone();
        let owned = current
            .app
            .try_state::<OverlayControlState>()
            .is_some_and(|state| state.has_panel_drag(current.token));
        let foreground = unsafe { GetForegroundWindow() };
        let owned = owned && (foreground == current.root || foreground == current.control);
        // Unregister at the first up, including an external/cancelled one.
        drop(current);
        let mut point = point.lock().unwrap();
        if !owned || point.stopped {
            point.cancel();
            return;
        }
        // SAFETY: WH_MOUSE_LL supplies a valid MSLLHOOKSTRUCT for this
        // callback. No other mouse message's payload is read or retained.
        let actual = unsafe { &*(data.0 as *const MSLLHOOKSTRUCT) };
        point.capture(ScreenPoint {
            x: actual.pt.x,
            y: actual.pt.y,
        });
    }

    unsafe extern "system" fn mouse_hook(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
        if code >= 0 && message.0 == WM_LBUTTONUP as usize {
            // A poisoned app state must not unwind through the native hook or
            // prevent the real input from continuing to its original target.
            let _ = std::panic::catch_unwind(|| capture_release(data));
        }
        unsafe { CallNextHookEx(None, code, message, data) }
    }

    pub(in crate::windows) struct ReleasePointLease {
        app: AppHandle,
        token: u64,
        point: Arc<Mutex<FirstRelease>>,
    }

    impl ReleasePointLease {
        pub(in crate::windows) fn install(
            app: &AppHandle,
            token: u64,
            root: HWND,
            control: HWND,
        ) -> Result<Option<Self>, ()> {
            // Synchronous overlay commands run on the native window thread,
            // which also pumps hooks during Windows' modal move loop.
            if unsafe { GetCurrentThreadId() } != unsafe { GetWindowThreadProcessId(root, None) } {
                return Err(());
            }
            clear_registration(None);
            if unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) } >= 0 {
                return Ok(None);
            }
            let point = Arc::new(Mutex::new(FirstRelease::default()));
            let module = unsafe { GetModuleHandleW(PCWSTR::null()) }.map_err(|_| ())?;
            let hook =
                unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), Some(module.into()), 0) }
                    .map_err(|_| ())?;
            ACTIVE.with(|slot| {
                *slot.borrow_mut() = Some(Registration {
                    token,
                    hook,
                    root,
                    control,
                    app: app.clone(),
                    point: point.clone(),
                });
            });
            if unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) } >= 0 {
                clear_registration(Some(token));
                return Ok(None);
            }
            Ok(Some(Self {
                app: app.clone(),
                token,
                point,
            }))
        }

        pub(in crate::windows) fn point(&self) -> Option<ScreenPoint> {
            self.point.lock().unwrap().point
        }

        pub(in crate::windows) fn tracking(&self) -> bool {
            !self.point.lock().unwrap().stopped
        }

        pub(in crate::windows) fn stop(&self) {
            self.point.lock().unwrap().cancel();
            let token = self.token;
            let _ = self
                .app
                .run_on_main_thread(move || clear_registration(Some(token)));
        }
    }

    impl Drop for ReleasePointLease {
        fn drop(&mut self) {
            self.stop();
        }
    }
}

#[cfg(target_os = "windows")]
pub(super) use native::ReleasePointLease;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_up_stays_fixed_when_the_pointer_moves_or_releases_again() {
        let mut release = FirstRelease::default();
        let first = ScreenPoint { x: 448, y: 211 };
        release.capture(first);
        release.capture(ScreenPoint { x: 478, y: 211 });
        assert_eq!(release.point, Some(first));
        assert!(release.stopped);
    }

    #[test]
    fn retiring_before_or_after_up_cannot_supply_a_stale_release() {
        let mut release = FirstRelease::default();
        release.cancel();
        release.capture(ScreenPoint { x: 478, y: 211 });
        assert_eq!(release.point, None);
        let mut release = FirstRelease::default();
        release.capture(ScreenPoint { x: 448, y: 211 });
        release.cancel();
        assert_eq!(release.point, None);
    }
}
