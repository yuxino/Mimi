//! Desktop preferences, service profiles and private local credentials.
//!
//! This entry point owns store construction. Responsibilities live in sibling
//! modules; callers keep using `settings_store::{SettingsStore, Preferences, …}`.

#[cfg(test)]
mod animation_switch_tests;
mod configuration;
mod credentials;
mod file_credentials;
#[cfg(all(test, feature = "development-debugger", target_os = "macos"))]
mod gemini_live_tests;
mod native_credentials;
mod persistence;
mod preferences;
mod profiles;
#[cfg(all(test, feature = "development-debugger", target_os = "macos"))]
mod qwen_comparison;
#[cfg(all(test, feature = "development-debugger", target_os = "macos"))]
mod read_only_manual_probe_tests;
mod secret_store;
#[cfg(test)]
mod tests;

pub use crate::core::ui::overlay_layout::DEFAULT_FONT_SIZE;
pub use credentials::CredentialEditorState;
pub use preferences::{
    clamp_visible_subtitle_count, OverlayFrame, Preferences, PulseStyle, SubtitleAlignment,
};
pub use secret_store::{CredentialState, SecretStore, SecretStoreError};

use crate::core::audio::audio_input::AudioInput;
use crate::core::services::provider::ServiceProfile;
use crate::core::ui::overlay_layout::normalize_font_size;
use crate::core::ui::subtitle_font::normalize_family_name;
use credentials::{credential_account, is_default_alibaba};
#[cfg(test)]
use credentials::{OpenAICompatibleDestination, TextTranslationDestination};
use native_credentials::KeyringSecretStore;
#[cfg(test)]
use persistence::atomic_write;
use preferences::normalize_preferences_value;
use profiles::{is_legacy_env_profile_id, ProfileCatalog};
use std::collections::HashMap;
#[cfg(all(test, feature = "development-debugger", target_os = "macos"))]
use std::fs::OpenOptions;
use std::io::ErrorKind;
#[cfg(all(test, feature = "development-debugger", target_os = "macos"))]
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;

pub const MAXIMUM_PROFILE_COUNT: usize = 20;
/// Visible-subtitle-count preference bounds and stored default.
pub const VISIBLE_SUBTITLE_COUNT_RANGE: std::ops::RangeInclusive<u8> = 1..=10;
pub const DEFAULT_VISIBLE_SUBTITLE_COUNT: u8 = 3;

pub const PROFILE_KEYCHAIN_SERVICE: &str = "app.yuxino.mimi.credentials.profiles";
pub const DEVELOPMENT_APPLICATION_IDENTIFIER: &str = "app.yuxino.mimi.dev";
const DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE: &str = "app.yuxino.mimi.dev.credentials.profiles";
pub const LEGACY_KEYCHAIN_SERVICE_V3: &str = "app.yuxino.mimi.credentials.v3";
pub const LEGACY_KEYCHAIN_SERVICE_V2: &str = "app.yuxino.mimi.credentials.v2";
pub const LEGACY_KEYCHAIN_SERVICE: &str = "app.yuxino.mimi.translation";
pub const LEGACY_KEYCHAIN_ACCOUNT: &str = "dashscope-api-key";

const PROFILE_CATALOG_FILE: &str = "service-profiles.json";
const PROFILE_CATALOG_SCHEMA_VERSION: u32 = 1;
const LEGACY_MIGRATION_TOMBSTONE_ACCOUNT: &str = "migration:legacy-alibaba:v1";
const LEGACY_MIGRATION_TOMBSTONE_VALUE: &str = "complete";
const PROFILE_CATALOG_UNAVAILABLE: &str = "Service profile settings are unavailable.";
const CREDENTIAL_STORE_UNAVAILABLE: &str = "credential_store_unavailable";
const CREDENTIAL_SERVICE_UNAVAILABLE: &str = "credential_service_unavailable";
const CREDENTIAL_STORE_ACCESS_DENIED: &str = "credential_store_access_denied";
const PREFERENCES_UNAVAILABLE: &str = "Settings could not be saved.";
const PROFILE_NOT_FOUND: &str = "The service profile does not exist.";
const LAST_PROFILE: &str = "At least one service profile is required.";

type SecretCacheKey = (String, String);

pub struct SettingsStore {
    prefs_path: PathBuf,
    prefs: Mutex<Preferences>,
    catalog_path: PathBuf,
    catalog: Mutex<ProfileCatalog>,
    /// Invalid/unknown catalog data is never overwritten. The runtime keeps a
    /// safe default only so the rest of the app can remain operational.
    catalog_write_blocked: bool,
    secret: Box<dyn SecretStore>,
    profile_keychain_service: &'static str,
    /// Cache each app/profile credential lookup, including unavailable results.
    /// Normal reads use the private local file, never a native-store fallback.
    /// Explicit saves/deletes replace the cached state; diagnostics retry failed
    /// reads for the selected profile, and restart retries all failed reads.
    secret_cache: Mutex<HashMap<SecretCacheKey, Result<Option<String>, SecretStoreError>>>,
    xai_translation_upgrade: Mutex<()>,
    is_ui_test: bool,
    ui_test_preferences_writable: bool,
}

impl SettingsStore {
    /// Loads preferences and `service-profiles.json` from the app config
    /// directory. New installations start with an empty catalog; existing
    /// pre-catalog preferences retain legacy migration. Malformed data is
    /// preserved and profile writes are blocked until it is repaired.
    pub fn load(app_config_dir: PathBuf, is_ui_test: bool, application_identifier: &str) -> Self {
        let is_development = application_identifier == DEVELOPMENT_APPLICATION_IDENTIFIER;
        let profile_keychain_service = if is_development {
            DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE
        } else {
            PROFILE_KEYCHAIN_SERVICE
        };
        if is_ui_test {
            return Self::load_with_secret(
                app_config_dir,
                true,
                Box::new(KeyringSecretStore),
                profile_keychain_service,
            );
        }
        let secret: Box<dyn SecretStore> =
            Box::new(file_credentials::FileCredentialStore::for_app(
                &app_config_dir,
                profile_keychain_service,
                !is_development,
            ));
        // One-time upgrade work. Each completed slot is persisted before the
        // next native read. Steady-state windows use only the local file.
        if secret.pending_imports().unwrap_or(0) > 0 && secret.migrate_legacy().is_err() {
            tracing::warn!("credential import incomplete label=legacy_import_pending");
        }
        Self::load_with_secret(app_config_dir, is_ui_test, secret, profile_keychain_service)
    }

    /// Paid manual probes may only use an already initialized local store.
    /// An unfinished OS import must never prompt or run from a test executable.
    #[cfg(all(test, feature = "development-debugger", target_os = "macos"))]
    pub(crate) fn load_for_manual_probe(directory: PathBuf) -> Result<Self, &'static str> {
        if !directory.is_absolute()
            || !directory.join("credentials-file-mode-complete").is_file()
            || !directory.join("credentials/credentials.json").is_file()
        {
            return Err("saved_development_credentials_required");
        }
        Ok(Self::load(
            directory,
            false,
            DEVELOPMENT_APPLICATION_IDENTIFIER,
        ))
    }

    /// A bounded manual experiment may reuse only a completed development
    /// store. Normalization is in memory and all persistence paths are detached.
    #[cfg(all(test, feature = "development-debugger", target_os = "macos"))]
    pub(crate) fn load_for_read_only_manual_probe(
        directory: PathBuf,
    ) -> Result<Self, &'static str> {
        Self::load_for_scoped_read_only_manual_probe(directory, DEVELOPMENT_APPLICATION_IDENTIFIER)
    }

    /// The caller must explicitly select the app scope for this authorized
    /// manual run. Never discover, copy or fall back to another app's store.
    #[cfg(all(test, feature = "development-debugger", target_os = "macos"))]
    pub(crate) fn load_for_scoped_read_only_manual_probe(
        directory: PathBuf,
        application_identifier: &str,
    ) -> Result<Self, &'static str> {
        use std::io::Read;
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

        let profile_keychain_service = match application_identifier {
            DEVELOPMENT_APPLICATION_IDENTIFIER => DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE,
            "app.yuxino.mimi" => PROFILE_KEYCHAIN_SERVICE,
            _ => return Err("manual_probe_scope_invalid"),
        };

        fn read_existing<T: serde::de::DeserializeOwned>(file: &Path) -> Result<T, &'static str> {
            const MAX_BYTES: u64 = 64 * 1024;
            let reject = || "saved_development_settings_required";
            let metadata = std::fs::symlink_metadata(file).map_err(|_| reject())?;
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.len() > MAX_BYTES
            {
                return Err(reject());
            }
            let opened = OpenOptions::new()
                .read(true)
                .custom_flags(0x100) // macOS O_NOFOLLOW
                .open(file)
                .map_err(|_| reject())?;
            let actual = opened.metadata().map_err(|_| reject())?;
            if metadata.dev() != actual.dev() || metadata.ino() != actual.ino() {
                return Err(reject());
            }
            let mut bytes = Vec::new();
            opened
                .take(MAX_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| reject())?;
            if bytes.len() as u64 > MAX_BYTES {
                return Err(reject());
            }
            serde_json::from_slice(&bytes).map_err(|_| reject())
        }

        let metadata = std::fs::symlink_metadata(&directory)
            .map_err(|_| "saved_development_credentials_required")?;
        if !directory.is_absolute()
            || directory.file_name() != Some(std::ffi::OsStr::new(application_identifier))
            || !metadata.is_dir()
            || metadata.file_type().is_symlink()
        {
            return Err("saved_development_credentials_required");
        }
        let mut prefs: Preferences = read_existing(&directory.join("preferences.json"))?;
        let catalog: ProfileCatalog = read_existing(&directory.join(PROFILE_CATALOG_FILE))?;
        let catalog = catalog
            .validated()
            .map_err(|_| "saved_development_settings_required")?;
        let secret = file_credentials::FileCredentialStore::for_scoped_read_only_manual_probe(
            &directory,
            profile_keychain_service,
        )
        .map_err(|_| "saved_development_credentials_required")?;
        prefs.font_size = normalize_font_size(prefs.font_size);
        prefs.subtitle_font_family = normalize_family_name(&prefs.subtitle_font_family)
            .unwrap_or_default()
            .to_owned();
        prefs.subtitle_background_opacity = prefs.subtitle_background_opacity.min(100);
        if !prefs.audio_input.is_available() {
            prefs.apply_audio_preferences(Some(AudioInput::System), None);
        }
        if let Some(profile) = catalog
            .profiles
            .iter()
            .find(|profile| profile.id == catalog.active_profile_id)
        {
            normalize_preferences_value(&mut prefs, profile);
        }
        Ok(Self {
            prefs_path: PathBuf::new(),
            prefs: Mutex::new(prefs),
            catalog_path: PathBuf::new(),
            catalog: Mutex::new(catalog),
            catalog_write_blocked: false,
            secret: Box::new(secret),
            profile_keychain_service,
            secret_cache: Mutex::new(HashMap::new()),
            xai_translation_upgrade: Mutex::new(()),
            is_ui_test: false,
            ui_test_preferences_writable: false,
        })
    }

    fn load_with_secret(
        app_config_dir: PathBuf,
        is_ui_test: bool,
        secret: Box<dyn SecretStore>,
        profile_keychain_service: &'static str,
    ) -> Self {
        if is_ui_test {
            // UI fixtures never inspect the user's preferences, catalog or keychain.
            let store = Self::in_memory_with_scope(secret, true, profile_keychain_service);
            *store.catalog.lock().unwrap() = ProfileCatalog::default();
            return store;
        }
        let prefs_path = app_config_dir.join("preferences.json");
        let prefs = std::fs::read_to_string(&prefs_path)
            .ok()
            .and_then(|text| serde_json::from_str::<Preferences>(&text).ok())
            .unwrap_or_default();
        let font_size = normalize_font_size(prefs.font_size);
        let prefs = Preferences {
            font_size,
            subtitle_font_family: normalize_family_name(&prefs.subtitle_font_family)
                .unwrap_or_default()
                .to_owned(),
            subtitle_background_opacity: prefs.subtitle_background_opacity.min(100),
            visible_subtitle_count: clamp_visible_subtitle_count(prefs.visible_subtitle_count),
            ..prefs
        };

        let catalog_path = app_config_dir.join(PROFILE_CATALOG_FILE);
        let (catalog, catalog_write_blocked, should_create_catalog) =
            match std::fs::read_to_string(&catalog_path) {
                Ok(text) => match serde_json::from_str::<ProfileCatalog>(&text)
                    .map_err(|_| ())
                    .and_then(ProfileCatalog::validated)
                {
                    Ok(catalog) => (catalog, false, false),
                    Err(()) => {
                        tracing::warn!("service profile catalog unavailable label=invalid_data");
                        (ProfileCatalog::default(), true, false)
                    }
                },
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    let catalog = if prefs_path.is_file() {
                        ProfileCatalog::legacy_alibaba()
                    } else {
                        ProfileCatalog::default()
                    };
                    (catalog, false, true)
                }
                Err(_) => {
                    tracing::warn!("service profile catalog unavailable label=read_failed");
                    (ProfileCatalog::default(), true, false)
                }
            };

        let store = Self {
            prefs_path,
            prefs: Mutex::new(prefs),
            catalog_path,
            catalog: Mutex::new(catalog),
            catalog_write_blocked,
            secret,
            profile_keychain_service,
            secret_cache: Mutex::new(HashMap::new()),
            xai_translation_upgrade: Mutex::new(()),
            is_ui_test,
            ui_test_preferences_writable: false,
        };
        if should_create_catalog && !is_ui_test && store.persist_catalog().is_err() {
            tracing::warn!("service profile catalog unavailable label=create_failed");
        }
        let active_profile = store.active_profile().ok();
        {
            let mut prefs = store.prefs.lock().unwrap();
            let original = prefs.clone();
            if !prefs.audio_input.is_available() {
                prefs.apply_audio_preferences(Some(AudioInput::System), None);
            }
            if let Some(profile) = active_profile {
                normalize_preferences_value(&mut prefs, &profile);
            }
            if *prefs != original && store.persist_preferences_value(&prefs).is_err() {
                tracing::warn!("preferences unavailable label=normalization_write_failed");
            }
        }
        store
    }

    #[cfg(test)]
    pub fn in_memory(secret: Box<dyn SecretStore>, is_ui_test: bool) -> Self {
        Self::in_memory_with_scope(secret, is_ui_test, PROFILE_KEYCHAIN_SERVICE)
    }

    fn in_memory_with_scope(
        secret: Box<dyn SecretStore>,
        is_ui_test: bool,
        profile_keychain_service: &'static str,
    ) -> Self {
        let catalog = ProfileCatalog::legacy_alibaba();
        Self {
            prefs_path: PathBuf::new(),
            prefs: Mutex::new(Preferences::default()),
            catalog_path: PathBuf::new(),
            catalog: Mutex::new(catalog),
            catalog_write_blocked: false,
            secret,
            profile_keychain_service,
            secret_cache: Mutex::new(HashMap::new()),
            xai_translation_upgrade: Mutex::new(()),
            is_ui_test,
            ui_test_preferences_writable: false,
        }
    }

    /// Explicit restart-test opt-in. Only non-secret preferences in an existing,
    /// private temporary fixture directory may persist. Profiles remain in memory.
    pub fn load_ui_test_preferences(directory: PathBuf) -> Result<Self, String> {
        let reject =
            || "UI-test preferences require a private temporary fixture directory".to_string();
        let metadata = std::fs::symlink_metadata(&directory).map_err(|_| reject())?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(reject());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(reject());
            }
        }
        let directory = directory.canonicalize().map_err(|_| reject())?;
        let temporary = std::env::temp_dir().canonicalize().map_err(|_| reject())?;
        if !directory.starts_with(temporary)
            || !directory
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("mimi-ui-test-"))
        {
            return Err(reject());
        }
        let prefs_path = directory.join("preferences.json");
        let prefs = match std::fs::symlink_metadata(&prefs_path) {
            Ok(metadata) => {
                if !metadata.is_file()
                    || metadata.file_type().is_symlink()
                    || metadata.len() > 65536
                {
                    return Err(reject());
                }
                let bytes = std::fs::read(&prefs_path).map_err(|_| reject())?;
                serde_json::from_slice::<Preferences>(&bytes).map_err(|_| reject())?
            }
            Err(error) if error.kind() == ErrorKind::NotFound => Preferences::default(),
            Err(_) => return Err(reject()),
        };
        let mut store = Self::in_memory_with_scope(
            Box::new(KeyringSecretStore),
            true,
            DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE,
        );
        store.prefs_path = prefs_path;
        store.ui_test_preferences_writable = true;
        let font_size = normalize_font_size(prefs.font_size);
        *store.prefs.lock().unwrap() = Preferences {
            font_size,
            subtitle_font_family: normalize_family_name(&prefs.subtitle_font_family)
                .unwrap_or_default()
                .to_owned(),
            subtitle_background_opacity: prefs.subtitle_background_opacity.min(100),
            visible_subtitle_count: clamp_visible_subtitle_count(prefs.visible_subtitle_count),
            ..prefs
        };
        Ok(store)
    }

    #[cfg(test)]
    pub(crate) fn at_path(app_config_dir: PathBuf, secret: Box<dyn SecretStore>) -> Self {
        // Existing provider/storage regressions exercise an explicitly configured
        // installation. First-run tests call load_with_secret without this fixture.
        if !app_config_dir.join(PROFILE_CATALOG_FILE).exists()
            && !app_config_dir.join("preferences.json").exists()
        {
            atomic_write(
                &app_config_dir.join(PROFILE_CATALOG_FILE),
                &serde_json::to_vec(&ProfileCatalog::legacy_alibaba()).unwrap(),
            )
            .unwrap();
        }
        Self::load_with_secret(app_config_dir, false, secret, PROFILE_KEYCHAIN_SERVICE)
    }
}
