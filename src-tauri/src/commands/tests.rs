//! Cross-surface IPC capability contracts and shared synthetic credential fixtures.

use std::sync::Arc;

pub(super) struct BootstrapCredentialStore {
    pub(super) reads: Arc<std::sync::Mutex<Vec<String>>>,
}

impl crate::settings_store::SecretStore for BootstrapCredentialStore {
    fn uses_local_file(&self) -> bool {
        true
    }

    fn load(
        &self,
        _service: &str,
        account: &str,
    ) -> Result<Option<String>, crate::settings_store::SecretStoreError> {
        self.reads.lock().unwrap().push(account.into());
        match account {
            "provider-profile:alibaba-default:alibabaCloud:api-key" => {
                Ok(Some("synthetic-key".into()))
            }
            "provider-profile:bootstrap-custom:customOpenAIASR:text-translation" => {
                Err(crate::settings_store::SecretStoreError::Unavailable)
            }
            _ => Ok(None),
        }
    }

    fn save(
        &self,
        _service: &str,
        _account: &str,
        _value: &str,
    ) -> Result<(), crate::settings_store::SecretStoreError> {
        panic!("Settings bootstrap must not write a credential")
    }

    fn delete(
        &self,
        _service: &str,
        _account: &str,
    ) -> Result<(), crate::settings_store::SecretStoreError> {
        panic!("Settings bootstrap must not delete a credential")
    }
}

pub(super) struct PartiallyUnavailableSecretStore;

impl crate::settings_store::SecretStore for PartiallyUnavailableSecretStore {
    fn load(
        &self,
        _service: &str,
        account: &str,
    ) -> Result<Option<String>, crate::settings_store::SecretStoreError> {
        if account.contains("alibaba-default") {
            Err(crate::settings_store::SecretStoreError::Unavailable)
        } else {
            Ok(None)
        }
    }

    fn save(
        &self,
        _service: &str,
        _account: &str,
        _value: &str,
    ) -> Result<(), crate::settings_store::SecretStoreError> {
        Ok(())
    }

    fn delete(
        &self,
        _service: &str,
        _account: &str,
    ) -> Result<(), crate::settings_store::SecretStoreError> {
        Ok(())
    }
}

#[test]
fn diagnostics_are_registered_and_allowed_only_in_settings() {
    for command in [
        "profile_test_connection",
        "windows_audio_status",
        "support_diagnostics",
        "app_open_support_issue",
        "profile_reveal_credential",
        "profile_credential_editor_state",
        "profile_duplicate",
        "open_tencent_setup_page",
        "open_alibaba_setup_page",
        "open_text_translation_setup_page",
        "open_service_setup_page",
    ] {
        assert!(command_registered(command));
        let permissions = include_str!("../../permissions/app.toml");
        let permitted: Vec<_> = permissions
            .split("[[permission]]")
            .filter(|entry| entry.contains(&format!("\"{command}\"")))
            .collect();
        assert_eq!(permitted.len(), 1);
        assert!(permitted[0].contains("identifier = \"app-settings\""));
        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../../capabilities/settings.json")).unwrap();
        assert!(capability["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|permission| permission == "app-settings"));
        assert_eq!(capability["windows"], serde_json::json!(["settings"]));
    }
}

pub(super) fn command_registered(command: &str) -> bool {
    let handlers = include_str!("../lib.rs")
        .split("tauri::generate_handler![")
        .nth(1)
        .expect("native IPC registration")
        .split("])")
        .next()
        .expect("native IPC registration end");
    handlers.split(',').any(|entry| {
        entry
            .trim()
            .strip_prefix("commands::")
            .is_some_and(|path| path.rsplit("::").next() == Some(command))
    })
}
