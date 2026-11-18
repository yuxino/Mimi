//! Which pre-existing window states a Dock-only transition may restore.
//! This does not change session state, overlay geometry or saved preferences.

#[derive(Clone, Copy, Debug)]
pub(crate) struct WindowPresentation {
    pub visible: bool,
    pub minimized: bool,
    pub key: bool,
}

pub(crate) fn may_restore_window(was_hidden: bool, before: WindowPresentation) -> bool {
    !was_hidden && before.visible && !before.minimized
}

pub(crate) fn may_restore_focus(
    was_hidden: bool,
    was_active: bool,
    before: WindowPresentation,
) -> bool {
    was_active && before.key && may_restore_window(was_hidden, before)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserve_visible_settings_focus_and_nonkey_subtitle_visibility() {
        let settings = WindowPresentation {
            visible: true,
            minimized: false,
            key: true,
        };
        let overlay = WindowPresentation {
            visible: true,
            minimized: false,
            key: false,
        };
        assert!(may_restore_window(false, settings));
        assert!(may_restore_focus(false, true, settings));
        assert!(may_restore_window(false, overlay));
        assert!(!may_restore_focus(false, true, overlay));
    }

    #[test]
    fn background_change_may_restore_visibility_but_never_steal_focus() {
        let before = WindowPresentation {
            visible: true,
            minimized: false,
            key: true,
        };
        assert!(may_restore_window(false, before));
        assert!(!may_restore_focus(false, false, before));
    }

    #[test]
    fn do_not_reopen_hidden_minimized_or_application_hidden_windows() {
        for before in [
            WindowPresentation {
                visible: false,
                minimized: false,
                key: true,
            },
            WindowPresentation {
                visible: true,
                minimized: true,
                key: true,
            },
        ] {
            assert!(!may_restore_window(false, before));
            assert!(!may_restore_focus(false, true, before));
        }
        let visible = WindowPresentation {
            visible: true,
            minimized: false,
            key: true,
        };
        assert!(!may_restore_window(true, visible));
        assert!(!may_restore_focus(true, true, visible));
    }
}
