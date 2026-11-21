//! Windows transfers focus to the subtitle root during a native drag that
//! began in its owned panel. Keep that internal handoff out of click-away
//! dismissal, then return keyboard ownership only if Mimi still owns focus.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Foreground {
    Root,
    Control,
    External,
    None,
}

fn drag_finished(saw_move: bool, moving: bool, left_down: bool, foreground: Foreground) -> bool {
    foreground == Foreground::External || (saw_move && !moving) || (!moving && !left_down)
}

fn forward_released_button(
    moving: bool,
    captured: bool,
    released: bool,
    foreground: Foreground,
) -> bool {
    moving && captured && released && foreground == Foreground::Root
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FocusAfterDrag {
    RestoreControl,
    KeepControl,
    Dismiss,
}

fn focus_after_drag(foreground: Foreground) -> FocusAfterDrag {
    match foreground {
        Foreground::Root => FocusAfterDrag::RestoreControl,
        Foreground::Control => FocusAfterDrag::KeepControl,
        Foreground::External | Foreground::None => FocusAfterDrag::Dismiss,
    }
}

#[cfg(target_os = "windows")]
mod native {
    use super::{
        drag_finished, focus_after_drag, forward_released_button, FocusAfterDrag, Foreground,
    };
    use crate::windows::windows_panel_drag_release::{ReleasePointLease, ScreenPoint};
    use crate::windows::{OverlayControlState, OverlayControlWindowManager};
    use ::windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use ::windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
    use ::windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, IsWindow, PostMessageW,
        SetForegroundWindow, GUITHREADINFO, GUI_INMOVESIZE, WM_CANCELMODE, WM_LBUTTONUP,
    };
    use std::time::{Duration, Instant};
    use tauri::{AppHandle, Manager, WebviewWindow};

    pub(crate) struct PanelDrag {
        token: u64,
        root: isize,
        control: isize,
        root_thread: u32,
        release: ReleasePointLease,
    }

    impl PanelDrag {
        fn foreground(&self) -> Foreground {
            let window = unsafe { GetForegroundWindow() }.0 as isize;
            if window == self.root {
                Foreground::Root
            } else if window == self.control {
                Foreground::Control
            } else if window == 0 {
                Foreground::None
            } else {
                Foreground::External
            }
        }

        fn finish(self, app: &AppHandle) {
            self.release.stop();
            let app = app.clone();
            let current_app = app.clone();
            let _ = app.run_on_main_thread(move || {
                let Some(state) = current_app.try_state::<OverlayControlState>() else {
                    return;
                };
                if !state.finish_panel_drag(self.token) {
                    return;
                }
                let Some(root) = current_app.get_webview_window("overlay") else {
                    return;
                };
                let Some(control) = current_app.get_webview_window("overlay-control") else {
                    return;
                };
                if root.hwnd().ok().map(|handle| handle.0 as isize) != Some(self.root)
                    || root.is_visible().ok() != Some(true)
                    || control.hwnd().ok().map(|handle| handle.0 as isize) != Some(self.control)
                    || control.is_visible().ok() != Some(true)
                {
                    return;
                }
                match focus_after_drag(self.foreground()) {
                    FocusAfterDrag::RestoreControl => {
                        // Recovery must respect an external activation, rather
                        // than Tao's explicit set_focus retry with synthetic Alt.
                        if !unsafe { SetForegroundWindow(HWND(self.control as *mut _)) }.as_bool()
                            && self.foreground() != Foreground::Control
                        {
                            OverlayControlWindowManager::dismiss_panel(&current_app);
                        }
                    }
                    FocusAfterDrag::KeepControl => {}
                    FocusAfterDrag::Dismiss => {
                        OverlayControlWindowManager::dismiss_panel(&current_app)
                    }
                }
            });
        }

        pub(crate) fn cancel(self, app: &AppHandle) {
            self.finish(app);
        }

        async fn post_end(&self, app: &AppHandle, release: Option<ScreenPoint>) -> bool {
            let (sender, receiver) = tokio::sync::oneshot::channel();
            let current_app = app.clone();
            let token = self.token;
            let root = self.root;
            let thread = self.root_thread;
            if app
                .run_on_main_thread(move || {
                    let same_root = current_app
                        .get_webview_window("overlay")
                        .and_then(|window| window.hwnd().ok())
                        .is_some_and(|window| window.0 as isize == root);
                    let posted = same_root
                        && current_app
                            .try_state::<OverlayControlState>()
                            .is_some_and(|state| {
                                state.with_current_panel_drag(token, || {
                                    let window = HWND(root as *mut _);
                                    if unsafe { GetForegroundWindow() } != window {
                                        return false;
                                    }
                                    let mut gui = GUITHREADINFO {
                                        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
                                        ..Default::default()
                                    };
                                    if unsafe { GetGUIThreadInfo(thread, &mut gui) }.is_err()
                                        || gui.flags.0 & GUI_INMOVESIZE.0 == 0
                                        || gui.hwndMoveSize != window
                                    {
                                        return false;
                                    }
                                    let (message, position) = if let Some(point) = release {
                                        if gui.hwndCapture != window {
                                            return false;
                                        }
                                        // This active move loop consumes screen coordinates.
                                        // Use the actual first up, never the later cursor or
                                        // client-relative coordinates (which committed 0,0).
                                        let position =
                                            ((point.y as u16 as u32) << 16) | point.x as u16 as u32;
                                        (WM_LBUTTONUP, LPARAM(position as isize))
                                    } else {
                                        (WM_CANCELMODE, LPARAM(0))
                                    };
                                    unsafe {
                                        PostMessageW(Some(window), message, WPARAM(0), position)
                                    }
                                    .is_ok()
                                })
                            });
                    let _ = sender.send(posted);
                })
                .is_err()
            {
                return false;
            }
            receiver.await.unwrap_or(false)
        }

        pub(crate) fn observe(self, app: &AppHandle) {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let mut saw_move = false;
                let mut release_forwarded = false;
                let mut startup_expired = false;
                let mut native_capture_ready = false;
                let started = Instant::now();
                loop {
                    if !app
                        .try_state::<OverlayControlState>()
                        .is_some_and(|state| state.has_panel_drag(self.token))
                    {
                        return;
                    }
                    // Pointer-bearing native structs do not cross the await.
                    let sample = {
                        let root = HWND(self.root as *mut _);
                        let control = HWND(self.control as *mut _);
                        if unsafe {
                            IsWindow(Some(root)).as_bool() && IsWindow(Some(control)).as_bool()
                        } {
                            let mut gui = GUITHREADINFO {
                                cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
                                ..Default::default()
                            };
                            let moving = unsafe { GetGUIThreadInfo(self.root_thread, &mut gui) }
                                .is_ok()
                                && gui.flags.0 & GUI_INMOVESIZE.0 != 0
                                && gui.hwndMoveSize == root;
                            let captured = gui.hwndCapture == root;
                            let left_down = unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) } < 0;
                            Some((moving, captured, left_down, self.foreground()))
                        } else {
                            None
                        }
                    };
                    let Some((moving, captured, left_down, foreground)) = sample else {
                        break;
                    };
                    if drag_finished(saw_move, moving, left_down, foreground)
                        || (startup_expired && !moving)
                    {
                        break;
                    }
                    // Once the root owns capture while held, Windows receives
                    // its normal up. No hook is needed for a long/native drag.
                    if moving && captured && left_down && self.release.tracking() {
                        self.release.stop();
                        native_capture_ready = true;
                    }
                    let release = self.release.point();
                    if !release_forwarded
                        && forward_released_button(moving, captured, release.is_some(), foreground)
                    {
                        release_forwarded = self.post_end(&app, release).await;
                    }
                    // Bound only startup without native capture. Long drags
                    // already disarmed above keep normal Windows behavior.
                    if !native_capture_ready
                        && !startup_expired
                        && started.elapsed() >= Duration::from_secs(30)
                    {
                        self.release.stop();
                        startup_expired = true;
                        let _ = self.post_end(&app, None).await;
                    }
                    saw_move |= moving;
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                self.finish(&app);
            });
        }
    }

    pub(crate) fn panel_drag_button_down() -> bool {
        (unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) }) < 0
    }

    pub(crate) fn prepare_panel_drag(
        app: &AppHandle,
        root: &WebviewWindow,
    ) -> Result<Option<PanelDrag>, ()> {
        let Some(control) = app.get_webview_window("overlay-control") else {
            return Ok(None);
        };
        let root = root.hwnd().map_err(|_| ())?.0 as isize;
        let control = control.hwnd().map_err(|_| ())?.0 as isize;
        if unsafe { GetForegroundWindow() }.0 as isize != control || !panel_drag_button_down() {
            return Ok(None);
        }
        let root_thread = unsafe { GetWindowThreadProcessId(HWND(root as *mut _), None) };
        if root_thread == 0 {
            return Err(());
        }
        let Some(state) = app.try_state::<OverlayControlState>() else {
            return Ok(None);
        };
        let Some(token) = state.begin_panel_drag() else {
            return Ok(None);
        };
        let release = match ReleasePointLease::install(
            app,
            token,
            HWND(root as *mut _),
            HWND(control as *mut _),
        ) {
            Ok(Some(release)) => release,
            result => {
                state.finish_panel_drag(token);
                return result.map(|_| None);
            }
        };
        Ok(Some(PanelDrag {
            token,
            root,
            control,
            root_thread,
            release,
        }))
    }
}

#[cfg(target_os = "windows")]
pub(crate) use native::{panel_drag_button_down, prepare_panel_drag};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_move_and_stationary_press_keep_the_internal_handoff_owned() {
        for foreground in [Foreground::Root, Foreground::Control, Foreground::None] {
            assert!(!drag_finished(false, true, true, foreground));
            assert!(!drag_finished(true, true, true, foreground));
            assert!(!drag_finished(false, false, true, foreground));
        }
    }

    #[test]
    fn release_short_click_and_cancel_finish_without_requiring_a_moved_event() {
        assert!(drag_finished(false, false, false, Foreground::Root));
        assert!(drag_finished(false, false, false, Foreground::Control));
        assert!(drag_finished(true, false, true, Foreground::Root));
    }

    #[test]
    fn actual_external_focus_ends_a_drag_without_waiting_for_mouse_release() {
        assert!(drag_finished(false, false, true, Foreground::External));
        assert!(drag_finished(true, true, true, Foreground::External));
        assert_eq!(
            focus_after_drag(Foreground::External),
            FocusAfterDrag::Dismiss
        );
        assert_eq!(focus_after_drag(Foreground::None), FocusAfterDrag::Dismiss);
    }

    #[test]
    fn only_an_owned_native_move_can_receive_the_already_released_button() {
        assert!(forward_released_button(true, true, true, Foreground::Root));
        assert!(!forward_released_button(
            true,
            true,
            false,
            Foreground::Root
        ));
        assert!(!forward_released_button(
            false,
            true,
            true,
            Foreground::Root
        ));
        assert!(!forward_released_button(
            true,
            false,
            true,
            Foreground::Root
        ));
        for foreground in [Foreground::Control, Foreground::External, Foreground::None] {
            assert!(!forward_released_button(true, true, true, foreground));
        }
    }

    #[test]
    fn only_the_owning_root_handoff_restores_control_focus() {
        assert_eq!(
            focus_after_drag(Foreground::Root),
            FocusAfterDrag::RestoreControl
        );
        assert_eq!(
            focus_after_drag(Foreground::Control),
            FocusAfterDrag::KeepControl
        );
    }
}
