//! Read-only provider presets beside ordinary development file credentials.
//! Never source this file or inspect process environment variables for keys.

#[cfg(test)]
use super::KeyringSecretStore;
use super::{
    SecretStore, SecretStoreError, DEVELOPMENT_APPLICATION_IDENTIFIER,
    DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE, LOCAL_DEV_ALIBABA_PROFILE_ID,
    LOCAL_DEV_GEMINI_PROFILE_ID,
};
#[cfg(unix)]
use std::io::{ErrorKind, Read};
use std::path::Path;

#[cfg(all(test, feature = "local-dev-credentials", target_os = "macos"))]
mod live_tests;
#[cfg(all(test, feature = "local-dev-credentials", target_os = "macos"))]
mod qwen_comparison;

#[cfg(unix)]
const MAX_FILE_BYTES: u64 = 16 * 1024;
const MAX_KEY_BYTES: usize = 4096;

pub(super) fn select(
    directory: &Path,
    is_ui_test: bool,
    identifier: &str,
) -> Option<Box<dyn SecretStore>> {
    if !cfg!(all(feature = "local-dev-credentials", target_os = "macos"))
        || is_ui_test
        || identifier != DEVELOPMENT_APPLICATION_IDENTIFIER
    {
        return None;
    }
    select_with_reader_and_store(
        cfg!(all(feature = "local-dev-credentials", target_os = "macos")),
        is_ui_test,
        identifier,
        Box::new(super::file_credentials::FileCredentialStore::for_app(
            directory,
            DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE,
            false,
        )),
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

#[cfg(test)]
fn select_with_reader(
    enabled: bool,
    is_ui_test: bool,
    identifier: &str,
    read: impl FnOnce() -> Result<Option<String>, SecretStoreError>,
) -> Option<Box<dyn SecretStore>> {
    select_with_reader_and_store(
        enabled,
        is_ui_test,
        identifier,
        Box::new(KeyringSecretStore),
        read,
    )
}

fn select_with_reader_and_store(
    enabled: bool,
    is_ui_test: bool,
    identifier: &str,
    ordinary: Box<dyn SecretStore>,
    read: impl FnOnce() -> Result<Option<String>, SecretStoreError>,
) -> Option<Box<dyn SecretStore>> {
    // All gates precede even file metadata access. UI-only also bypasses this
    // branch in SettingsStore, and ordinary builds do not compile this module.
    if !enabled || is_ui_test || identifier != DEVELOPMENT_APPLICATION_IDENTIFIER {
        return None;
    }
    match read() {
        Ok(None) => None, // Only a genuinely absent file retains OS storage.
        result => {
            let keys = result.and_then(|text| parse(text.as_deref().unwrap_or_default()));
            Some(Box::new(FileSecretStore {
                key: keys
                    .as_ref()
                    .map(|keys| keys.alibaba.clone())
                    .map_err(|error| *error),
                gemini_key: keys
                    .as_ref()
                    .map(|keys| keys.gemini.clone())
                    .map_err(|error| *error),
                gemini_configured: keys.as_ref().map_or(true, |keys| keys.gemini_configured),
                os: ordinary,
            }))
        }
    }
}

struct FileSecretStore {
    // No Debug/Serialize implementation: the value stays inside the backend.
    key: Result<Option<String>, SecretStoreError>,
    gemini_key: Result<Option<String>, SecretStoreError>,
    gemini_configured: bool,
    os: Box<dyn SecretStore>,
}

fn preset_account(account: &str) -> bool {
    [LOCAL_DEV_ALIBABA_PROFILE_ID, LOCAL_DEV_GEMINI_PROFILE_ID]
        .iter()
        .any(|id| account.starts_with(&format!("provider-profile:{id}:")))
}

impl SecretStore for FileSecretStore {
    fn contains(&self, service: &str, account: &str) -> Result<bool, SecretStoreError> {
        if service != DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE
            || !account.starts_with("provider-profile:")
        {
            return Ok(false);
        }
        if preset_account(account) {
            return self.load(service, account).map(|value| value.is_some());
        }
        self.os.contains(service, account)
    }

    fn load(&self, service: &str, account: &str) -> Result<Option<String>, SecretStoreError> {
        if service != DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE
            || !account.starts_with("provider-profile:")
        {
            return Ok(None);
        }
        if preset_account(account) {
            return match account {
                "provider-profile:alibaba-local-dev:alibabaCloud:api-key" => self.key.clone(),
                "provider-profile:gemini-local-dev:googleGeminiLive:api-key" => {
                    self.gemini_key.clone()
                }
                _ => Ok(None),
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

    fn uses_local_file(&self) -> bool {
        self.os.uses_local_file()
    }
    fn pending_imports(&self) -> Result<usize, SecretStoreError> {
        self.os.pending_imports()
    }
    fn migrate_legacy(&self) -> Result<(), SecretStoreError> {
        self.os.migrate_legacy()
    }

    fn local_dev_profile_ids(&self) -> Vec<&'static str> {
        let mut profiles = vec![LOCAL_DEV_ALIBABA_PROFILE_ID];
        if self.gemini_configured {
            profiles.push(LOCAL_DEV_GEMINI_PROFILE_ID);
        }
        profiles
    }
}

#[cfg(test)]
pub(super) fn test_store(
    key: Result<Option<String>, SecretStoreError>,
    os: Box<dyn SecretStore>,
) -> Box<dyn SecretStore> {
    Box::new(FileSecretStore {
        key,
        gemini_key: Ok(None),
        gemini_configured: false,
        os,
    })
}

#[derive(Default)]
struct LocalDevKeys {
    alibaba: Option<String>,
    gemini: Option<String>,
    gemini_configured: bool,
}

fn parse(text: &str) -> Result<LocalDevKeys, SecretStoreError> {
    let invalid = || SecretStoreError::LocalDevFileUnavailable;
    let mut keys = LocalDevKeys::default();
    let mut alibaba_found = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (name, value) = line.split_once('=').ok_or_else(invalid)?;
        let slot = match name.trim() {
            "ALIBABA_API_KEY" if !alibaba_found => {
                alibaba_found = true;
                &mut keys.alibaba
            }
            "GEMINI_API_KEY" if !keys.gemini_configured => {
                keys.gemini_configured = true;
                &mut keys.gemini
            }
            _ => return Err(invalid()),
        };
        let value = value.trim();
        if value.len() > MAX_KEY_BYTES
            || value.bytes().any(|byte| {
                !byte.is_ascii_graphic() || matches!(byte, b'\'' | b'"' | b'`' | b'$' | b'\\')
            })
        {
            return Err(invalid());
        }
        if !value.is_empty() {
            *slot = Some(value.to_owned());
        }
    }
    Ok(keys)
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
    const GEMINI_ACCOUNT: &str = "provider-profile:gemini-local-dev:googleGeminiLive:api-key";

    #[test]
    fn local_dev_qwen_model_survives_catalog_reload_without_editing_credentials() {
        use super::super::{ProfileCatalog, SettingsStore};
        use crate::core::protocols::qwen_mt::QwenMTModel;
        let store = SettingsStore::in_memory_with_scope(
            Box::new(FileSecretStore {
                key: Ok(Some("synthetic-alibaba".into())),
                gemini_key: Ok(None),
                gemini_configured: false,
                os: Box::new(TestOsStore::default()),
            }),
            false,
            DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE,
            false,
        );
        let updated = store
            .update_profile_options_with_model(
                LOCAL_DEV_ALIBABA_PROFILE_ID,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(QwenMTModel::Plus),
            )
            .unwrap();
        store.select_profile(&updated.id).unwrap();
        assert_eq!(
            store.configuration().unwrap().qwen_mt_model,
            QwenMTModel::Plus
        );
        assert_eq!(
            store
                .configuration_for_text_probe(&updated)
                .unwrap()
                .qwen_mt_model,
            QwenMTModel::Plus
        );
        let persisted = serde_json::to_string(&*store.catalog.lock().unwrap()).unwrap();
        let reloaded: ProfileCatalog = serde_json::from_str(&persisted).unwrap();
        let refreshed = reloaded.with_local_dev_profiles(&[LOCAL_DEV_ALIBABA_PROFILE_ID]);
        assert_eq!(
            refreshed
                .profiles
                .iter()
                .find(|p| p.id == updated.id)
                .unwrap()
                .qwen_mt_model,
            QwenMTModel::Plus
        );
        assert!(store
            .save_api_key(&updated.id, "synthetic-replacement")
            .is_err());
    }

    #[test]
    fn local_dev_credentials_gemini_is_independent_read_only_and_never_falls_back() {
        use super::super::SettingsStore;
        use crate::core::credentials::{CredentialRevealField, ProviderCredentials};
        use crate::core::provider::ProviderKind;
        let keys =
            parse("ALIBABA_API_KEY=synthetic-alibaba\nGEMINI_API_KEY=synthetic-gemini\n").unwrap();
        assert_eq!(keys.alibaba.as_deref(), Some("synthetic-alibaba"));
        assert_eq!(keys.gemini.as_deref(), Some("synthetic-gemini"));
        let os = TestOsStore::default();
        os.save(
            DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE,
            GEMINI_ACCOUNT,
            "synthetic-os-fallback",
        )
        .unwrap();
        let secret = Box::new(FileSecretStore {
            key: Ok(keys.alibaba),
            gemini_key: Ok(keys.gemini),
            gemini_configured: true,
            os: Box::new(os.clone()),
        });
        let store = SettingsStore::in_memory_with_scope(
            secret,
            false,
            DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE,
            false,
        );
        let (_, profiles) = store.profile_catalog().unwrap();
        let gemini = profiles
            .iter()
            .find(|p| p.id == LOCAL_DEV_GEMINI_PROFILE_ID)
            .unwrap();
        assert_eq!(gemini.provider, ProviderKind::GoogleGeminiLive);
        assert_eq!(store.profile_credential_storage(&gemini.id), "localDevFile");
        store.select_profile(&gemini.id).unwrap();
        let config = store.configuration().unwrap();
        assert!(
            matches!(config.credentials, ProviderCredentials::ApiKey { api_key } if api_key == "synthetic-gemini")
        );
        for result in [
            store.save_credentials(
                &gemini.id,
                &ProviderCredentials::api_key("synthetic-replacement"),
            ),
            store.delete_api_key(&gemini.id),
            store.delete_profile(&gemini.id),
        ] {
            assert_eq!(result, Err("local_dev_credentials_read_only".into()));
        }
        assert_eq!(
            store.reveal_credential(&gemini.id, CredentialRevealField::ApiKey, None),
            Err("local_dev_credentials_read_only".into())
        );
        let regular = store
            .create_profile(ProviderKind::GoogleGeminiLive, "Regular Gemini")
            .unwrap();
        store.select_profile(&regular.id).unwrap();
        assert!(store.configuration().is_err());
        store
            .save_credentials(
                &regular.id,
                &ProviderCredentials::api_key("synthetic-regular-gemini"),
            )
            .unwrap();
        assert!(
            matches!(store.configuration().unwrap().credentials, ProviderCredentials::ApiKey { api_key } if api_key == "synthetic-regular-gemini")
        );
        let catalog = serde_json::to_string(&store.profile_catalog().unwrap()).unwrap();
        assert!(!catalog.contains("synthetic-gemini"));
        let missing = FileSecretStore {
            key: Ok(None),
            gemini_key: Ok(None),
            gemini_configured: true,
            os: Box::new(os),
        };
        assert_eq!(
            missing
                .load(DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE, GEMINI_ACCOUNT)
                .unwrap(),
            None
        );
        assert_eq!(
            missing
                .load(
                    DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE,
                    "provider-profile:alibaba-local-dev:googleGeminiLive:api-key"
                )
                .unwrap(),
            None
        );
        assert_eq!(
            missing
                .load(
                    DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE,
                    "provider-profile:gemini-local-dev:alibabaCloud:api-key"
                )
                .unwrap(),
            None
        );
        for text in [
            "GEMINI_API_KEY=a\nGEMINI_API_KEY=b",
            "GEMINI_API_KEY=${OTHER}",
            "GEMINI_API_KEY='quoted'",
        ] {
            assert!(matches!(
                parse(text),
                Err(SecretStoreError::LocalDevFileUnavailable)
            ));
        }
    }

    #[test]
    fn local_dev_credentials_two_presets_preserve_selection_and_user_slots() {
        use super::super::{SettingsStore, DEFAULT_ALIBABA_PROFILE_ID, MAXIMUM_PROFILE_COUNT};
        use crate::core::provider::ProviderKind;
        let directory = tempfile::tempdir().unwrap();
        let load = |gemini_configured| {
            SettingsStore::load_with_secret(
                directory.path().into(),
                false,
                Box::new(FileSecretStore {
                    key: Ok(Some("synthetic-alibaba".into())),
                    gemini_key: Ok(Some("synthetic-gemini".into())),
                    gemini_configured,
                    os: Box::new(TestOsStore::default()),
                }),
                DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE,
                false,
            )
        };
        let store = load(true);
        store.select_profile(LOCAL_DEV_GEMINI_PROFILE_ID).unwrap();
        drop(store);
        let reopened = load(true);
        assert_eq!(
            reopened.active_profile().unwrap().id,
            LOCAL_DEV_GEMINI_PROFILE_ID
        );
        for _ in 1..MAXIMUM_PROFILE_COUNT {
            reopened
                .create_profile(ProviderKind::GoogleGeminiLive, "Regular")
                .unwrap();
        }
        assert_eq!(
            reopened.profile_catalog().unwrap().1.len(),
            MAXIMUM_PROFILE_COUNT + 2
        );
        assert!(reopened
            .create_profile(ProviderKind::GoogleGeminiLive, "Too many")
            .is_err());
        drop(reopened);
        let removed = load(false);
        assert_eq!(
            removed.active_profile().unwrap().id,
            LOCAL_DEV_ALIBABA_PROFILE_ID
        );
        assert!(!removed
            .profile_catalog()
            .unwrap()
            .1
            .iter()
            .any(|p| p.id == LOCAL_DEV_GEMINI_PROFILE_ID));
        assert!(removed
            .profile_catalog()
            .unwrap()
            .1
            .iter()
            .any(|p| p.id == DEFAULT_ALIBABA_PROFILE_ID));
    }

    #[cfg(all(feature = "local-dev-credentials", target_os = "macos"))]
    #[tokio::test]
    #[ignore = "explicit live check using the private macOS Gemini dev preset; no audio capture"]
    async fn local_dev_credentials_live_gemini_probe() {
        use super::super::SettingsStore;
        use crate::clients::connection_diagnostics::{check_speech_service, ServiceAvailability};
        let directory = std::path::PathBuf::from(std::env::var_os("HOME").unwrap())
            .join("Library/Application Support/app.yuxino.mimi.dev");
        let secret = select(&directory, false, DEVELOPMENT_APPLICATION_IDENTIFIER).unwrap();
        let store = SettingsStore::in_memory_with_scope(
            secret,
            false,
            DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE,
            false,
        );
        store.select_profile(LOCAL_DEV_GEMINI_PROFILE_ID).unwrap();
        let diagnostic = check_speech_service(&store.configuration().unwrap(), false).await;
        println!("{}", serde_json::to_string(&diagnostic).unwrap());
        assert_eq!(diagnostic.service, ServiceAvailability::Available);
    }

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
                store.local_dev_profile_ids(),
                vec![LOCAL_DEV_ALIBABA_PROFILE_ID, LOCAL_DEV_GEMINI_PROFILE_ID]
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
            parse("# example\nALIBABA_API_KEY=synthetic-test-only\n")
                .unwrap()
                .alibaba,
            Some("synthetic-test-only".into())
        );
        assert_eq!(parse("ALIBABA_API_KEY=\n").unwrap().alibaba, None);
        for text in [
            "ALIBABA_API_KEY='synthetic'",
            "ALIBABA_API_KEY=${OTHER}",
            "ALIBABA_API_KEY=$(example)",
            "ALIBABA_API_KEY=with space",
            "OPENAI_API_KEY=synthetic",
            "ALIBABA_API_KEY=a\nALIBABA_API_KEY=b",
            "ALIBABA_API_KEY=synthetic\nnot-an-assignment",
        ] {
            assert!(matches!(
                parse(text),
                Err(SecretStoreError::LocalDevFileUnavailable)
            ));
        }
        assert!(parse(&format!("ALIBABA_API_KEY={}", "a".repeat(MAX_KEY_BYTES))).is_ok());
        assert!(matches!(
            parse(&format!(
                "ALIBABA_API_KEY={}",
                "a".repeat(MAX_KEY_BYTES + 1)
            )),
            Err(SecretStoreError::LocalDevFileUnavailable)
        ));
    }

    #[test]
    fn local_dev_credentials_preset_has_no_os_fallback_or_secret_writes() {
        let store = FileSecretStore {
            key: Ok(Some("synthetic-test-only".into())),
            gemini_key: Ok(None),
            gemini_configured: false,
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
            gemini_key: Ok(None),
            gemini_configured: false,
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
            gemini_key: Err(SecretStoreError::LocalDevFileUnavailable),
            gemini_configured: true,
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
            "localFile"
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
        assert_eq!(production_store.credential_storage(), "localFile");
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
