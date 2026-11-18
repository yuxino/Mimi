//! The system lane can capture the whole system or one explicitly selected app.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SystemAudioTarget {
    #[default]
    System,
    Application {
        id: String,
        name: String,
    },
}

impl SystemAudioTarget {
    pub fn application_id(&self) -> Option<&str> {
        match self {
            Self::System => None,
            Self::Application { id, .. } => Some(id),
        }
    }

    pub fn application_name(&self) -> Option<&str> {
        match self {
            Self::System => None,
            Self::Application { name, .. } => Some(name),
        }
    }

    pub fn validate(&self) -> bool {
        match self {
            Self::System => true,
            Self::Application { id, name } => {
                !id.trim().is_empty()
                    && id.len() <= 2048
                    && !name.trim().is_empty()
                    && name.len() <= 256
                    && !id.chars().any(char::is_control)
                    && !name.chars().any(char::is_control)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_is_default_and_application_identity_survives_round_trip() {
        assert_eq!(SystemAudioTarget::default(), SystemAudioTarget::System);
        let app = SystemAudioTarget::Application {
            id: "com.example.player".into(),
            name: "Player".into(),
        };
        let wire = serde_json::to_value(&app).unwrap();
        assert_eq!(wire["kind"], "application");
        assert_eq!(
            serde_json::from_value::<SystemAudioTarget>(wire).unwrap(),
            app
        );
        assert_eq!(app.application_id(), Some("com.example.player"));
        assert!(SystemAudioTarget::System.application_id().is_none());
        assert!(serde_json::from_str::<SystemAudioTarget>(
            r#"{"kind":"application","name":"Player"}"#
        )
        .is_err());
    }

    #[test]
    fn invalid_or_unbounded_targets_are_rejected() {
        for (id, name) in [("", "Player"), ("app", " "), ("app\n", "Player")] {
            assert!(!SystemAudioTarget::Application {
                id: id.into(),
                name: name.into()
            }
            .validate());
        }
        assert!(!SystemAudioTarget::Application {
            id: "x".repeat(2049),
            name: "Player".into()
        }
        .validate());
    }
}

/// Metadata-only ownership check for audio helpers; never match bundle prefixes.
/// Browser helpers can live inside an app bundle under a different bundle ID.
#[cfg(any(target_os = "macos", test))]
pub fn application_owns_audio_process(
    own_pid: i32,
    pid: i32,
    process_bundle_id: Option<&str>,
    application_id: &str,
    application_pids: &[i32],
    application_paths: &[std::path::PathBuf],
    executable_path: Option<&std::path::Path>,
) -> bool {
    if pid <= 0 || pid == own_pid {
        return false;
    }
    process_bundle_id == Some(application_id)
        || application_pids.contains(&pid)
        || executable_path
            .is_some_and(|path| application_paths.iter().any(|app| path.starts_with(app)))
}

/// External XPC helpers require an exact, live application responsibility PID.
/// Unknown or own-process ownership must never expand an inclusion list.
#[cfg(any(target_os = "macos", test))]
pub fn application_owns_responsible_audio_process(
    own_pid: i32,
    pid: i32,
    responsible_pid: Option<i32>,
    application_pids: &[i32],
) -> bool {
    pid > 0
        && pid != own_pid
        && responsible_pid.is_some_and(|responsible| {
            responsible > 0 && responsible != own_pid && application_pids.contains(&responsible)
        })
}

#[cfg(test)]
mod process_tests {
    use super::{application_owns_audio_process, application_owns_responsible_audio_process};
    use std::path::{Path, PathBuf};

    #[test]
    fn includes_exact_app_and_nested_helpers_but_never_own_or_neighboring_apps() {
        let app = "example.browser";
        let paths = [PathBuf::from("/Applications/Browser.app")];
        let matches = |pid, bundle, path| {
            application_owns_audio_process(99, pid, bundle, app, &[10], &paths, path)
        };
        assert!(matches(10, None, None));
        assert!(matches(11, Some(app), None));
        assert!(matches(
            12,
            Some("example.browser.helper"),
            Some(Path::new(
                "/Applications/Browser.app/Contents/Frameworks/Helper.app/Contents/MacOS/Helper"
            ))
        ));
        assert!(!matches(
            99,
            Some(app),
            Some(Path::new(
                "/Applications/Browser.app/Contents/MacOS/Browser"
            ))
        ));
        assert!(!matches(13, Some("example.browser.other"), None));
        assert!(!matches(
            14,
            None,
            Some(Path::new(
                "/Applications/Browser.app.backup/Contents/MacOS/Browser"
            ))
        ));
        assert!(!matches(
            15,
            None,
            Some(Path::new("/Applications/Other.app/Contents/MacOS/Other"))
        ));
        assert!(!matches(0, Some(app), None));
    }

    #[test]
    fn external_helpers_require_exact_live_responsibility_and_exclude_own_audio() {
        let matches = |pid, responsible, apps: &[i32]| {
            application_owns_responsible_audio_process(99, pid, responsible, apps)
        };
        assert!(matches(20, Some(10), &[10]));
        assert!(matches(21, Some(11), &[10, 11]));
        assert!(!matches(20, None, &[10]));
        assert!(!matches(20, Some(-1), &[10]));
        assert!(!matches(20, Some(0), &[10]));
        assert!(!matches(20, Some(12), &[10, 11]));
        assert!(!matches(20, Some(10), &[]));
        assert!(!matches(99, Some(10), &[10]));
        assert!(!matches(20, Some(99), &[99]));
        assert!(!matches(0, Some(10), &[10]));
    }
}
