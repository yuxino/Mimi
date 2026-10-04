//! Desktop-owned shortcuts for Linux sessions that cannot use X11 key grabs.

#[cfg(any(target_os = "linux", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopAction {
    ToggleSession,
    ToggleImmersive,
    CycleSubtitleDisplay,
}

#[cfg(any(target_os = "linux", test))]
impl DesktopAction {
    pub fn from_args(args: &[String]) -> Option<Self> {
        // Only one explicit action is accepted. Never interpret an action
        // hidden among unrelated arguments or execute an arbitrary command.
        if args.len() != 2 {
            return None;
        }
        match args[1].as_str() {
            "--toggle-session" => Some(Self::ToggleSession),
            "--toggle-immersive" => Some(Self::ToggleImmersive),
            "--cycle-subtitle-display" => Some(Self::CycleSubtitleDisplay),
            _ => None,
        }
    }
}

#[cfg(any(target_os = "linux", test))]
fn is_wayland(session_type: Option<&str>, display: Option<&str>) -> bool {
    session_type.is_some_and(|value| value.eq_ignore_ascii_case("wayland"))
        || display.is_some_and(|value| !value.is_empty())
}

pub fn uses_system_shortcuts() -> bool {
    #[cfg(target_os = "linux")]
    {
        is_wayland(
            std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
            std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
        )
    }
    #[cfg(not(target_os = "linux"))]
    false
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopShortcutCommands {
    toggle_session: String,
    toggle_immersive: String,
    cycle_subtitle_display: String,
}

#[cfg(any(target_os = "linux", test))]
fn quote_executable(path: &str) -> String {
    // GNOME custom commands are parsed with shell-style argument quoting.
    format!("'{}'", path.replace('\'', "'\\''"))
}

/// Null means native shortcuts; Wayland gets commands for this installation,
/// including an AppImage's original path rather than its temporary mount.
#[tauri::command]
pub fn app_desktop_shortcut_commands() -> Result<Option<DesktopShortcutCommands>, String> {
    #[cfg(target_os = "linux")]
    if uses_system_shortcuts() {
        let executable = std::env::var_os("APPIMAGE")
            .filter(|value| !value.is_empty())
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::current_exe().ok())
            .ok_or_else(|| "shortcut.executable_unavailable".to_string())?;
        let executable = executable
            .to_str()
            .ok_or_else(|| "shortcut.executable_unavailable".to_string())?;
        let command = quote_executable(executable);
        return Ok(Some(DesktopShortcutCommands {
            toggle_session: format!("{command} --toggle-session"),
            toggle_immersive: format!("{command} --toggle-immersive"),
            cycle_subtitle_display: format!("{command} --cycle-subtitle-display"),
        }));
    }
    Ok(None)
}

#[cfg(target_os = "linux")]
pub fn dispatch_linux_launch(app: &tauri::AppHandle, args: &[String]) {
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use tauri::Manager;

    let action = DesktopAction::from_args(args);
    let handle = app.clone();
    // The D-Bus listener is registered before app setup. Queue on the main
    // thread so simultaneous cold launches cannot access uninitialized state
    // or create GTK windows on the D-Bus worker thread.
    if app
        .run_on_main_thread(move || {
            let Some(action) = action else {
                crate::windows::ensure_settings_window(&handle);
                return;
            };
            static LAST_ACTION: Mutex<[Option<Instant>; 3]> = Mutex::new([None; 3]);
            let index = action as usize;
            {
                let mut last = LAST_ACTION.lock().unwrap();
                let now = Instant::now();
                if last[index]
                    .is_some_and(|time| now.duration_since(time) < Duration::from_millis(500))
                {
                    return;
                }
                last[index] = Some(now);
            }
            let Some(state) = handle.try_state::<crate::commands::AppState>() else {
                tracing::warn!("desktop shortcut unavailable before app setup");
                return;
            };
            let session = Arc::clone(&state.session);
            tauri::async_runtime::spawn(async move {
                match action {
                    DesktopAction::ToggleSession => {
                        if matches!(session.status_kind().as_str(), "connecting" | "stopping") {
                            return;
                        }
                        if session.is_active() {
                            session.stop().await;
                        } else if session.start().await.is_err() {
                            // SessionManager publishes the normal localized
                            // error state; keep logs free of provider content.
                            tracing::warn!("desktop session shortcut failed");
                        }
                    }
                    DesktopAction::ToggleImmersive => {
                        if crate::commands::toggle_immersive_mode(&handle)
                            .await
                            .is_err()
                        {
                            tracing::warn!("desktop immersive shortcut failed");
                        }
                    }
                    DesktopAction::CycleSubtitleDisplay => {
                        if crate::commands::set_subtitle_display_mode(&handle, None)
                            .await
                            .is_err()
                        {
                            tracing::warn!("desktop subtitle display shortcut failed");
                        }
                    }
                }
            });
        })
        .is_err()
    {
        tracing::warn!("desktop shortcut dispatch failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_actions_reject_extra_arguments() {
        for (flag, expected) in [
            ("--toggle-session", DesktopAction::ToggleSession),
            ("--toggle-immersive", DesktopAction::ToggleImmersive),
            (
                "--cycle-subtitle-display",
                DesktopAction::CycleSubtitleDisplay,
            ),
        ] {
            let args = vec!["/opt/Mimi AppImage".into(), flag.into()];
            assert_eq!(DesktopAction::from_args(&args), Some(expected));
            let mut extra = args;
            extra.push("--unknown".into());
            assert_eq!(DesktopAction::from_args(&extra), None);
        }
        assert_eq!(DesktopAction::from_args(&[]), None);
        assert_eq!(DesktopAction::from_args(&["mimi".into()]), None);
        assert_eq!(
            DesktopAction::from_args(&["mimi".into(), "--unknown".into()]),
            None
        );
    }

    #[test]
    fn wayland_session_does_not_become_x11_because_xwayland_exists() {
        assert!(is_wayland(Some("Wayland"), None));
        assert!(is_wayland(None, Some("wayland-0")));
        assert!(is_wayland(Some("x11"), Some("wayland-1")));
        assert!(!is_wayland(Some("x11"), Some("")));
        assert!(!is_wayland(None, None));
    }

    #[test]
    fn command_paths_preserve_spaces_quotes_and_shell_metacharacters() {
        assert_eq!(quote_executable("/usr/bin/mimi"), "'/usr/bin/mimi'");
        assert_eq!(
            quote_executable("/home/user/Mimi's $(touch nope).AppImage"),
            "'/home/user/Mimi'\\''s $(touch nope).AppImage'"
        );
    }
}
