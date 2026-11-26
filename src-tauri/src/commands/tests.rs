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

#[test]
fn every_registered_command_matches_app_permissions() {
    let template = include_str!("../../permissions/app.toml");
    let registered: std::collections::BTreeSet<String> = registered_commands()
        .into_iter()
        .map(str::to_owned)
        .collect();
    let permitted = permitted_commands(template);
    assert_eq!(
        registered, permitted,
        "every Tauri command registered in lib.rs must match permissions/app.toml",
    );

    let production = crate::core::development::development_build_permissions::permissions_for_build(
        template, false,
    );
    let production_permitted = permitted_commands(&production);
    let expected_production: std::collections::BTreeSet<String> = registered
        .iter()
        .filter(|command| !command.starts_with("development_debug_"))
        .cloned()
        .collect();
    assert_eq!(
        expected_production, production_permitted,
        "production builds must permit every non-debug registered command",
    );

    let mut context = crate::app_context();
    let authority = context.runtime_authority_mut();
    for command in &registered {
        let allowed_in_any_window =
            ["settings", "overlay", "overlay-control"]
                .iter()
                .any(|window| {
                    authority
                        .resolve_access(command, window, window, &tauri::ipc::Origin::Local)
                        .is_some()
                });
        let expected_allowed =
            cfg!(feature = "development-debugger") || !command.starts_with("development_debug_");
        assert_eq!(
            allowed_in_any_window, expected_allowed,
            "compiled Tauri runtime ACL mismatch for command {command}",
        );
    }
}

#[test]
fn session_history_commands_are_registered_and_allowed_only_in_settings() {
    let permissions = include_str!("../../permissions/app.toml");
    let mut context = crate::app_context();
    let authority = context.runtime_authority_mut();
    for command in [
        "session_archive_state",
        "session_transcript_page",
        "session_history_list",
        "session_history_page",
        "session_history_audio",
        "session_archive_audio",
        "session_history_delete",
        "session_history_delete_many",
        "session_history_rename",
        "session_archive_clear",
        "session_export",
    ] {
        assert!(command_registered(command), "{command}");
        let permitted: Vec<_> = permissions
            .split("[[permission]]")
            .filter(|entry| entry.contains(&format!("\"{command}\"")))
            .collect();
        assert_eq!(permitted.len(), 1, "{command}");
        assert!(
            permitted[0].contains("identifier = \"app-settings\""),
            "{command}"
        );
        assert!(
            authority
                .resolve_access(command, "settings", "settings", &tauri::ipc::Origin::Local)
                .is_some(),
            "settings window must be allowed to invoke {command}",
        );
        for other_window in ["overlay", "overlay-control"] {
            assert!(
                authority
                    .resolve_access(
                        command,
                        other_window,
                        other_window,
                        &tauri::ipc::Origin::Local,
                    )
                    .is_none(),
                "{other_window} window must not be allowed to invoke {command}",
            );
        }
    }
}

pub(super) fn command_registered(command: &str) -> bool {
    registered_commands().contains(command)
}

fn registered_commands() -> std::collections::BTreeSet<&'static str> {
    let handlers = include_str!("../lib.rs")
        .split("tauri::generate_handler![")
        .nth(1)
        .expect("native IPC registration")
        .split("])")
        .next()
        .expect("native IPC registration end");
    handlers
        .split(',')
        .filter_map(|entry| {
            let path = entry.lines().rev().map(str::trim).find(|line| {
                !line.is_empty() && !line.starts_with('#') && !line.starts_with("//")
            })?;
            path.rsplit("::").next().filter(|name| !name.is_empty())
        })
        .collect()
}

fn permitted_commands(template: &str) -> std::collections::BTreeSet<String> {
    let mut allowed = std::collections::BTreeSet::new();
    for section in template.split("[[permission]]").skip(1) {
        let Some(block) = section
            .split("commands.allow = [")
            .nth(1)
            .and_then(|rest| rest.split(']').next())
        else {
            continue;
        };
        for line in block.lines() {
            let trimmed = line.trim();
            if let Some(command) = trimmed
                .strip_prefix('"')
                .and_then(|rest| rest.split('"').next())
            {
                if !command.is_empty() {
                    allowed.insert(command.to_owned());
                }
            }
        }
    }
    allowed
}
