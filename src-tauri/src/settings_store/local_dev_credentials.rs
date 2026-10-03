//! A read-only Alibaba preset beside ordinary development OS credentials.
//! Never source this file or inspect process environment variables for keys.

use super::{
    KeyringSecretStore, SecretStore, SecretStoreError, DEVELOPMENT_APPLICATION_IDENTIFIER,
    DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE, LOCAL_DEV_ALIBABA_PROFILE_ID,
};
#[cfg(unix)]
use std::io::{ErrorKind, Read};
use std::path::Path;

#[cfg(unix)]
const MAX_FILE_BYTES: u64 = 16 * 1024;
const MAX_KEY_BYTES: usize = 4096;

pub(super) fn select(
    directory: &Path,
    is_ui_test: bool,
    identifier: &str,
) -> Option<Box<dyn SecretStore>> {
    select_with_reader(
        cfg!(all(feature = "local-dev-credentials", target_os = "macos")),
        is_ui_test,
        identifier,
        || {
            if !directory.is_absolute() {
                // Tauri path-resolution failure must not turn this into a
                // working-directory .env read or a silent Keychain fallback.
                return Err(SecretStoreError::LocalDevFileUnavailable);
            }
            read_file(&directory.join(".env"))
        },
    )
}

fn select_with_reader(
    enabled: bool,
    is_ui_test: bool,
    identifier: &str,
    read: impl FnOnce() -> Result<Option<String>, SecretStoreError>,
) -> Option<Box<dyn SecretStore>> {
    // All gates precede even file metadata access. UI-only also bypasses this
    // branch in SettingsStore, and ordinary builds do not compile this module.
    if !enabled || is_ui_test || identifier != DEVELOPMENT_APPLICATION_IDENTIFIER {
        return None;
    }
    match read() {
        Ok(None) => None, // Only a genuinely absent file retains OS storage.
        result => Some(Box::new(FileSecretStore {
            key: result.and_then(|text| parse(text.as_deref().unwrap_or_default())),
            os: Box::new(KeyringSecretStore),
        })),
    }
}

struct FileSecretStore {
    // No Debug/Serialize implementation: the value stays inside native storage.
    key: Result<Option<String>, SecretStoreError>,
    os: Box<dyn SecretStore>,
}

fn preset_account(account: &str) -> bool {
    account.starts_with(&format!("provider-profile:{LOCAL_DEV_ALIBABA_PROFILE_ID}:"))
}

impl SecretStore for FileSecretStore {
    fn load(&self, service: &str, account: &str) -> Result<Option<String>, SecretStoreError> {
        if service != DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE
            || !account.starts_with("provider-profile:")
        {
            return Ok(None);
        }
        if preset_account(account) {
            return if account
                == format!("provider-profile:{LOCAL_DEV_ALIBABA_PROFILE_ID}:alibabaCloud:api-key")
            {
                self.key.clone()
            } else {
                Ok(None)
            };
        }
        self.os.load(service, account)
    }

    fn save(&self, service: &str, account: &str, value: &str) -> Result<(), SecretStoreError> {
        if preset_account(account)
            || service != DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE
            || !account.starts_with("provider-profile:")
        {
            return Err(SecretStoreError::ReadOnly);
        }
        self.os.save(service, account, value)
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), SecretStoreError> {
        if preset_account(account)
            || service != DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE
            || !account.starts_with("provider-profile:")
        {
            return Err(SecretStoreError::ReadOnly);
        }
        self.os.delete(service, account)
    }

    fn local_dev_profile_id(&self) -> Option<&'static str> {
        Some(LOCAL_DEV_ALIBABA_PROFILE_ID)
    }
}

#[cfg(test)]
pub(super) fn test_store(
    key: Result<Option<String>, SecretStoreError>,
    os: Box<dyn SecretStore>,
) -> Box<dyn SecretStore> {
    Box::new(FileSecretStore { key, os })
}

fn parse(text: &str) -> Result<Option<String>, SecretStoreError> {
    let invalid = || SecretStoreError::LocalDevFileUnavailable;
    let mut key = None;
    let mut found = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (name, value) = line.split_once('=').ok_or_else(invalid)?;
        if name.trim() != "ALIBABA_API_KEY" || found {
            return Err(invalid());
        }
        found = true;
        let value = value.trim();
        if value.len() > MAX_KEY_BYTES
            || value.bytes().any(|byte| {
                !byte.is_ascii_graphic() || matches!(byte, b'\'' | b'"' | b'`' | b'$' | b'\\')
            })
        {
            return Err(invalid());
        }
        if !value.is_empty() {
            key = Some(value.to_owned());
        }
    }
    Ok(key)
}

#[cfg(unix)]
fn read_file(path: &Path) -> Result<Option<String>, SecretStoreError> {
    use std::fs::OpenOptions;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    // Public Unix ABI; no additional dependency or global environment change.
    unsafe extern "C" {
        fn geteuid() -> u32;
    }
    let uid = unsafe { geteuid() };
    let invalid = || SecretStoreError::LocalDevFileUnavailable;
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(invalid()),
    };
    if !valid_metadata(&metadata, uid) {
        return Err(invalid());
    }
    // O_NOFOLLOW prevents a symlink swap between lstat and open. Compare the
    // opened file's identity and permissions, then cap reads even if it grows.
    #[cfg(target_os = "macos")]
    let flags = 0x0100 | 0x0004; // O_NOFOLLOW | O_NONBLOCK (reject raced FIFO).
    #[cfg(not(target_os = "macos"))]
    let flags = 0x20000 | 0x0800;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(flags)
        .open(path)
        .map_err(|_| invalid())?;
    let opened = file.metadata().map_err(|_| invalid())?;
    if !valid_metadata(&opened, uid)
        || opened.dev() != metadata.dev()
        || opened.ino() != metadata.ino()
    {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid())?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(invalid());
    }
    String::from_utf8(bytes).map(Some).map_err(|_| invalid())
}

#[cfg(unix)]
fn valid_metadata(metadata: &std::fs::Metadata, uid: u32) -> bool {
    use std::os::unix::fs::MetadataExt;
    metadata.is_file()
        && metadata.mode() & 0o7777 == 0o600
        && metadata.uid() == uid
        && metadata.len() <= MAX_FILE_BYTES
}

#[cfg(not(unix))]
fn read_file(_: &Path) -> Result<Option<String>, SecretStoreError> {
    // File mode is macOS-only; tests on other platforms exercise the gate and
    // parser without touching secret files.
    Err(SecretStoreError::LocalDevFileUnavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct TestOsStore(Arc<Mutex<HashMap<(String, String), String>>>);

    impl SecretStore for TestOsStore {
        fn load(&self, service: &str, account: &str) -> Result<Option<String>, SecretStoreError> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .get(&(service.into(), account.into()))
                .cloned())
        }
        fn save(&self, service: &str, account: &str, value: &str) -> Result<(), SecretStoreError> {
            self.0
                .lock()
                .unwrap()
                .insert((service.into(), account.into()), value.into());
            Ok(())
        }
        fn delete(&self, service: &str, account: &str) -> Result<(), SecretStoreError> {
            self.0
                .lock()
                .unwrap()
                .remove(&(service.into(), account.into()));
            Ok(())
        }
    }

    const PRESET_ACCOUNT: &str = "provider-profile:alibaba-local-dev:alibabaCloud:api-key";

    #[test]
    fn local_dev_credentials_all_gates_precede_any_file_access() {
        for (enabled, ui_test, identifier) in [
            (false, false, DEVELOPMENT_APPLICATION_IDENTIFIER),
            (true, true, DEVELOPMENT_APPLICATION_IDENTIFIER),
            (true, false, "app.yuxino.mimi"),
            (true, false, "app.yuxino.mimi.dev.other"),
        ] {
            assert!(select_with_reader(enabled, ui_test, identifier, || {
                panic!("disabled/production/UI-only must never inspect the file")
            })
            .is_none());
        }
        let nonexistent = Path::new("/mimi-disabled-secret-file-must-not-be-read");
        assert!(select(nonexistent, true, DEVELOPMENT_APPLICATION_IDENTIFIER).is_none());
        assert!(select(nonexistent, false, "app.yuxino.mimi").is_none());
        #[cfg(not(all(feature = "local-dev-credentials", target_os = "macos")))]
        assert!(select(nonexistent, false, DEVELOPMENT_APPLICATION_IDENTIFIER).is_none());
        #[cfg(all(feature = "local-dev-credentials", target_os = "macos"))]
        {
            let store = select(Path::new(""), false, DEVELOPMENT_APPLICATION_IDENTIFIER).unwrap();
            assert_eq!(
                store.load(DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE, PRESET_ACCOUNT),
                Err(SecretStoreError::LocalDevFileUnavailable)
            );
        }
    }

    #[test]
    fn local_dev_credentials_only_absence_selects_os_storage() {
        assert!(
            select_with_reader(true, false, DEVELOPMENT_APPLICATION_IDENTIFIER, || Ok(None))
                .is_none()
        );
        for result in [
            Err(SecretStoreError::LocalDevFileUnavailable),
            Ok(Some("export ALIBABA_API_KEY=synthetic".into())),
        ] {
            let store =
                select_with_reader(true, false, DEVELOPMENT_APPLICATION_IDENTIFIER, || result)
                    .unwrap();
            assert_eq!(
                store.local_dev_profile_id(),
                Some(LOCAL_DEV_ALIBABA_PROFILE_ID)
            );
            assert_eq!(
                store.load(DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE, PRESET_ACCOUNT),
                Err(SecretStoreError::LocalDevFileUnavailable)
            );
        }
    }

    #[test]
    fn local_dev_credentials_parser_is_literal_bounded_and_strict() {
        assert_eq!(
            parse("# example\nALIBABA_API_KEY=synthetic-test-only\n").unwrap(),
            Some("synthetic-test-only".into())
        );
        assert_eq!(parse("ALIBABA_API_KEY=\n").unwrap(), None);
        for text in [
            "ALIBABA_API_KEY='synthetic'",
            "ALIBABA_API_KEY=${OTHER}",
            "ALIBABA_API_KEY=$(example)",
            "ALIBABA_API_KEY=with space",
            "OPENAI_API_KEY=synthetic",
            "ALIBABA_API_KEY=a\nALIBABA_API_KEY=b",
            "ALIBABA_API_KEY=synthetic\nnot-an-assignment",
        ] {
            assert_eq!(parse(text), Err(SecretStoreError::LocalDevFileUnavailable));
        }
        assert!(parse(&format!("ALIBABA_API_KEY={}", "a".repeat(MAX_KEY_BYTES))).is_ok());
        assert_eq!(
            parse(&format!(
                "ALIBABA_API_KEY={}",
                "a".repeat(MAX_KEY_BYTES + 1)
            )),
            Err(SecretStoreError::LocalDevFileUnavailable)
        );
    }

    #[test]
    fn local_dev_credentials_preset_has_no_os_fallback_or_secret_writes() {
        let store = FileSecretStore {
            key: Ok(Some("synthetic-test-only".into())),
            os: Box::new(TestOsStore::default()),
        };
        assert_eq!(
            store
                .load(DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE, PRESET_ACCOUNT)
                .unwrap(),
            Some("synthetic-test-only".into())
        );
        assert_eq!(
            store.save(
                DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE,
                PRESET_ACCOUNT,
                "synthetic-replacement"
            ),
            Err(SecretStoreError::ReadOnly)
        );
        assert_eq!(
            store.delete(DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE, PRESET_ACCOUNT),
            Err(SecretStoreError::ReadOnly)
        );
        for account in [
            "provider-profile:a:openAIRealtime:api-key",
            "provider-profile:a:deepLX:api-key",
            "provider-profile:a:alibabaCloud:text-translation",
            "provider-profile:a:customDashScopeASR:api-key",
            "provider-profile:a:customOpenAIASR:api-key",
            "provider-profile:a:customDashScopeASR:text-translation",
            "provider-profile:a:customOpenAIASR:text-translation",
            "migration:legacy-alibaba:v1",
            "dashscope-api-key",
        ] {
            assert_eq!(store.load("unused", account).unwrap(), None);
        }
        assert_eq!(
            store.save("unused", "unused", "synthetic"),
            Err(SecretStoreError::ReadOnly)
        );
        assert_eq!(
            store.delete("unused", "unused"),
            Err(SecretStoreError::ReadOnly)
        );
    }

    #[test]
    fn local_dev_credentials_native_settings_use_keys_without_reveal_or_json_writes() {
        use super::super::SettingsStore;
        use crate::core::credentials::{CredentialRevealField, ProviderCredentials};
        use crate::core::provider::{ProviderKind, TextTranslation};
        let secret = Box::new(FileSecretStore {
            key: Ok(Some("synthetic-test-only".into())),
            os: Box::new(TestOsStore::default()),
        });
        let store = SettingsStore::in_memory_with_scope(
            secret,
            false,
            DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE,
            false,
        );
        let profile = store.active_profile().unwrap();
        assert_eq!(
            store.credential_state(&profile),
            super::super::CredentialState::Present
        );
        assert!(store.configuration().is_ok());
        let readonly = "local_dev_credentials_read_only";
        assert_eq!(
            store.save_credentials(
                &profile.id,
                &ProviderCredentials::api_key("synthetic-replacement")
            ),
            Err(readonly.into())
        );
        assert_eq!(store.delete_api_key(&profile.id), Err(readonly.into()));
        assert_eq!(
            store.reveal_credential(&profile.id, CredentialRevealField::ApiKey, None),
            Err(readonly.into())
        );
        assert_eq!(
            store.credential_state(&profile),
            super::super::CredentialState::Present
        );
        let other = store
            .create_profile(ProviderKind::OpenAIRealtime, "Synthetic provider")
            .unwrap();
        assert_eq!(
            store.credential_state(&other),
            super::super::CredentialState::Missing
        );
        let mut mt = profile.clone();
        mt.text_translation = Some(TextTranslation::DeepL);
        assert!(store.credentials_for_profile(&mt).unwrap().is_none());
        store.update_profile(&other.id, "Renamed metadata").unwrap();
        store.delete_profile(&other.id).unwrap();
        let prefs = serde_json::to_string(&store.preferences()).unwrap();
        let catalog = serde_json::to_string(&store.profile_catalog().unwrap()).unwrap();
        assert!(!prefs.contains("synthetic-test-only"));
        assert!(!catalog.contains("synthetic-test-only"));
        assert_eq!(
            store.credential_state(&profile),
            super::super::CredentialState::Present
        );
    }

    #[test]
    fn local_dev_credentials_invalid_file_has_specific_safe_diagnostics() {
        use super::super::SettingsStore;
        let secret = Box::new(FileSecretStore {
            key: Err(SecretStoreError::LocalDevFileUnavailable),
            os: Box::new(TestOsStore::default()),
        });
        let store = SettingsStore::in_memory_with_scope(
            secret,
            false,
            DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE,
            false,
        );
        let profile = store.active_profile().unwrap();
        assert_eq!(store.credential_storage(), "localDevFile");
        assert_eq!(
            store.credential_state(&profile),
            super::super::CredentialState::Unavailable
        );
        assert_eq!(store.credential_diagnostic(&profile), "localDevUnavailable");
        assert_eq!(
            store.configuration().unwrap_err(),
            "local_dev_credentials_unavailable"
        );
        let json =
            serde_json::to_value(crate::commands::SettingsSnapshotPayload::from_store(&store))
                .unwrap();
        assert_eq!(json["credentialStorage"], "localDevFile");
        assert_eq!(json["profiles"][0]["credentialState"], "unavailable");
        let diagnostic =
            crate::clients::connection_diagnostics::ConnectionDiagnostic::credential_failure(
                "localDevUnavailable",
            )
            .unwrap();
        let json = serde_json::to_value(diagnostic).unwrap();
        assert_eq!(json["reason"], "localDevCredentialsUnavailable");
        assert_eq!(json["service"], "unavailable");
    }

    #[cfg(unix)]
    #[test]
    fn local_dev_credentials_file_permission_type_and_size_checks() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let directory = std::env::temp_dir().join(format!(
            "mimi-dev-credentials-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = directory.join(".env");
        assert_eq!(read_file(&path).unwrap(), None);
        std::fs::write(&path, "ALIBABA_API_KEY=synthetic-test-only\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(read_file(&path).unwrap().is_some());
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::metadata(&path).unwrap();
        assert!(!valid_metadata(&metadata, metadata.uid().wrapping_add(1)));
        // Exercise the real SettingsStore selection using this private
        // synthetic fixture, never the user's config directory or Keychain.
        let store = super::super::SettingsStore::load(
            directory.clone(),
            false,
            DEVELOPMENT_APPLICATION_IDENTIFIER,
        );
        let expected = if cfg!(all(feature = "local-dev-credentials", target_os = "macos")) {
            "localDevFile"
        } else {
            "keychain"
        };
        assert_eq!(store.credential_storage(), expected);
        assert!(!store.migrate_legacy_alibaba);
        let ui_store = super::super::SettingsStore::load(
            directory.clone(),
            true,
            DEVELOPMENT_APPLICATION_IDENTIFIER,
        );
        assert_eq!(ui_store.credential_storage(), "keychain");
        let production_store =
            super::super::SettingsStore::load(directory.clone(), false, "app.yuxino.mimi");
        assert_eq!(production_store.credential_storage(), "keychain");
        let catalog = std::fs::read_to_string(directory.join("service-profiles.json")).unwrap();
        assert!(!catalog.contains("synthetic-test-only"));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            read_file(&path),
            Err(SecretStoreError::LocalDevFileUnavailable)
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = directory.join("link");
        symlink(&path, &link).unwrap();
        assert_eq!(
            read_file(&link),
            Err(SecretStoreError::LocalDevFileUnavailable)
        );
        assert_eq!(
            read_file(&directory),
            Err(SecretStoreError::LocalDevFileUnavailable)
        );
        std::fs::write(&path, vec![b'a'; MAX_FILE_BYTES as usize + 1]).unwrap();
        assert_eq!(
            read_file(&path),
            Err(SecretStoreError::LocalDevFileUnavailable)
        );
        std::fs::write(&path, [0xff]).unwrap();
        assert_eq!(
            read_file(&path),
            Err(SecretStoreError::LocalDevFileUnavailable)
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
