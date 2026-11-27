//! Global shortcut registration and Linux desktop-owned launch commands.

use crate::commands;
use crate::session_manager::SessionManager;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[cfg(any(target_os = "linux", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopAction {
    ToggleSession,
    ToggleImmersive,
    ToggleLock,
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
            "--toggle-lock" => Some(Self::ToggleLock),
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
    toggle_lock: String,
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
            toggle_lock: format!("{command} --toggle-lock"),
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
            static LAST_ACTION: Mutex<[Option<Instant>; 4]> = Mutex::new([None; 4]);
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
                        if crate::commands::settings::toggle_immersive_mode(&handle)
                            .await
                            .is_err()
                        {
                            tracing::warn!("desktop immersive shortcut failed");
                        }
                    }
                    DesktopAction::ToggleLock => {
                        if crate::commands::overlay::toggle_overlay_lock(&handle)
                            .await
                            .is_err()
                        {
                            tracing::warn!("desktop lock shortcut failed");
                        }
                    }
                    DesktopAction::CycleSubtitleDisplay => {
                        if crate::commands::settings::set_subtitle_display_mode(&handle, None)
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

/// Registers the global session, immersive, lock, and subtitle display shortcuts. Each action
const SHORTCUT_DEBOUNCE: Duration = Duration::from_millis(500);

struct ShortcutDebounce {
    last_trigger: Mutex<Option<Instant>>,
}

impl ShortcutDebounce {
    fn new() -> Self {
        Self {
            last_trigger: Mutex::new(None),
        }
    }

    fn allow_at(&self, now: Instant) -> bool {
        let mut last = self.last_trigger.lock().unwrap();
        if last.is_some_and(|previous| now.saturating_duration_since(previous) < SHORTCUT_DEBOUNCE)
        {
            return false;
        }
        *last = Some(now);
        true
    }
}

/// owns an independent 500ms debounce so presentation switching never blocks
/// a session lifecycle action (or vice versa).
pub(crate) fn setup_global_shortcuts(
    app: &tauri::AppHandle,
    session: Arc<SessionManager>,
) -> tauri::Result<()> {
    use tauri_plugin_global_shortcut::{
        Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState,
    };

    if uses_system_shortcuts() {
        tracing::info!("Wayland shortcuts are configured in desktop settings");
        return Ok(());
    }

    // macOS: Cmd+Shift (SUPER is the Command key); Windows: Ctrl+Shift.
    #[cfg(target_os = "macos")]
    let modifiers = Modifiers::SUPER | Modifiers::SHIFT;
    #[cfg(not(target_os = "macos"))]
    let modifiers = Modifiers::CONTROL | Modifiers::SHIFT;
    let session_shortcut = Shortcut::new(Some(modifiers), Code::Space);
    let immersive_shortcut = Shortcut::new(Some(modifiers), Code::KeyM);
    let lock_shortcut = Shortcut::new(Some(modifiers), Code::KeyL);
    let display_shortcut = Shortcut::new(Some(modifiers), Code::KeyB);

    let session_debounce = ShortcutDebounce::new();
    let session_for_handler = Arc::clone(&session);

    // Register the global shortcut. A failure must not abort startup: another
    // app (e.g. a second mimi instance) may already own the combination, in
    // which case the OS keeps delivering it to that app.
    let session_register =
        app.global_shortcut()
            .on_shortcut(session_shortcut, move |_app, _shortcut, event| {
                if event.state() != ShortcutState::Pressed {
                    return;
                }
                if !session_debounce.allow_at(Instant::now()) {
                    return;
                }
                let session = Arc::clone(&session_for_handler);
                tauri::async_runtime::spawn(async move {
                    let status = session.status_kind();
                    if status == "connecting" || status == "stopping" {
                        return;
                    }
                    if session.is_active() {
                        session.stop().await;
                    } else {
                        let _ = session.start().await;
                    }
                });
            });
    match session_register {
        Ok(()) => tracing::info!("global session shortcut registered"),
        Err(error) => tracing::warn!(
            "global session shortcut could not be registered: {error} \
             (another app may already own the start/stop combination)"
        ),
    }

    let immersive_debounce = ShortcutDebounce::new();
    let immersive_register =
        app.global_shortcut()
            .on_shortcut(immersive_shortcut, move |app, _shortcut, event| {
                if event.state() != ShortcutState::Pressed {
                    return;
                }
                if !immersive_debounce.allow_at(Instant::now()) {
                    return;
                }
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = commands::settings::toggle_immersive_mode(&app).await {
                        tracing::warn!(
                            "immersive shortcut failed label=settings_unavailable error={error}"
                        );
                    }
                });
            });
    match immersive_register {
        Ok(()) => tracing::info!("global immersive shortcut registered"),
        Err(error) => tracing::warn!(
            "global immersive shortcut could not be registered: {error} \
             (another app may already own the combination)"
        ),
    }
    let lock_debounce = ShortcutDebounce::new();
    let lock_register = app
        .global_shortcut()
        .on_shortcut(lock_shortcut, move |app, _, event| {
            if event.state() != ShortcutState::Pressed {
                return;
            }
            if !lock_debounce.allow_at(Instant::now()) {
                return;
            }
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if commands::overlay::toggle_overlay_lock(&app).await.is_err() {
                    tracing::warn!("lock shortcut failed label=settings_unavailable");
                }
            });
        });
    match lock_register {
        Ok(()) => tracing::info!("global lock shortcut registered"),
        Err(error) => tracing::warn!(
            "global lock shortcut could not be registered: {error} \
             (another app may already own the combination)"
        ),
    }
    let display_debounce = ShortcutDebounce::new();
    let display_register =
        app.global_shortcut()
            .on_shortcut(display_shortcut, move |app, _, event| {
                if event.state() != ShortcutState::Pressed {
                    return;
                }
                if !display_debounce.allow_at(Instant::now()) {
                    return;
                }
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if commands::settings::set_subtitle_display_mode(&app, None)
                        .await
                        .is_err()
                    {
                        tracing::warn!(
                            "subtitle display shortcut failed label=settings_unavailable"
                        );
                    }
                });
            });
    match display_register {
        Ok(()) => tracing::info!("global subtitle display shortcut registered"),
        Err(error) => {
            tracing::warn!("global subtitle display shortcut could not be registered: {error}")
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_actions_reject_extra_arguments() {
        for (flag, expected) in [
            ("--toggle-session", DesktopAction::ToggleSession),
            ("--toggle-immersive", DesktopAction::ToggleImmersive),
            ("--toggle-lock", DesktopAction::ToggleLock),
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

    #[test]
    fn monotonic_shortcut_debounce_ignores_repeats_within_window_only() {
        let debounce = ShortcutDebounce::new();
        let start = Instant::now();
        assert!(debounce.allow_at(start));
        assert!(!debounce.allow_at(start + Duration::from_millis(499)));
        assert!(debounce.allow_at(start + Duration::from_millis(500)));
    }
}
