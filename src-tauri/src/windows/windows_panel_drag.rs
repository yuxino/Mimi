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
    left_down: bool,
    foreground: Foreground,
) -> bool {
    moving && captured && !left_down && foreground == Foreground::Root
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
    use crate::windows::{OverlayControlState, OverlayControlWindowManager};
    use ::windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
    use ::windows::Win32::Graphics::Gdi::ScreenToClient;
    use ::windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
    use ::windows::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, IsWindow,
        PostMessageW, SetForegroundWindow, GUITHREADINFO, GUI_INMOVESIZE, WM_LBUTTONUP,
    };
    use std::time::Duration;
    use tauri::{AppHandle, Manager, WebviewWindow};

    pub(crate) struct PanelDrag {
        token: u64,
        root: isize,
        control: isize,
        root_thread: u32,
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
            let app = app.clone();
            let current_app = app.clone();
            // Serialize the current foreground check with focus restoration;
            // an external application or Settings must never be reactivated over.
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
                        // Tao's explicit set_focus retries with synthetic Alt
                        // input. Recovery must respect a concurrent external
                        // activation instead of taking that force-focus path.
                        if !unsafe { SetForegroundWindow(HWND(self.control as *mut _)) }.as_bool()
                            && self.foreground() != Foreground::Control
                        {
                            OverlayControlWindowManager::dismiss_panel(&current_app);
                        }
                    }
                    FocusAfterDrag::KeepControl => {}
                    FocusAfterDrag::Dismiss => {
                        OverlayControlWindowManager::dismiss_panel(&current_app);
                    }
                }
            });
        }

        pub(crate) fn cancel(self, app: &AppHandle) {
            self.finish(app);
        }

        pub(crate) fn observe(self, app: &AppHandle) {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let mut saw_move = false;
                let mut release_forwarded = false;
                loop {
                    if !app
                        .try_state::<OverlayControlState>()
                        .is_some_and(|state| state.has_panel_drag(self.token))
                    {
                        return;
                    }
                    // Keep native pointer-bearing structures out of the
                    // async task state across the timer await.
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
                            let left_down = unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) } < 0;
                            let foreground = self.foreground();
                            if !release_forwarded
                                && forward_released_button(
                                    moving,
                                    gui.hwndCapture == root,
                                    left_down,
                                    foreground,
                                )
                            {
                                // A short cross-WebView press can release before
                                // the root acquires capture. Forward that already
                                // released button once to finish the owned move;
                                // cancelling would discard outline-mode movement.
                                let mut point = POINT::default();
                                if unsafe { GetCursorPos(&mut point) }.is_ok()
                                    && unsafe { ScreenToClient(root, &mut point) }.as_bool()
                                {
                                    let position =
                                        ((point.y as u16 as u32) << 16) | point.x as u16 as u32;
                                    release_forwarded = unsafe {
                                        PostMessageW(
                                            Some(root),
                                            WM_LBUTTONUP,
                                            WPARAM(0),
                                            LPARAM(position as isize),
                                        )
                                    }
                                    .is_ok();
                                }
                            }
                            Some((moving, left_down, foreground))
                        } else {
                            None
                        }
                    };
                    let Some((moving, left_down, foreground)) = sample else {
                        break;
                    };
                    if drag_finished(saw_move, moving, left_down, foreground) {
                        break;
                    }
                    saw_move |= moving;
                    // This task exists only for one panel-authored gesture.
                    // Outline drags may produce no Moved event until release;
                    // a click or a cancelled/short drag may produce none at all.
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                self.finish(&app);
            });
        }
    }

    pub(crate) fn prepare_panel_drag(app: &AppHandle, root: &WebviewWindow) -> Option<PanelDrag> {
        let control = app.get_webview_window("overlay-control")?;
        let root = root.hwnd().ok()?.0 as isize;
        let control = control.hwnd().ok()?.0 as isize;
        // Only a gesture from the currently focused panel owns the temporary
        // internal handoff. Ordinary root/click-away focus policy is unchanged.
        if unsafe { GetForegroundWindow() }.0 as isize != control {
            return None;
        }
        let root_thread = unsafe { GetWindowThreadProcessId(HWND(root as *mut _), None) };
        if root_thread == 0 {
            return None;
        }
        let token = app.try_state::<OverlayControlState>()?.begin_panel_drag()?;
        Some(PanelDrag {
            token,
            root,
            control,
            root_thread,
        })
    }
}

#[cfg(target_os = "windows")]
pub(crate) use native::prepare_panel_drag;

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
        assert!(forward_released_button(true, true, false, Foreground::Root));
        assert!(!forward_released_button(true, true, true, Foreground::Root));
        assert!(!forward_released_button(
            false,
            true,
            false,
            Foreground::Root
        ));
        assert!(!forward_released_button(
            true,
            false,
            false,
            Foreground::Root
        ));
        for foreground in [Foreground::Control, Foreground::External, Foreground::None] {
            assert!(!forward_released_button(true, true, false, foreground));
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
