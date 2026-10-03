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
