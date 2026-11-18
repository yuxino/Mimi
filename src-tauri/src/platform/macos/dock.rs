//! Optional Dock/Cmd-Tab presence; never changes the user's Dock preferences.

#[cfg(target_os = "macos")]
pub(crate) fn policy(show: bool) -> tauri::ActivationPolicy {
    if show {
        tauri::ActivationPolicy::Regular
    } else {
        tauri::ActivationPolicy::Accessory
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn apply(app: &tauri::AppHandle, show: bool) -> Result<(), String> {
    // Capture and restore in the same main-thread transaction, rather than
    // enqueueing a policy change followed by a separate window-show command.
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        let result = apply_on_main_thread(show);
        let _ = send.send(result);
    })
    .map_err(|_| "dock-policy-unavailable".to_string())?;
    receive
        .recv()
        .map_err(|_| "dock-policy-unavailable".to_string())?
}

#[cfg(target_os = "macos")]
fn apply_on_main_thread(show: bool) -> Result<(), String> {
    use crate::core::dock_presentation::{
        may_restore_focus, may_restore_window, WindowPresentation,
    };
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

    let main = MainThreadMarker::new().ok_or("dock-policy-unavailable")?;
    let app = NSApplication::sharedApplication(main);
    let was_hidden = app.isHidden();
    let was_active = app.isActive();
    let key = app.keyWindow();
    let responder = key.as_ref().and_then(|window| window.firstResponder());
    let windows: Vec<_> = app
        .windows()
        .into_iter()
        .map(|window| {
            let presentation = WindowPresentation {
                visible: window.isVisible(),
                minimized: window.isMiniaturized(),
                key: window.isKeyWindow(),
            };
            let can_hide = window.canHide();
            // AppKit may hide windows when demoting Regular to Accessory.
            // Tao's Dock path uses this same protection, but leaves it set.
            window.setCanHide(false);
            (window, can_hide, presentation)
        })
        .collect();
    let changed = app.setActivationPolicy(if show {
        NSApplicationActivationPolicy::Regular
    } else {
        NSApplicationActivationPolicy::Accessory
    });
    if !was_hidden && app.isHidden() {
        app.unhideWithoutActivation();
    }
    for (window, _, before) in &windows {
        if may_restore_window(was_hidden, *before) && !window.isVisible() {
            window.orderFront(None);
        }
    }
    // A background change must not activate Mimi over the user's other app.
    // Restore the existing key window/responder only when Mimi was active.
    if let Some(window) = key {
        if windows
            .iter()
            .any(|(_, _, before)| may_restore_focus(was_hidden, was_active, *before))
        {
            // `activate` requires macOS 14; Mimi supports macOS 13 too.
            // Only restore an application that was already foreground.
            #[allow(deprecated)]
            app.activateIgnoringOtherApps(true);
            window.makeKeyWindow();
            if let Some(responder) = responder {
                window.makeFirstResponder(Some(&responder));
            }
        }
    }
    // Preserve normal Hide behavior after the transition, including panels'
    // original canHide value. Never show an originally hidden/minimized window.
    for (window, can_hide, _) in windows {
        window.setCanHide(can_hide);
    }
    if changed {
        Ok(())
    } else {
        Err("dock-policy-unavailable".into())
    }
}

/// Called under the existing settings mutation guard. Do not publish or
/// persist a changed choice if the runtime request fails. If persistence
/// fails after requesting the new policy, restore the previous policy.
pub(crate) fn save_with_policy(
    previous: bool,
    next: Option<bool>,
    mut apply: impl FnMut(bool) -> Result<(), String>,
    save: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let changed = next.filter(|next| *next != previous);
    if let Some(next) = changed {
        apply(next)?;
    }
    let result = save();
    if result.is_err() && changed.is_some() && apply(previous).is_err() {
        tracing::warn!("Dock preference rollback failed label=dock_policy_unavailable");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::save_with_policy;
    use std::cell::RefCell;

    #[test]
    fn failed_runtime_request_never_persists() {
        let saved = std::cell::Cell::new(false);
        let result = save_with_policy(
            false,
            Some(true),
            |_| Err("unavailable".into()),
            || {
                saved.set(true);
                Ok(())
            },
        );
        assert_eq!(result.unwrap_err(), "unavailable");
        assert!(!saved.get());
    }

    #[test]
    fn persistence_failure_restores_old_policy_before_returning_error() {
        let calls = RefCell::new(Vec::new());
        let result = save_with_policy(
            false,
            Some(true),
            |show| {
                calls.borrow_mut().push(if show { "show" } else { "hide" });
                Ok(())
            },
            || {
                calls.borrow_mut().push("save");
                Err("write-failed".into())
            },
        );
        assert_eq!(result.unwrap_err(), "write-failed");
        assert_eq!(*calls.borrow(), ["show", "save", "hide"]);
    }

    #[test]
    fn unrelated_or_unchanged_save_does_not_touch_app_policy() {
        for next in [None, Some(false)] {
            assert!(
                save_with_policy(false, next, |_| panic!("must not change policy"), || Ok(()))
                    .is_ok()
            );
        }
    }

    #[test]
    fn successful_choice_requests_policy_before_persisting() {
        let calls = RefCell::new(Vec::new());
        assert!(save_with_policy(
            true,
            Some(false),
            |show| {
                assert!(!show);
                calls.borrow_mut().push("hide");
                Ok(())
            },
            || {
                calls.borrow_mut().push("save");
                Ok(())
            }
        )
        .is_ok());
        assert_eq!(*calls.borrow(), ["hide", "save"]);
    }
}
