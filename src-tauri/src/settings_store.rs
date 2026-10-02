//! Non-secret preferences, service-profile metadata, and native credentials.
//!
//! General preferences and the profile catalog are separate JSON documents.
//! API keys never enter either document: normal/release builds use a scoped OS
//! account. The explicit macOS dev feature can select a private read-only file.

#[cfg(any(all(feature = "local-dev-credentials", target_os = "macos"), test))]
mod local_dev_credentials;

use crate::core::configuration::LiveTranslationConfiguration;
use crate::core::credentials::{CredentialRevealField, ProviderCredentials};
use crate::core::models::{
    SourceLanguage, SubtitleColor, SubtitleDisplayMode, TargetLanguage, TranslationMode,
};
use crate::core::network_proxy::ProxyConfig;
use crate::core::provider::{
    ProviderKind, ProviderPreferences, ServiceProfile, TextTranslation, DEFAULT_ALIBABA_PROFILE_ID,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
#[cfg(unix)]
use std::fs::File;
use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

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

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TextTranslationDestination {
    endpoint: String,
    token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    deep_l_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    open_ai_compatible: Option<OpenAICompatibleDestination>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenAICompatibleDestination {
    endpoint: String,
    api_key: String,
    model: String,
}

pub const MAXIMUM_PROFILE_COUNT: usize = 20;
pub const FONT_SIZE_RANGE: std::ops::RangeInclusive<f64> = 14.0..=20.0;
pub const DEFAULT_FONT_SIZE: f64 = 18.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubtitleAlignment {
    Left,
    #[default]
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PulseStyle {
    Syllable,
    #[default]
    #[serde(other)]
    Ribbon,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OverlayFrame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub source_language: SourceLanguage,
    pub target_language: TargetLanguage,
    pub translation_mode: TranslationMode,
    pub font_size: f64,
    pub subtitle_color: SubtitleColor,
    pub subtitle_alignment: SubtitleAlignment,
    pub subtitle_display_mode: SubtitleDisplayMode,
    pub show_subtitle_dividers: bool,
    /// Animation switches: `None` follows the system reduce-motion setting.
    pub pulse_animation: Option<bool>,
    pub pulse_style: PulseStyle,
    pub subtitle_animation: Option<bool>,
    pub subtitle_blends_with_background: bool,
    pub overlay_locked: bool,
    pub overlay_frame: Option<OverlayFrame>,
    pub frame_layout_version: u64,
    /// UI language override. `None` and `Some("system")` both follow the
    /// operating-system language.
    pub ui_language: Option<String>,
    pub retain_session_history: bool,
    pub record_session_audio: bool,
    /// Empty means follow the Windows default output, including live changes.
    pub windows_audio_source: String,
    /// macOS Dock/Cmd-Tab presence. Legacy installations remain accessory utilities.
    pub show_in_dock: bool,
    pub network_proxy: ProxyConfig,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            source_language: SourceLanguage::Automatic,
            target_language: TargetLanguage::SimplifiedChinese,
            translation_mode: TranslationMode::Turbo,
            font_size: DEFAULT_FONT_SIZE,
            subtitle_color: SubtitleColor::White,
            subtitle_alignment: SubtitleAlignment::Center,
            subtitle_display_mode: SubtitleDisplayMode::Translation,
            show_subtitle_dividers: false,
            pulse_animation: None,
            pulse_style: PulseStyle::Ribbon,
            subtitle_animation: None,
            subtitle_blends_with_background: false,
            overlay_locked: false,
            overlay_frame: None,
            frame_layout_version: 0,
            ui_language: None,
            retain_session_history: false,
            record_session_audio: false,
            windows_audio_source: String::new(),
            show_in_dock: false,
            network_proxy: ProxyConfig::default(),
        }
    }
}

/// Public, deliberately coarse credential state. OS/keyring error details are
/// never serialized to the frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CredentialState {
    Present,
    Missing,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretStoreError {
    Unavailable,
    ReadOnly,
    #[cfg(any(all(feature = "local-dev-credentials", target_os = "macos"), test))]
    LocalDevFileUnavailable,
    #[cfg(any(target_os = "linux", test))]
    ServiceUnavailable,
    #[cfg(any(target_os = "linux", test))]
    AccessDenied,
}

impl SecretStoreError {
    fn public_error(self) -> String {
        match self {
            Self::Unavailable => CREDENTIAL_STORE_UNAVAILABLE,
            Self::ReadOnly => "local_dev_credentials_read_only",
            #[cfg(any(all(feature = "local-dev-credentials", target_os = "macos"), test))]
            Self::LocalDevFileUnavailable => "local_dev_credentials_unavailable",
            #[cfg(any(target_os = "linux", test))]
            Self::ServiceUnavailable => CREDENTIAL_SERVICE_UNAVAILABLE,
            #[cfg(any(target_os = "linux", test))]
            Self::AccessDenied => CREDENTIAL_STORE_ACCESS_DENIED,
        }
        .to_string()
    }
}

#[cfg(any(target_os = "linux", test))]
fn secret_service_operation_error(error: keyring_core::Error) -> SecretStoreError {
    // NoStorageAccess covers locked storage, dismissed prompts, and unavailable
    // results. Do not assert that the collection is locked or expose its payload.
    match error {
        keyring_core::Error::NoStorageAccess(_) => SecretStoreError::AccessDenied,
        _ => SecretStoreError::Unavailable,
    }
}

fn keyring_operation_error(error: keyring_core::Error) -> SecretStoreError {
    #[cfg(target_os = "linux")]
    {
        secret_service_operation_error(error)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = error;
        SecretStoreError::Unavailable
    }
}

/// Small keyed abstraction over the OS credential store. Tests provide an
/// in-memory implementation; production uses `keyring`.
pub trait SecretStore: Send + Sync {
    fn load(&self, service: &str, account: &str) -> Result<Option<String>, SecretStoreError>;
    fn save(&self, service: &str, account: &str, value: &str) -> Result<(), SecretStoreError>;
    fn delete(&self, service: &str, account: &str) -> Result<(), SecretStoreError>;
    fn is_read_only(&self) -> bool {
        false
    }
}

struct KeyringSecretStore;

#[cfg(target_os = "windows")]
// `windows-native-keyring-store` maps `local` to
// `CRED_PERSIST_LOCAL_MACHINE` in Windows Credential Manager.
const WINDOWS_CREDENTIAL_PERSISTENCE: &str = "local";

fn credential_entry(service: &str, account: &str) -> Result<keyring_core::Entry, SecretStoreError> {
    #[cfg(target_os = "windows")]
    {
        // `keyring` owns one-time selection of the native platform store. Its
        // compatibility wrapper does not expose store modifiers, so initialise
        // it there and construct the actual entry through keyring-core.
        if keyring::Entry::store_status().is_err() {
            return Err(SecretStoreError::Unavailable);
        }
        let modifiers = HashMap::from([("persistence", WINDOWS_CREDENTIAL_PERSISTENCE)]);
        keyring_core::Entry::new_with_modifiers(service, account, &modifiers)
            .map_err(|_| SecretStoreError::Unavailable)
    }

    #[cfg(target_os = "linux")]
    {
        // keyring's compatibility Entry caches its first store initialization
        // in a LazyLock, including failure. A desktop Secret Service can appear
        // later, so use the same backend directly with a fresh encrypted session.
        use keyring_core::api::CredentialStoreApi;
        // An initialization failure means no encrypted service session could
        // be established; it does not prove that a provider is uninstalled.
        let store = zbus_secret_service_keyring_store::Store::new()
            .map_err(|_| SecretStoreError::ServiceUnavailable)?;
        store
            .build(service, account, None)
            .map_err(keyring_operation_error)
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        keyring::Entry::new(service, account)
            .map(|entry| entry.inner)
            .map_err(|_| SecretStoreError::Unavailable)
    }
}

#[cfg(any(target_os = "windows", test))]
fn ensure_local_credential_persistence(
    password: &str,
    mut read_attributes: impl FnMut() -> Result<HashMap<String, String>, SecretStoreError>,
    rewrite: impl FnOnce(&str) -> Result<(), SecretStoreError>,
    mut retry_pause: impl FnMut(),
) -> Result<(), SecretStoreError> {
    const VERIFICATION_ATTEMPTS: usize = 3;
    let is_local = |attributes: &HashMap<String, String>| {
        attributes
            .get("persistence")
            .is_some_and(|value| value.eq_ignore_ascii_case("local"))
    };
    let mut initial_attributes = None;
    for attempt in 0..VERIFICATION_ATTEMPTS {
        if let Ok(attributes) = read_attributes() {
            initial_attributes = Some(attributes);
            break;
        }
        if attempt + 1 < VERIFICATION_ATTEMPTS {
            retry_pause();
        }
    }
    let Some(initial_attributes) = initial_attributes else {
        return Err(SecretStoreError::Unavailable);
    };
    if is_local(&initial_attributes) {
        return Ok(());
    }

    // Entries written by earlier Mimi builds used Windows' Enterprise
    // persistence. Rewriting the same target with a local-spec entry keeps the
    // secret value while stopping future credential roaming.
    rewrite(password)?;
    // Windows Credential Manager can briefly expose stale attributes after a
    // persistence rewrite. Verify a bounded number of times instead of
    // caching that transient state as unavailable until restart.
    for attempt in 0..VERIFICATION_ATTEMPTS {
        if read_attributes().is_ok_and(|attributes| is_local(&attributes)) {
            return Ok(());
        }
        if attempt + 1 < VERIFICATION_ATTEMPTS {
            retry_pause();
        }
    }
    Err(SecretStoreError::Unavailable)
}

#[cfg(target_os = "windows")]
fn enforce_local_credential_persistence(
    entry: &keyring_core::Entry,
    password: &str,
) -> Result<(), SecretStoreError> {
    ensure_local_credential_persistence(
        password,
        || {
            entry
                .get_attributes()
                .map_err(|_| SecretStoreError::Unavailable)
        },
        |value| {
            entry
                .set_password(value)
                .map_err(|_| SecretStoreError::Unavailable)
        },
        || std::thread::sleep(std::time::Duration::from_millis(25)),
    )
}

impl SecretStore for KeyringSecretStore {
    fn load(&self, service: &str, account: &str) -> Result<Option<String>, SecretStoreError> {
        let entry = credential_entry(service, account)?;
        match entry.get_password() {
            Ok(password) => {
                #[cfg(target_os = "windows")]
                enforce_local_credential_persistence(&entry, &password)?;
                Ok(Some(password))
            }
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(error) => Err(keyring_operation_error(error)),
        }
    }

    fn save(&self, service: &str, account: &str, value: &str) -> Result<(), SecretStoreError> {
        let entry = credential_entry(service, account)?;
        entry.set_password(value).map_err(keyring_operation_error)?;
        #[cfg(target_os = "windows")]
        enforce_local_credential_persistence(&entry, value)?;
        Ok(())
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), SecretStoreError> {
        let entry = credential_entry(service, account)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(error) => Err(keyring_operation_error(error)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProfileCatalog {
    schema_version: u32,
    active_profile_id: String,
    profiles: Vec<ServiceProfile>,
}

impl Default for ProfileCatalog {
    fn default() -> Self {
        Self {
            schema_version: PROFILE_CATALOG_SCHEMA_VERSION,
            active_profile_id: DEFAULT_ALIBABA_PROFILE_ID.to_string(),
            profiles: vec![ServiceProfile::alibaba_default()],
        }
    }
}

impl ProfileCatalog {
    fn validated(self) -> Result<Self, ()> {
        if self.schema_version != PROFILE_CATALOG_SCHEMA_VERSION
            || self.profiles.is_empty()
            || self.profiles.len() > MAXIMUM_PROFILE_COUNT
        {
            return Err(());
        }

        let mut ids = HashSet::with_capacity(self.profiles.len());
        let mut profiles = Vec::with_capacity(self.profiles.len());
        for profile in self.profiles {
            let validated = profile.validated().map_err(|_| ())?;
            if !ids.insert(validated.id.clone()) {
                return Err(());
            }
            profiles.push(validated);
        }
        if !ids.contains(&self.active_profile_id) {
            return Err(());
        }

        Ok(Self {
            schema_version: self.schema_version,
            active_profile_id: self.active_profile_id,
            profiles,
        })
    }
}

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
    migrate_legacy_alibaba: bool,
    /// Every first read is cached, including unavailable results. This keeps
    /// concurrent windows from triggering repeated OS authorization prompts.
    /// Explicit saves/deletes replace the cached state; diagnostics retry failed
    /// reads for the selected profile, and restart retries all failed reads.
    secret_cache: Mutex<HashMap<SecretCacheKey, Result<Option<String>, SecretStoreError>>>,
    is_ui_test: bool,
    ui_test_preferences_writable: bool,
}

impl SettingsStore {
    /// Loads preferences and `service-profiles.json` from the app config
    /// directory. A missing catalog is created atomically with Alibaba as the
    /// active default; malformed data is preserved and all profile writes are
    /// blocked until it is repaired.
    pub fn load(app_config_dir: PathBuf, is_ui_test: bool, application_identifier: &str) -> Self {
        let is_development = application_identifier == DEVELOPMENT_APPLICATION_IDENTIFIER;
        let profile_keychain_service = if is_development {
            DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE
        } else {
            PROFILE_KEYCHAIN_SERVICE
        };
        let secret: Box<dyn SecretStore> = Box::new(KeyringSecretStore);
        #[cfg(any(all(feature = "local-dev-credentials", target_os = "macos"), test))]
        let secret =
            local_dev_credentials::select(&app_config_dir, is_ui_test, application_identifier)
                .unwrap_or(secret);
        let is_file_mode = secret.is_read_only();
        Self::load_with_secret(
            app_config_dir,
            is_ui_test,
            secret,
            profile_keychain_service,
            !is_development && !is_file_mode,
        )
    }

    fn load_with_secret(
        app_config_dir: PathBuf,
        is_ui_test: bool,
        secret: Box<dyn SecretStore>,
        profile_keychain_service: &'static str,
        migrate_legacy_alibaba: bool,
    ) -> Self {
        if is_ui_test {
            // UI fixtures never inspect the user's preferences, catalog or keychain.
            return Self::in_memory_with_scope(secret, true, profile_keychain_service, false);
        }
        let prefs_path = app_config_dir.join("preferences.json");
        let prefs = std::fs::read_to_string(&prefs_path)
            .ok()
            .and_then(|text| serde_json::from_str::<Preferences>(&text).ok())
            .unwrap_or_default();
        let font_size = prefs
            .font_size
            .clamp(*FONT_SIZE_RANGE.start(), *FONT_SIZE_RANGE.end());
        let prefs = Preferences { font_size, ..prefs };

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
                    (ProfileCatalog::default(), false, true)
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
            migrate_legacy_alibaba,
            secret_cache: Mutex::new(HashMap::new()),
            is_ui_test,
            ui_test_preferences_writable: false,
        };
        if should_create_catalog && !is_ui_test && store.persist_catalog().is_err() {
            tracing::warn!("service profile catalog unavailable label=create_failed");
        }
        if !store.catalog_write_blocked {
            let profile = store.active_profile().unwrap_or_default();
            let mut prefs = store.prefs.lock().unwrap();
            let original = prefs.clone();
            normalize_preferences_value(&mut prefs, &profile);
            if *prefs != original && store.persist_preferences_value(&prefs).is_err() {
                tracing::warn!("preferences unavailable label=normalization_write_failed");
            }
        }
        store
    }

    #[cfg(test)]
    pub fn in_memory(secret: Box<dyn SecretStore>, is_ui_test: bool) -> Self {
        Self::in_memory_with_scope(secret, is_ui_test, PROFILE_KEYCHAIN_SERVICE, true)
    }

    fn in_memory_with_scope(
        secret: Box<dyn SecretStore>,
        is_ui_test: bool,
        profile_keychain_service: &'static str,
        migrate_legacy_alibaba: bool,
    ) -> Self {
        Self {
            prefs_path: PathBuf::new(),
            prefs: Mutex::new(Preferences::default()),
            catalog_path: PathBuf::new(),
            catalog: Mutex::new(ProfileCatalog::default()),
            catalog_write_blocked: false,
            secret,
            profile_keychain_service,
            migrate_legacy_alibaba,
            secret_cache: Mutex::new(HashMap::new()),
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
            false,
        );
        store.prefs_path = prefs_path;
        store.ui_test_preferences_writable = true;
        let font_size = prefs
            .font_size
            .clamp(*FONT_SIZE_RANGE.start(), *FONT_SIZE_RANGE.end());
        *store.prefs.lock().unwrap() = Preferences { font_size, ..prefs };
        Ok(store)
    }

    #[cfg(test)]
    fn at_path(app_config_dir: PathBuf, secret: Box<dyn SecretStore>) -> Self {
        Self::load_with_secret(
            app_config_dir,
            false,
            secret,
            PROFILE_KEYCHAIN_SERVICE,
            true,
        )
    }

    pub fn is_ui_test(&self) -> bool {
        self.is_ui_test
    }

    pub fn preferences(&self) -> Preferences {
        let mut prefs = self.prefs.lock().unwrap().clone();
        // Even a blocked profile catalog or legacy UI fixture must expose the
        // one current mode instead of restoring a removed selector choice.
        prefs.translation_mode = TranslationMode::Turbo;
        prefs
    }

    /// Applies and durably persists a preference update as one in-process
    /// transaction. A failed write leaves the published in-memory snapshot
    /// unchanged, so callers never report settings that will vanish on restart.
    pub fn save_preferences(&self, update: impl FnOnce(&mut Preferences)) -> Result<(), String> {
        self.save_preferences_validated(update, |_, _| Ok(()))
    }

    fn save_preferences_validated(
        &self,
        update: impl FnOnce(&mut Preferences),
        validate: impl FnOnce(&Preferences, &mut Preferences) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut current = self.prefs.lock().unwrap();
        let mut next = current.clone();
        update(&mut next);
        validate(&current, &mut next)?;
        next.translation_mode = TranslationMode::Turbo;
        next.font_size = next
            .font_size
            .clamp(*FONT_SIZE_RANGE.start(), *FONT_SIZE_RANGE.end());
        next.network_proxy = next
            .network_proxy
            .validate()
            .map_err(|error| error.to_string())?;
        self.persist_preferences_value(&next)?;
        *current = next;
        Ok(())
    }

    /// Applies a preference update and the active provider's capability
    /// normalization before committing either change.
    pub fn save_preferences_for_active_profile(
        &self,
        update: impl FnOnce(&mut Preferences),
    ) -> Result<(), String> {
        let profile = self.active_profile()?;
        self.save_preferences_validated(update, |previous, next| {
            let capabilities = profile.capabilities(next.target_language);
            if next.target_language != previous.target_language
                && !capabilities.target_languages.contains(&next.target_language) {
                return Err(crate::core::configuration::LiveTranslationConfigurationError::UnsupportedTargetLanguage.to_string());
            }
            if next.source_language != previous.source_language
                && !capabilities.source_languages.contains(&next.source_language) {
                return Err(crate::core::configuration::LiveTranslationConfigurationError::UnsupportedSourceLanguage.to_string());
            }
            normalize_preferences_value(next, &profile);
            Ok(())
        })
    }

    pub fn profile_catalog(&self) -> Result<(String, Vec<ServiceProfile>), String> {
        if self.catalog_write_blocked {
            return Err(PROFILE_CATALOG_UNAVAILABLE.to_string());
        }
        let catalog = self.catalog.lock().unwrap();
        Ok((catalog.active_profile_id.clone(), catalog.profiles.clone()))
    }

    pub fn preferences_and_catalog(
        &self,
    ) -> Result<(Preferences, String, Vec<ServiceProfile>), String> {
        if self.catalog_write_blocked {
            return Err(PROFILE_CATALOG_UNAVAILABLE.to_string());
        }
        let catalog = self.catalog.lock().unwrap();
        let mut prefs = self.prefs.lock().unwrap().clone();
        prefs.translation_mode = TranslationMode::Turbo;
        Ok((
            prefs,
            catalog.active_profile_id.clone(),
            catalog.profiles.clone(),
        ))
    }

    /// Safe fallback used only for best-effort broadcasts after the initial
    /// `settings_get` has already reported a catalog error.
    pub fn profile_catalog_or_default(&self) -> (String, Vec<ServiceProfile>) {
        let catalog = self.catalog.lock().unwrap();
        (catalog.active_profile_id.clone(), catalog.profiles.clone())
    }

    pub fn active_profile(&self) -> Result<ServiceProfile, String> {
        let (active_id, profiles) = self.profile_catalog()?;
        profiles
            .into_iter()
            .find(|profile| profile.id == active_id)
            .ok_or_else(|| PROFILE_CATALOG_UNAVAILABLE.to_string())
    }

    pub fn create_profile(
        &self,
        provider: ProviderKind,
        name: &str,
    ) -> Result<ServiceProfile, String> {
        self.mutate_catalog(|catalog| {
            if catalog.profiles.len() >= MAXIMUM_PROFILE_COUNT {
                return Err("No more service profiles can be added.".to_string());
            }
            let profile = ServiceProfile::new(
                format!("profile-{}", uuid::Uuid::new_v4().simple()),
                name,
                provider,
            )
            .map_err(|error| error.to_string())?;
            catalog.profiles.push(profile.clone());
            Ok(profile)
        })
    }

    pub fn update_profile(&self, profile_id: &str, name: &str) -> Result<ServiceProfile, String> {
        self.mutate_catalog(|catalog| {
            let current = catalog
                .profiles
                .iter_mut()
                .find(|profile| profile.id == profile_id)
                .ok_or_else(|| PROFILE_NOT_FOUND.to_string())?;
            let mut updated = current.clone();
            updated.name = name.trim().to_string();
            let updated = updated.validated().map_err(|error| error.to_string())?;
            *current = updated.clone();
            Ok(updated)
        })
    }

    pub fn select_profile(&self, profile_id: &str) -> Result<(), String> {
        if self.catalog_write_blocked {
            return Err(PROFILE_CATALOG_UNAVAILABLE.to_string());
        }

        let mut catalog = self.catalog.lock().unwrap();
        let mut next_catalog = catalog.clone();
        let profile = next_catalog
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .cloned()
            .ok_or_else(|| PROFILE_NOT_FOUND.to_string())?;
        next_catalog.active_profile_id = profile_id.to_string();
        next_catalog
            .clone()
            .validated()
            .map_err(|_| PROFILE_CATALOG_UNAVAILABLE.to_string())?;

        let mut prefs = self.prefs.lock().unwrap();
        let previous_prefs = prefs.clone();
        let mut next_prefs = previous_prefs.clone();
        normalize_preferences_value(&mut next_prefs, &profile);

        self.persist_preferences_value(&next_prefs)?;
        if let Err(error) = self.persist_catalog_value(&next_catalog) {
            if self.persist_preferences_value(&previous_prefs).is_err() {
                tracing::warn!("preferences unavailable label=profile_select_rollback_failed");
            }
            return Err(error);
        }

        *prefs = next_prefs;
        *catalog = next_catalog;
        Ok(())
    }

    pub fn delete_profile(&self, profile_id: &str) -> Result<(), String> {
        if self.catalog_write_blocked {
            return Err(PROFILE_CATALOG_UNAVAILABLE.to_string());
        }
        let mut catalog = self.catalog.lock().unwrap();
        if catalog.profiles.len() == 1 {
            return Err(LAST_PROFILE.to_string());
        }
        let profile = catalog
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .cloned()
            .ok_or_else(|| PROFILE_NOT_FOUND.to_string())?;
        // File credentials belong to the shared dev file, not this metadata
        // entry. Removing a profile must not try to modify or reveal that file.
        let (previous_secret, previous_destination) = if self.secret.is_read_only() {
            (None, None)
        } else {
            (
                self.load_api_key_for_profile(&profile)
                    .map_err(SecretStoreError::public_error)?,
                self.destination_value(&profile)?,
            )
        };

        let previous_catalog = catalog.clone();
        let mut next_catalog = previous_catalog.clone();
        next_catalog
            .profiles
            .retain(|candidate| candidate.id != profile_id);
        if next_catalog.active_profile_id == profile_id {
            next_catalog.active_profile_id = next_catalog.profiles[0].id.clone();
        }
        next_catalog
            .clone()
            .validated()
            .map_err(|_| PROFILE_CATALOG_UNAVAILABLE.to_string())?;

        let next_profile = next_catalog
            .profiles
            .iter()
            .find(|candidate| candidate.id == next_catalog.active_profile_id)
            .cloned()
            .ok_or_else(|| PROFILE_CATALOG_UNAVAILABLE.to_string())?;
        let mut prefs = self.prefs.lock().unwrap();
        let previous_prefs = prefs.clone();
        let mut next_prefs = previous_prefs.clone();
        normalize_preferences_value(&mut next_prefs, &next_profile);

        // Persist the normalized preferences before selecting their provider.
        // The old provider also accepts the current providers' normalized
        // subset, and startup revalidates both documents after a crash.
        self.persist_preferences_value(&next_prefs)?;
        let deleted = if self.secret.is_read_only() {
            Ok(())
        } else {
            self.delete_profile_credentials(&profile)
        };
        if let Err(error) = deleted {
            let secret_restored = previous_secret
                .as_deref()
                .map(|value| self.save_api_key_for_profile(&profile, value))
                .transpose()
                .is_ok();
            if self.persist_preferences_value(&previous_prefs).is_err() {
                tracing::warn!("preferences unavailable label=profile_delete_rollback_failed");
            }
            let destination_restored = previous_destination
                .as_deref()
                .map(|value| self.write_destination_value(&profile, Some(value)))
                .transpose()
                .is_ok();
            if !secret_restored || !destination_restored {
                tracing::warn!("service profile delete rollback failed label=credential_restore");
            }
            return Err(error);
        }
        if let Err(error) = self.persist_catalog_value(&next_catalog) {
            // Restore the credential when the public catalog cannot commit.
            // Deleting the secret first makes a process interruption leave a
            // visible profile with a missing key, never an unreachable secret.
            let secret_restored = previous_secret
                .as_deref()
                .map(|value| self.save_api_key_for_profile(&profile, value))
                .transpose()
                .is_ok();
            if self.persist_preferences_value(&previous_prefs).is_err() {
                tracing::warn!("preferences unavailable label=profile_delete_rollback_failed");
            }
            let destination_restored = previous_destination
                .as_deref()
                .map(|value| self.write_destination_value(&profile, Some(value)))
                .transpose()
                .is_ok();
            if !secret_restored || !destination_restored {
                tracing::warn!("service profile delete rollback failed label=credential_restore");
            }
            return Err(error);
        }

        *catalog = next_catalog;
        *prefs = next_prefs;
        Ok(())
    }

    /// Content-free diagnostic distinguishes unreadable storage from malformed data.
    pub fn credential_diagnostic(&self, profile: &ServiceProfile) -> &'static str {
        let account = credential_account(profile);
        let retry_legacy = is_default_alibaba(profile) && self.migrate_legacy_alibaba;
        self.secret_cache
            .lock()
            .unwrap()
            .retain(|(service, slot), result| {
                let selected = service == self.profile_keychain_service
                    && (slot == &account
                        || (matches!(
                            profile.provider,
                            ProviderKind::AlibabaCloud | ProviderKind::DeepLX
                        ) && slot == &Self::destination_account(profile)));
                let migration = retry_legacy
                    && ((service == self.profile_keychain_service
                        && slot == LEGACY_MIGRATION_TOMBSTONE_ACCOUNT)
                        || (slot == LEGACY_KEYCHAIN_ACCOUNT
                            && matches!(
                                service.as_str(),
                                LEGACY_KEYCHAIN_SERVICE_V3
                                    | LEGACY_KEYCHAIN_SERVICE_V2
                                    | LEGACY_KEYCHAIN_SERVICE
                            )));
                result.is_ok() || !(selected || migration)
            });
        match self.credentials_for_profile(profile) {
            Err(error) if error == "local_dev_credentials_unavailable" => "localDevUnavailable",
            Err(error) if error == CREDENTIAL_STORE_UNAVAILABLE => "unavailable",
            Err(error) if error == CREDENTIAL_SERVICE_UNAVAILABLE => "serviceUnavailable",
            Err(error) if error == CREDENTIAL_STORE_ACCESS_DENIED => "accessDenied",
            Err(_) => "invalid",
            Ok(None) => "missing",
            Ok(Some(_)) => "present",
        }
    }

    pub fn credential_state(&self, profile: &ServiceProfile) -> CredentialState {
        match self.credentials_for_profile(profile) {
            Ok(Some(_)) => CredentialState::Present,
            Ok(None) => CredentialState::Missing,
            Err(_) => CredentialState::Unavailable,
        }
    }

    /// Public source only; never exposes a file path or secret. This is not a
    /// persisted preference and cannot enable file mode from the frontend.
    pub fn credential_storage(&self) -> &'static str {
        if self.secret.is_read_only() {
            "localDevFile"
        } else {
            "keychain"
        }
    }

    /// Explicit settings-window action only. Uses the existing profile-scoped
    /// OS store/cache path and never changes preferences or emits a snapshot.
    pub fn reveal_credential(
        &self,
        profile_id: &str,
        field: CredentialRevealField,
        text_translation: Option<TextTranslation>,
    ) -> Result<Option<String>, String> {
        self.require_writable_credentials()?;
        let profile = self.profile(profile_id)?;
        if !field.allowed_for(&profile, text_translation) {
            return Err("credential_reveal_field_mismatch".into());
        }
        // Read only the selected field's existing OS-store slot. In particular,
        // an unavailable MT destination must not prevent revealing the ASR key.
        if field == CredentialRevealField::Token {
            if let Some(value) = self.destination_value(&profile)? {
                let destination: TextTranslationDestination = serde_json::from_str(&value)
                    .map_err(|_| "credential_store_unavailable".to_string())?;
                let credentials = match profile.text_translation() {
                    TextTranslation::DeepL => ProviderCredentials::DeepL {
                        asr_api_key: String::new(),
                        api_key: destination.deep_l_api_key.unwrap_or_default(),
                    },
                    TextTranslation::DeepLX => ProviderCredentials::DeepLX {
                        asr_api_key: String::new(),
                        endpoint: destination.endpoint,
                        token: destination.token,
                    },
                    TextTranslation::OpenAICompatible => {
                        let destination = destination.open_ai_compatible.unwrap_or_default();
                        ProviderCredentials::OpenAICompatible {
                            asr_api_key: String::new(),
                            endpoint: destination.endpoint,
                            api_key: destination.api_key,
                            model: destination.model,
                        }
                    }
                    TextTranslation::FollowService => unreachable!(),
                };
                return credentials
                    .revealed_field(&profile, field, text_translation)
                    .map(|value| value.map(str::to_owned))
                    .map_err(|_| "credential_store_unavailable".to_string());
            }
            if profile.provider != ProviderKind::DeepLX {
                return Ok(None);
            }
        }
        let Some(value) = self
            .load_api_key_for_profile(&profile)
            .map_err(SecretStoreError::public_error)?
        else {
            return Ok(None);
        };
        let credentials = ProviderCredentials::decode_for_profile(&profile, &value)
            .map_err(|_| "credential_store_unavailable".to_string())?;
        credentials
            .revealed_field(&profile, field, text_translation)
            .map(|value| value.map(str::to_owned))
            .map_err(|_| "credential_reveal_field_mismatch".into())
    }

    #[cfg(test)]
    fn load_api_key(&self) -> Result<Option<String>, String> {
        let profile = self.active_profile()?;
        self.load_api_key_for_profile(&profile)
            .map_err(SecretStoreError::public_error)
    }

    #[cfg(test)]
    pub fn save_api_key(&self, profile_id: &str, api_key: &str) -> Result<(), String> {
        self.save_credentials(profile_id, &ProviderCredentials::api_key(api_key))
    }

    pub fn save_credentials(
        &self,
        profile_id: &str,
        credentials: &ProviderCredentials,
    ) -> Result<(), String> {
        self.require_writable_credentials()?;
        let profile = self.profile(profile_id)?;
        if let ProviderCredentials::AlibabaTranslation {
            api_key,
            text_translation,
            endpoint,
            token,
            model,
        } = credentials
        {
            return self.save_text_translation(
                &profile,
                api_key,
                *text_translation,
                endpoint,
                token,
                model,
            );
        }
        if let ProviderCredentials::OpenAICompatible {
            asr_api_key,
            endpoint,
            api_key,
            model,
        } = credentials
        {
            return self.save_text_translation(
                &profile,
                asr_api_key,
                TextTranslation::OpenAICompatible,
                endpoint,
                api_key,
                model,
            );
        }
        let value = credentials
            .encode_for_keychain(profile.provider)
            .map_err(|error| error.to_string())?;
        self.save_api_key_for_profile(&profile, &value)
    }

    fn destination_account(profile: &ServiceProfile) -> String {
        format!(
            "provider-profile:{}:{}:text-translation",
            profile.id,
            profile.provider.wire_value()
        )
    }

    fn destination_value(&self, profile: &ServiceProfile) -> Result<Option<String>, String> {
        if !matches!(
            profile.provider,
            ProviderKind::AlibabaCloud | ProviderKind::DeepLX
        ) {
            return Ok(None);
        }
        self.load_secret(
            self.profile_keychain_service,
            &Self::destination_account(profile),
        )
        .map_err(SecretStoreError::public_error)
    }

    fn write_destination_value(
        &self,
        profile: &ServiceProfile,
        value: Option<&str>,
    ) -> Result<(), String> {
        let account = Self::destination_account(profile);
        match value {
            Some(value) => {
                self.save_secret(self.profile_keychain_service, &account, value)
                    .map_err(SecretStoreError::public_error)?;
                let verified = self
                    .load_secret_uncached(self.profile_keychain_service, &account)
                    .map_err(SecretStoreError::public_error)?;
                if verified.as_deref() != Some(value) {
                    self.secret_cache
                        .lock()
                        .unwrap()
                        .remove(&cache_key(self.profile_keychain_service, &account));
                    return Err(CREDENTIAL_STORE_UNAVAILABLE.to_string());
                }
                self.cache_secret(self.profile_keychain_service, &account, verified);
                Ok(())
            }
            None => self
                .delete_secret(self.profile_keychain_service, &account)
                .map_err(SecretStoreError::public_error),
        }
    }

    fn credentials_for_profile(
        &self,
        profile: &ServiceProfile,
    ) -> Result<Option<ProviderCredentials>, String> {
        if self.secret.is_read_only()
            && profile.text_translation() != TextTranslation::FollowService
        {
            // The local file supplies an Alibaba key only, not an independent
            // MT key/endpoint. Never borrow an OS-store destination instead.
            return Ok(None);
        }
        let Some(value) = self
            .load_api_key_for_profile(profile)
            .map_err(SecretStoreError::public_error)?
        else {
            return Ok(None);
        };
        let credentials = ProviderCredentials::decode_for_profile(profile, &value)
            .map_err(|error| error.to_string())?;
        if matches!(
            profile.provider,
            ProviderKind::AlibabaCloud | ProviderKind::DeepLX
        ) && profile.text_translation() != TextTranslation::FollowService
        {
            let Some(destination) = self.destination_value(profile)? else {
                if profile.provider == ProviderKind::DeepLX
                    && matches!(
                        (profile.text_translation(), &credentials),
                        (TextTranslation::DeepLX, ProviderCredentials::DeepLX { .. })
                            | (TextTranslation::DeepL, ProviderCredentials::DeepL { .. })
                    )
                {
                    return Ok(Some(credentials));
                }
                return Err(
                    crate::core::credentials::ProviderCredentialsError::InvalidStoredValue
                        .to_string(),
                );
            };
            let destination: TextTranslationDestination = serde_json::from_str(&destination)
                .map_err(|_| {
                    crate::core::credentials::ProviderCredentialsError::InvalidStoredValue
                        .to_string()
                })?;
            let asr_api_key = credentials.alibaba_key().unwrap_or_default().into();
            let credentials = match profile.text_translation() {
                TextTranslation::DeepL => ProviderCredentials::DeepL {
                    asr_api_key,
                    api_key: destination.deep_l_api_key.unwrap_or_default(),
                },
                TextTranslation::DeepLX => ProviderCredentials::DeepLX {
                    asr_api_key,
                    endpoint: destination.endpoint,
                    token: destination.token,
                },
                TextTranslation::OpenAICompatible => {
                    let destination = destination.open_ai_compatible.unwrap_or_default();
                    ProviderCredentials::OpenAICompatible {
                        asr_api_key,
                        endpoint: destination.endpoint,
                        api_key: destination.api_key,
                        model: destination.model,
                    }
                }
                TextTranslation::FollowService => unreachable!(),
            };
            return credentials
                .validated_for(profile.effective_provider())
                .map(Some)
                .map_err(|error| error.to_string());
        }
        Ok(Some(credentials))
    }

    fn save_text_translation(
        &self,
        profile: &ServiceProfile,
        api_key: &str,
        translation: TextTranslation,
        endpoint: &str,
        token: &str,
        model: &str,
    ) -> Result<(), String> {
        if !matches!(
            profile.provider,
            ProviderKind::AlibabaCloud | ProviderKind::DeepLX
        ) {
            return Err("Independent text translation requires Alibaba speech recognition.".into());
        }
        if translation == TextTranslation::DeepLX && !endpoint.trim().is_empty() {
            crate::core::protocols::deeplx::endpoint(endpoint).map_err(|_| {
                crate::core::credentials::ProviderCredentialsError::InvalidDeepLXEndpoint
                    .to_string()
            })?;
        }
        if translation == TextTranslation::OpenAICompatible {
            if !endpoint.trim().is_empty() {
                crate::core::protocols::openai_compatible::endpoint(endpoint).map_err(|_| {
                    crate::core::credentials::ProviderCredentialsError::InvalidOpenAICompatibleEndpoint
                        .to_string()
                })?;
            }
            if !model.trim().is_empty() {
                crate::core::protocols::openai_compatible::validate_model(model).map_err(|_| {
                    crate::core::credentials::ProviderCredentialsError::InvalidOpenAICompatibleModel
                        .to_string()
                })?;
            }
        }
        let previous = self
            .load_api_key_for_profile(profile)
            .map_err(SecretStoreError::public_error)?;
        let previous_credentials = previous
            .as_deref()
            .map(|value| ProviderCredentials::decode_for_profile(profile, value))
            .transpose()
            .map_err(|error| error.to_string())?;
        let previous_destination = self.destination_value(profile)?;
        let retained = previous_destination
            .as_deref()
            .map(serde_json::from_str::<TextTranslationDestination>)
            .transpose()
            .map_err(|_| {
                crate::core::credentials::ProviderCredentialsError::InvalidStoredValue.to_string()
            })?
            .or_else(|| {
                previous_credentials
                    .as_ref()
                    .and_then(|credentials| match credentials {
                        ProviderCredentials::DeepLX {
                            endpoint, token, ..
                        } => Some(TextTranslationDestination {
                            endpoint: endpoint.clone(),
                            token: token.clone(),
                            deep_l_api_key: None,
                            open_ai_compatible: None,
                        }),
                        ProviderCredentials::DeepL { api_key, .. } => {
                            Some(TextTranslationDestination {
                                endpoint: String::new(),
                                token: String::new(),
                                deep_l_api_key: Some(api_key.clone()),
                                open_ai_compatible: None,
                            })
                        }
                        _ => None,
                    })
            });
        let key = if api_key.trim().is_empty() {
            previous_credentials
                .as_ref()
                .and_then(ProviderCredentials::alibaba_key)
                .unwrap_or_default()
        } else {
            api_key
        };
        let key = ProviderCredentials::api_key(key)
            .encode_for_keychain(ProviderKind::AlibabaCloud)
            .map_err(|error| error.to_string())?;
        let mut destination = retained;
        match translation {
            TextTranslation::DeepL => {
                let value = destination.get_or_insert_with(TextTranslationDestination::default);
                if !token.trim().is_empty() {
                    value.deep_l_api_key = Some(token.trim().into());
                }
                let checked = ProviderCredentials::DeepL {
                    asr_api_key: key.clone(),
                    api_key: value.deep_l_api_key.clone().unwrap_or_default(),
                }
                .validated_for(ProviderKind::AlibabaCloud)
                .map_err(|error| error.to_string())?;
                let ProviderCredentials::DeepL { api_key, .. } = checked else {
                    unreachable!()
                };
                value.deep_l_api_key = Some(api_key);
            }
            TextTranslation::DeepLX => {
                let value = destination.get_or_insert_with(TextTranslationDestination::default);
                if !endpoint.trim().is_empty() {
                    value.endpoint = endpoint.into();
                    value.token = token.into();
                } else if !token.trim().is_empty() {
                    value.token = token.into();
                }
                let checked = ProviderCredentials::DeepLX {
                    asr_api_key: key.clone(),
                    endpoint: value.endpoint.clone(),
                    token: value.token.clone(),
                }
                .validated_for(ProviderKind::DeepLX)
                .map_err(|error| error.to_string())?;
                let ProviderCredentials::DeepLX {
                    endpoint, token, ..
                } = checked
                else {
                    unreachable!()
                };
                value.endpoint = endpoint;
                value.token = token;
            }
            TextTranslation::OpenAICompatible => {
                let value = destination
                    .get_or_insert_with(TextTranslationDestination::default)
                    .open_ai_compatible
                    .get_or_insert_with(OpenAICompatibleDestination::default);
                if !endpoint.trim().is_empty() {
                    let endpoint = crate::core::protocols::openai_compatible::endpoint(endpoint)
                        .map_err(|_| {
                            crate::core::credentials::ProviderCredentialsError::InvalidOpenAICompatibleEndpoint
                                .to_string()
                        })?
                        .to_string();
                    // Never send a retained key to a newly entered service.
                    if endpoint != value.endpoint {
                        value.api_key = token.into();
                    }
                    value.endpoint = endpoint;
                }
                if !token.trim().is_empty() {
                    value.api_key = token.into();
                }
                if !model.trim().is_empty() {
                    value.model = model.into();
                }
                let checked = ProviderCredentials::OpenAICompatible {
                    asr_api_key: key.clone(),
                    endpoint: value.endpoint.clone(),
                    api_key: value.api_key.clone(),
                    model: value.model.clone(),
                }
                .validated_for(ProviderKind::AlibabaCloud)
                .map_err(|error| error.to_string())?;
                let ProviderCredentials::OpenAICompatible {
                    endpoint,
                    api_key,
                    model,
                    ..
                } = checked
                else {
                    unreachable!()
                };
                value.endpoint = endpoint;
                value.api_key = api_key;
                value.model = model;
            }
            TextTranslation::FollowService => {}
        }
        let destination_value = destination
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|_| CREDENTIAL_STORE_UNAVAILABLE.to_string())?;
        // Keep the legacy ASR account and representation. Independent text
        // destinations belong to the separate profile-scoped destination item.
        let key_value = if profile.provider == ProviderKind::DeepLX
            && api_key.trim().is_empty()
            && previous.is_some()
        {
            previous.clone().unwrap()
        } else if profile.provider == ProviderKind::DeepLX {
            match previous_credentials.as_ref() {
                Some(ProviderCredentials::DeepLX {
                    endpoint, token, ..
                }) => ProviderCredentials::DeepLX {
                    asr_api_key: key,
                    endpoint: endpoint.clone(),
                    token: token.clone(),
                }
                .encode_for_keychain(ProviderKind::DeepLX),
                Some(ProviderCredentials::DeepL { api_key, .. }) => ProviderCredentials::DeepL {
                    asr_api_key: key,
                    api_key: api_key.clone(),
                }
                .encode_for_keychain(ProviderKind::AlibabaCloud),
                _ => Ok(key),
            }
            .map_err(|error| error.to_string())?
        } else {
            key
        };
        if self.catalog_write_blocked {
            return Err(PROFILE_CATALOG_UNAVAILABLE.into());
        }
        let mut catalog = self.catalog.lock().unwrap();
        let mut next = catalog.clone();
        let updated = next
            .profiles
            .iter_mut()
            .find(|candidate| candidate.id == profile.id)
            .ok_or_else(|| PROFILE_NOT_FOUND.to_string())?;
        updated.text_translation = Some(translation);
        let mut prefs = self.prefs.lock().unwrap();
        let previous_prefs = prefs.clone();
        let mut next_prefs = previous_prefs.clone();
        if next.active_profile_id == profile.id {
            normalize_preferences_value(&mut next_prefs, updated);
        }
        let prefs_changed = next_prefs != previous_prefs;
        let destination_changed = destination_value != previous_destination;
        let key_changed = previous.as_deref() != Some(key_value.as_str());
        let write_result = (|| {
            if prefs_changed {
                self.persist_preferences_value(&next_prefs)?;
            }
            if destination_changed {
                self.write_destination_value(profile, destination_value.as_deref())?;
            }
            if key_changed {
                self.save_api_key_for_profile(profile, &key_value)?;
            }
            self.persist_catalog_value(&next)
        })();
        if let Err(error) = write_result {
            if prefs_changed && self.persist_preferences_value(&previous_prefs).is_err() {
                tracing::warn!("preferences unavailable label=text_route_rollback_failed");
            }
            // Restore in place, never delete/recreate an existing key or widen its ACL.
            if destination_changed
                && self
                    .write_destination_value(profile, previous_destination.as_deref())
                    .is_err()
            {
                tracing::warn!("credential update rollback failed label=text_destination");
            }
            if key_changed {
                let rollback = match previous {
                    Some(previous) => self.save_api_key_for_profile(profile, &previous),
                    None => self.delete_api_key_for_profile(profile),
                };
                if rollback.is_err() {
                    tracing::warn!("credential update rollback failed label=text_key");
                }
            }
            return Err(error);
        }
        *prefs = next_prefs;
        *catalog = next;
        Ok(())
    }

    pub fn delete_api_key(&self, profile_id: &str) -> Result<(), String> {
        let profile = self.profile(profile_id)?;
        self.delete_profile_credentials(&profile)
    }

    fn delete_profile_credentials(&self, profile: &ServiceProfile) -> Result<(), String> {
        self.require_writable_credentials()?;
        let destination = self.destination_value(profile)?;
        if destination.is_some() {
            self.write_destination_value(profile, None)?;
        }
        if let Err(error) = self.delete_api_key_for_profile(profile) {
            if let Some(destination) = destination {
                if self
                    .write_destination_value(profile, Some(&destination))
                    .is_err()
                {
                    tracing::warn!("credential delete rollback failed label=text_destination");
                }
            }
            return Err(error);
        }
        Ok(())
    }

    /// The validated configuration used to start a session. The provider is
    /// resolved natively from the active profile; credentials never cross IPC.
    pub fn configuration(&self) -> Result<LiveTranslationConfiguration, String> {
        let profile = self.active_profile()?;
        self.configuration_for_profile(&profile)
    }

    /// Reads only the requested profile's Keychain credentials. A connection
    /// check must not select it or change the current listening session.
    pub fn configuration_for_profile(
        &self,
        profile: &ServiceProfile,
    ) -> Result<LiveTranslationConfiguration, String> {
        self.configuration_for_profile_options(profile, false)
    }

    /// The selected service is checked using compatible language options,
    /// without changing the active profile or persisted listening preferences.
    pub fn configuration_for_profile_probe(
        &self,
        profile: &ServiceProfile,
    ) -> Result<LiveTranslationConfiguration, String> {
        self.configuration_for_profile_options(profile, true)
    }

    fn configuration_for_profile_options(
        &self,
        profile: &ServiceProfile,
        for_probe: bool,
    ) -> Result<LiveTranslationConfiguration, String> {
        let mut prefs = self.prefs.lock().unwrap().clone();
        if for_probe {
            let normalized = profile.normalize_preferences(ProviderPreferences {
                source_language: prefs.source_language,
                target_language: prefs.target_language,
                translation_mode: prefs.translation_mode,
            });
            prefs.source_language = normalized.source_language;
            prefs.target_language = normalized.target_language;
            prefs.translation_mode = normalized.translation_mode;
        }
        let credentials = self.credentials_for_profile(profile)?.ok_or_else(|| {
            crate::core::credentials::ProviderCredentialsError::Missing(
                profile.effective_provider(),
            )
            .to_string()
        })?;
        let provider = profile.effective_provider();
        let credentials = if provider == ProviderKind::AlibabaCloud
            && !matches!(
                profile.text_translation(),
                TextTranslation::DeepL | TextTranslation::OpenAICompatible
            ) {
            ProviderCredentials::api_key(credentials.alibaba_key().unwrap_or_default())
        } else {
            credentials
        };
        LiveTranslationConfiguration::with_credentials(
            provider,
            credentials,
            prefs.source_language,
            prefs.target_language,
            prefs.translation_mode,
        )
        .with_network_proxy(prefs.network_proxy)
        .validated()
        .map_err(|error| error.to_string())
    }

    /// Applies listening-time constraints without changing profile metadata.
    pub fn prepare_for_listening(&self) -> Result<(), String> {
        let profile = self.active_profile()?;
        self.save_preferences(|prefs| normalize_preferences_value(prefs, &profile))
    }

    fn persist_preferences_value(&self, prefs: &Preferences) -> Result<(), String> {
        if (self.is_ui_test && !self.ui_test_preferences_writable)
            || self.prefs_path.as_os_str().is_empty()
        {
            return Ok(());
        }
        let bytes =
            serde_json::to_vec_pretty(prefs).map_err(|_| PREFERENCES_UNAVAILABLE.to_string())?;
        atomic_write(&self.prefs_path, &bytes).map_err(|_| PREFERENCES_UNAVAILABLE.to_string())
    }

    fn profile(&self, profile_id: &str) -> Result<ServiceProfile, String> {
        let (_, profiles) = self.profile_catalog()?;
        profiles
            .into_iter()
            .find(|profile| profile.id == profile_id)
            .ok_or_else(|| PROFILE_NOT_FOUND.to_string())
    }

    fn mutate_catalog<T>(
        &self,
        update: impl FnOnce(&mut ProfileCatalog) -> Result<T, String>,
    ) -> Result<T, String> {
        if self.catalog_write_blocked {
            return Err(PROFILE_CATALOG_UNAVAILABLE.to_string());
        }
        let mut catalog = self.catalog.lock().unwrap();
        let mut next = catalog.clone();
        let result = update(&mut next)?;
        next.clone()
            .validated()
            .map_err(|_| PROFILE_CATALOG_UNAVAILABLE.to_string())?;
        self.persist_catalog_value(&next)?;
        *catalog = next;
        Ok(result)
    }

    fn persist_catalog(&self) -> Result<(), String> {
        let catalog = self.catalog.lock().unwrap().clone();
        self.persist_catalog_value(&catalog)
    }

    fn persist_catalog_value(&self, catalog: &ProfileCatalog) -> Result<(), String> {
        if self.catalog_write_blocked {
            return Err(PROFILE_CATALOG_UNAVAILABLE.to_string());
        }
        if self.is_ui_test || self.catalog_path.as_os_str().is_empty() {
            return Ok(());
        }
        let bytes = serde_json::to_vec_pretty(catalog)
            .map_err(|_| PROFILE_CATALOG_UNAVAILABLE.to_string())?;
        atomic_write(&self.catalog_path, &bytes)
            .map_err(|_| PROFILE_CATALOG_UNAVAILABLE.to_string())
    }

    fn load_api_key_for_profile(
        &self,
        profile: &ServiceProfile,
    ) -> Result<Option<String>, SecretStoreError> {
        let account = credential_account(profile);
        let destination = self.load_secret(self.profile_keychain_service, &account)?;
        if let Some(value) = destination {
            if value.trim().is_empty() {
                return Err(SecretStoreError::Unavailable);
            }
            // A profile-scoped credential is already authoritative. Do not
            // touch the separate migration marker on the normal read path:
            // macOS authorizes Keychain items independently, so reading both
            // accounts can produce two password prompts after an intentional
            // signing-identity migration. Save, delete, and actual legacy
            // migration still persist the marker before it matters.
            return Ok(Some(value));
        }
        if !is_default_alibaba(profile) || !self.migrate_legacy_alibaba {
            return Ok(None);
        }

        match self.load_secret(
            self.profile_keychain_service,
            LEGACY_MIGRATION_TOMBSTONE_ACCOUNT,
        )? {
            Some(value) if value == LEGACY_MIGRATION_TOMBSTONE_VALUE => return Ok(None),
            Some(_) => return Err(SecretStoreError::Unavailable),
            None => {}
        }

        let mut saw_blank_legacy_value = false;
        for service in [
            LEGACY_KEYCHAIN_SERVICE_V3,
            LEGACY_KEYCHAIN_SERVICE_V2,
            LEGACY_KEYCHAIN_SERVICE,
        ] {
            let Some(legacy) = self.load_secret(service, LEGACY_KEYCHAIN_ACCOUNT)? else {
                continue;
            };
            if legacy.trim().is_empty() {
                saw_blank_legacy_value = true;
                continue;
            }

            self.save_secret(self.profile_keychain_service, &account, &legacy)?;
            let verified = self.load_secret_uncached(self.profile_keychain_service, &account)?;
            if verified.as_deref() != Some(legacy.as_str()) {
                self.secret_cache
                    .lock()
                    .unwrap()
                    .remove(&cache_key(self.profile_keychain_service, &account));
                return Err(SecretStoreError::Unavailable);
            }
            self.cache_secret(self.profile_keychain_service, &account, verified);
            if self.write_legacy_tombstone().is_err() {
                tracing::warn!("credential migration deferred label=tombstone_write_failed");
            }
            return Ok(Some(legacy));
        }

        if saw_blank_legacy_value {
            return Err(SecretStoreError::Unavailable);
        }

        // An empty scan is not a migration. Leave it retryable so a credential
        // created by an older installed version can still be imported later.
        Ok(None)
    }

    fn delete_api_key_for_profile(&self, profile: &ServiceProfile) -> Result<(), String> {
        if is_default_alibaba(profile) && self.migrate_legacy_alibaba {
            // The tombstone must be durable before deletion so a missing new
            // slot can never revive an older single-slot Alibaba credential.
            self.write_legacy_tombstone()
                .map_err(SecretStoreError::public_error)?;
            // Explicit removal is also the point where rollback-era single
            // slots are retired. Keep the current profile credential intact
            // unless every legacy deletion succeeds.
            self.delete_legacy_alibaba_credentials()
                .map_err(SecretStoreError::public_error)?;
        }
        let account = credential_account(profile);
        self.delete_secret(self.profile_keychain_service, &account)
            .map_err(SecretStoreError::public_error)
    }

    fn delete_legacy_alibaba_credentials(&self) -> Result<(), SecretStoreError> {
        for service in [
            LEGACY_KEYCHAIN_SERVICE_V3,
            LEGACY_KEYCHAIN_SERVICE_V2,
            LEGACY_KEYCHAIN_SERVICE,
        ] {
            self.delete_secret(service, LEGACY_KEYCHAIN_ACCOUNT)?;
        }
        Ok(())
    }

    fn save_api_key_for_profile(
        &self,
        profile: &ServiceProfile,
        value: &str,
    ) -> Result<(), String> {
        let account = credential_account(profile);
        self.save_secret(self.profile_keychain_service, &account, value)
            .map_err(SecretStoreError::public_error)?;
        let verified = self
            .load_secret_uncached(self.profile_keychain_service, &account)
            .map_err(SecretStoreError::public_error)?;
        if verified.as_deref() != Some(value) {
            self.secret_cache
                .lock()
                .unwrap()
                .remove(&cache_key(self.profile_keychain_service, &account));
            return Err(CREDENTIAL_STORE_UNAVAILABLE.to_string());
        }
        self.cache_secret(self.profile_keychain_service, &account, verified);

        if is_default_alibaba(profile)
            && self.migrate_legacy_alibaba
            && self.write_legacy_tombstone().is_err()
        {
            // Saving and verifying the profile slot is the user-visible
            // transaction. A migration marker failure is retryable and must
            // not turn a successful key replacement into an error.
            tracing::warn!("credential migration deferred label=tombstone_write_failed");
        }
        Ok(())
    }

    fn write_legacy_tombstone(&self) -> Result<(), SecretStoreError> {
        match self.load_secret(
            self.profile_keychain_service,
            LEGACY_MIGRATION_TOMBSTONE_ACCOUNT,
        )? {
            Some(value) if value == LEGACY_MIGRATION_TOMBSTONE_VALUE => return Ok(()),
            Some(_) => return Err(SecretStoreError::Unavailable),
            None => {}
        }
        self.save_secret(
            self.profile_keychain_service,
            LEGACY_MIGRATION_TOMBSTONE_ACCOUNT,
            LEGACY_MIGRATION_TOMBSTONE_VALUE,
        )?;
        let verified = self.load_secret_uncached(
            self.profile_keychain_service,
            LEGACY_MIGRATION_TOMBSTONE_ACCOUNT,
        )?;
        if verified.as_deref() != Some(LEGACY_MIGRATION_TOMBSTONE_VALUE) {
            self.secret_cache.lock().unwrap().remove(&cache_key(
                self.profile_keychain_service,
                LEGACY_MIGRATION_TOMBSTONE_ACCOUNT,
            ));
            return Err(SecretStoreError::Unavailable);
        }
        self.cache_secret(
            self.profile_keychain_service,
            LEGACY_MIGRATION_TOMBSTONE_ACCOUNT,
            verified,
        );
        Ok(())
    }

    fn load_secret(
        &self,
        service: &str,
        account: &str,
    ) -> Result<Option<String>, SecretStoreError> {
        let key = cache_key(service, account);
        let mut cache = self.secret_cache.lock().unwrap();
        if let Some(value) = cache.get(&key) {
            return value.clone();
        }
        if self.is_ui_test {
            let value = if service == self.profile_keychain_service
                && account == credential_account(&ServiceProfile::alibaba_default())
            {
                Some("sk-demo-not-a-real-key".to_string())
            } else {
                None
            };
            cache.insert(key, Ok(value.clone()));
            return Ok(value);
        }
        let result = self.secret.load(service, account);
        cache.insert(key, result.clone());
        result
    }

    fn load_secret_uncached(
        &self,
        service: &str,
        account: &str,
    ) -> Result<Option<String>, SecretStoreError> {
        if self.is_ui_test {
            return self
                .secret_cache
                .lock()
                .unwrap()
                .get(&cache_key(service, account))
                .cloned()
                .unwrap_or(Ok(None));
        }
        let result = self.secret.load(service, account);
        self.secret_cache
            .lock()
            .unwrap()
            .insert(cache_key(service, account), result.clone());
        result
    }

    fn save_secret(
        &self,
        service: &str,
        account: &str,
        value: &str,
    ) -> Result<(), SecretStoreError> {
        if !self.is_ui_test {
            if let Err(error) = self.secret.save(service, account, value) {
                self.secret_cache
                    .lock()
                    .unwrap()
                    .insert(cache_key(service, account), Err(error));
                return Err(error);
            }
        }
        self.cache_secret(service, account, Some(value.to_string()));
        Ok(())
    }

    fn delete_secret(&self, service: &str, account: &str) -> Result<(), SecretStoreError> {
        if !self.is_ui_test {
            if let Err(error) = self.secret.delete(service, account) {
                self.secret_cache
                    .lock()
                    .unwrap()
                    .insert(cache_key(service, account), Err(error));
                return Err(error);
            }
        }
        self.cache_secret(service, account, None);
        Ok(())
    }

    fn cache_secret(&self, service: &str, account: &str, value: Option<String>) {
        self.secret_cache
            .lock()
            .unwrap()
            .insert(cache_key(service, account), Ok(value));
    }

    fn require_writable_credentials(&self) -> Result<(), String> {
        if self.secret.is_read_only() {
            return Err(SecretStoreError::ReadOnly.public_error());
        }
        Ok(())
    }
}

fn credential_account(profile: &ServiceProfile) -> String {
    format!(
        "provider-profile:{}:{}:api-key",
        profile.id,
        profile.provider.wire_value()
    )
}

fn is_default_alibaba(profile: &ServiceProfile) -> bool {
    profile.id == DEFAULT_ALIBABA_PROFILE_ID && profile.provider == ProviderKind::AlibabaCloud
}

fn normalize_preferences_value(prefs: &mut Preferences, profile: &ServiceProfile) {
    let normalized = profile.normalize_preferences(ProviderPreferences {
        source_language: prefs.source_language,
        target_language: prefs.target_language,
        translation_mode: prefs.translation_mode,
    });
    prefs.source_language = normalized.source_language;
    prefs.target_language = normalized.target_language;
    prefs.translation_mode = normalized.translation_mode;
}

fn cache_key(service: &str, account: &str) -> SecretCacheKey {
    (service.to_string(), account.to_string())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(ErrorKind::InvalidInput, "catalog path has no parent")
    })?;
    std::fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(PROFILE_CATALOG_FILE);
    let temporary = parent.join(format!(
        ".{file_name}.{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    let write_result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        replace_file(&temporary, path)?;
        sync_directory(parent);
        Ok(())
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    write_result
}

#[cfg(unix)]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::rename(source, destination)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::iter::once;
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
    }

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(once(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(once(0))
        .collect();
    // SAFETY: both arguments are valid, NUL-terminated UTF-16 buffers for the
    // duration of the call. The flags request same-volume atomic replacement.
    let replaced = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if replaced == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(any(unix, windows)))]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::rename(source, destination)
}

#[cfg(unix)]
fn sync_directory(path: &Path) {
    if let Ok(directory) = File::open(path) {
        let _ = directory.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) {}

#[cfg(test)]
mod animation_switch_tests {
    use super::Preferences;

    /// An explicit "off" is a choice, not an absent value: it must survive a
    /// round trip even though the field defaults to `None` (follow the system).
    #[test]
    fn animation_switches_round_trip_explicit_values() {
        let mut prefs = Preferences::default();
        assert_eq!(prefs.pulse_animation, None);
        assert_eq!(prefs.subtitle_animation, None);

        prefs.pulse_animation = Some(false);
        prefs.subtitle_animation = Some(true);
        let json = serde_json::to_string(&prefs).unwrap();
        let restored: Preferences = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.pulse_animation, Some(false));
        assert_eq!(restored.subtitle_animation, Some(true));
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires an isolated D-Bus session and unlocked test Secret Service"]
    fn linux_secret_service_roundtrip() {
        assert_eq!(
            std::env::var("MIMI_TEST_SECRET_SERVICE").as_deref(),
            Ok("1")
        );
        let account = format!("linux-ci-{}", uuid::Uuid::new_v4());
        let service = "app.yuxino.mimi.test.secret-service";
        let store = KeyringSecretStore;
        assert_eq!(store.load(service, &account).unwrap(), None);
        store
            .save(service, &account, "non-secret-test-value")
            .unwrap();
        assert!(store.load(service, &account).unwrap().as_deref() == Some("non-secret-test-value"));
        store.save(service, &account, "updated-test-value").unwrap();
        assert!(store.load(service, &account).unwrap().as_deref() == Some("updated-test-value"));
        store.delete(service, &account).unwrap();
        assert_eq!(store.load(service, &account).unwrap(), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires a private non-activating D-Bus session; starts synthetic Secret Service"]
    fn linux_secret_service_recovers_without_restart() {
        use std::io::Write;
        use std::process::{Command, Stdio};
        assert_eq!(
            std::env::var("MIMI_TEST_SECRET_SERVICE_RECOVERY").as_deref(),
            Ok("1")
        );
        let directory = std::env::var("MIMI_TEST_PRIVATE_KEYRING_DIRECTORY").unwrap();
        assert!(std::env::var("DBUS_SESSION_BUS_ADDRESS")
            .unwrap()
            .contains(&directory));
        // Reproduce the dependency's permanently cached first failure too.
        assert!(keyring::Entry::store_status().is_err());
        let store = SettingsStore::in_memory_with_scope(
            Box::new(KeyringSecretStore),
            false,
            "app.yuxino.mimi.test.secret-service-recovery",
            false,
        );
        let profile = store.active_profile().unwrap();
        assert_eq!(store.credential_diagnostic(&profile), "serviceUnavailable");
        assert_eq!(
            store.credential_state(&profile),
            CredentialState::Unavailable
        );
        assert_eq!(
            store.save_api_key(&profile.id, "synthetic-recovery-value"),
            Err(CREDENTIAL_SERVICE_UNAVAILABLE.to_string())
        );

        struct Daemon(std::process::Child);
        impl Drop for Daemon {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let mut daemon = Daemon(
            Command::new("gnome-keyring-daemon")
                .args([
                    "--foreground",
                    "--unlock",
                    "--components=secrets",
                    "--control-directory",
                ])
                .arg(Path::new(&directory).join("control"))
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        daemon
            .0
            .stdin
            .take()
            .unwrap()
            .write_all(b"mimi-ci-synthetic-recovery")
            .unwrap();
        let mut unlocked = false;
        for _ in 0..40 {
            let result = Command::new("gdbus")
                .args([
                    "call",
                    "--session",
                    "--dest",
                    "org.freedesktop.secrets",
                    "--object-path",
                    "/org/freedesktop/secrets/collection/login",
                    "--method",
                    "org.freedesktop.DBus.Properties.Get",
                    "org.freedesktop.Secret.Collection",
                    "Locked",
                ])
                .output()
                .unwrap();
            if result.status.success() && String::from_utf8_lossy(&result.stdout).contains("false")
            {
                unlocked = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        assert!(unlocked, "synthetic Secret Service did not unlock");
        assert!(keyring::Entry::store_status().is_err());
        assert_eq!(store.credential_diagnostic(&profile), "missing");
        assert_eq!(store.credential_state(&profile), CredentialState::Missing);
        store
            .save_api_key(&profile.id, "synthetic-recovery-value")
            .unwrap();
        assert_eq!(store.credential_diagnostic(&profile), "present");
        assert!(
            KeyringSecretStore
                .load(
                    store.profile_keychain_service,
                    &credential_account(&profile)
                )
                .unwrap()
                .as_deref()
                == Some("synthetic-recovery-value")
        );
        store.delete_api_key(&profile.id).unwrap();
        assert_eq!(store.credential_diagnostic(&profile), "missing");
    }

    #[test]
    fn dock_visibility_keeps_legacy_default_and_persists_without_credentials() {
        let legacy: Preferences = serde_json::from_str(r#"{"ui_language":"ja"}"#).unwrap();
        assert!(!legacy.show_in_dock);
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        store
            .save_preferences_for_active_profile(|prefs| prefs.show_in_dock = true)
            .unwrap();
        let profile = store
            .create_profile(ProviderKind::OpenAIRealtime, "Synthetic")
            .unwrap();
        store.select_profile(&profile.id).unwrap();
        let reloaded = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        assert!(reloaded.preferences().show_in_dock);
        reloaded
            .save_preferences(|prefs| prefs.show_in_dock = false)
            .unwrap();
        let final_store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        assert!(!final_store.preferences().show_in_dock);
        assert!(fake.state.lock().unwrap().loads.is_empty());
    }

    #[test]
    fn windows_source_defaults_to_system_and_preserves_stable_id() {
        let legacy: super::Preferences = serde_json::from_str("{}").unwrap();
        assert!(legacy.windows_audio_source.is_empty());
        let preferences = super::Preferences {
            windows_audio_source: "wasapi:stable-render-id".into(),
            ..Default::default()
        };
        let restored: super::Preferences =
            serde_json::from_str(&serde_json::to_string(&preferences).unwrap()).unwrap();
        assert_eq!(
            restored.windows_audio_source,
            preferences.windows_audio_source
        );
    }

    #[test]
    fn legacy_preferences_keep_both_export_options_off() {
        let preferences: super::Preferences = serde_json::from_str("{}").unwrap();
        assert!(!preferences.retain_session_history);
        assert!(!preferences.record_session_audio);
        let enabled = super::Preferences {
            retain_session_history: true,
            record_session_audio: true,
            ..Default::default()
        };
        let restored: super::Preferences =
            serde_json::from_str(&serde_json::to_string(&enabled).unwrap()).unwrap();
        assert!(restored.retain_session_history && restored.record_session_audio);
    }

    use super::*;

    #[test]
    fn explicit_settings_preserve_chinese_translation_and_reject_wrong_provider_languages() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        store
            .save_preferences_for_active_profile(|prefs| {
                prefs.source_language = SourceLanguage::Chinese;
                prefs.target_language = TargetLanguage::English;
            })
            .unwrap();
        assert_eq!(store.preferences().target_language, TargetLanguage::English);
        let openai = store
            .create_profile(ProviderKind::OpenAIRealtime, "Synthetic realtime")
            .unwrap();
        store.select_profile(&openai.id).unwrap();
        let before = store.preferences();
        assert!(store
            .save_preferences_for_active_profile(
                |prefs| prefs.target_language = TargetLanguage::French
            )
            .is_err());
        assert_eq!(store.preferences(), before);
        assert!(store
            .save_preferences_for_active_profile(
                |prefs| prefs.source_language = SourceLanguage::French
            )
            .is_err());
        assert_eq!(store.preferences(), before);
    }

    #[test]
    fn original_only_source_is_normalized_to_auto_when_translation_is_selected() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        store
            .save_preferences_for_active_profile(|prefs| {
                prefs.target_language = TargetLanguage::Original;
                prefs.source_language = SourceLanguage::Greek;
            })
            .unwrap();
        assert_eq!(store.preferences().source_language, SourceLanguage::Greek);
        store
            .save_preferences_for_active_profile(|prefs| {
                prefs.target_language = TargetLanguage::French
            })
            .unwrap();
        assert_eq!(
            store.preferences().source_language,
            SourceLanguage::Automatic
        );
        assert_eq!(store.preferences().target_language, TargetLanguage::French);
    }

    #[test]
    fn active_text_route_change_normalizes_languages_but_inactive_profile_and_probe_do_not() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let active = store.active_profile().unwrap();
        store.save_api_key(&active.id, "synthetic-asr").unwrap();
        store
            .save_preferences_for_active_profile(|prefs| {
                prefs.source_language = SourceLanguage::French;
                prefs.target_language = TargetLanguage::TraditionalChinese;
            })
            .unwrap();
        let expanded = store.preferences();
        let other = store
            .create_profile(ProviderKind::AlibabaCloud, "Synthetic DeepL")
            .unwrap();
        store
            .save_credentials(
                &other.id,
                &translation_request(TextTranslation::DeepL, "synthetic-asr", "", "synthetic:fx"),
            )
            .unwrap();
        assert_eq!(store.preferences(), expanded);
        let updated = store.profile(&other.id).unwrap();
        let probe = store.configuration_for_profile_probe(&updated).unwrap();
        assert_eq!(probe.source_language, SourceLanguage::Automatic);
        assert_eq!(probe.target_language, TargetLanguage::Original);
        assert_eq!(store.preferences(), expanded);
        store
            .save_credentials(
                &active.id,
                &translation_request(TextTranslation::DeepL, "", "", "synthetic:fx"),
            )
            .unwrap();
        assert_eq!(
            store.preferences().source_language,
            SourceLanguage::Automatic
        );
        assert_eq!(
            store.preferences().target_language,
            TargetLanguage::Original
        );
        assert!(store.configuration().is_ok());
        store
            .save_preferences_for_active_profile(|prefs| {
                prefs.target_language = TargetLanguage::English
            })
            .unwrap();
        assert!(store
            .save_preferences_for_active_profile(
                |prefs| prefs.target_language = TargetLanguage::French
            )
            .is_err());
        store
            .save_credentials(
                &active.id,
                &translation_request(
                    TextTranslation::DeepLX,
                    "",
                    "https://example.com/translate",
                    "",
                ),
            )
            .unwrap();
        assert!(store.configuration().is_ok());
        assert!(store.preferences().target_language.translates_audio());
        store
            .save_credentials(
                &active.id,
                &translation_request(TextTranslation::FollowService, "", "", ""),
            )
            .unwrap();
        store
            .save_preferences_for_active_profile(|prefs| {
                prefs.source_language = SourceLanguage::French;
                prefs.target_language = TargetLanguage::TraditionalChinese;
            })
            .unwrap();
        assert!(store.configuration().is_ok());
    }

    #[test]
    fn failed_route_catalog_commit_restores_expanded_preferences_in_memory_and_on_disk() {
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let mut store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        let profile = store.active_profile().unwrap();
        store.save_api_key(&profile.id, "synthetic-asr").unwrap();
        store
            .save_preferences_for_active_profile(|prefs| {
                prefs.source_language = SourceLanguage::French;
                prefs.target_language = TargetLanguage::Persian;
            })
            .unwrap();
        let before = store.preferences();
        store.catalog_path = directory.path().join("blocked-catalog");
        std::fs::create_dir(&store.catalog_path).unwrap();
        assert!(store
            .save_credentials(
                &profile.id,
                &translation_request(TextTranslation::DeepL, "", "", "synthetic:fx")
            )
            .is_err());
        assert_eq!(store.preferences(), before);
        assert_eq!(
            store.active_profile().unwrap().text_translation(),
            TextTranslation::FollowService
        );
        let persisted: Preferences =
            serde_json::from_slice(&std::fs::read(&store.prefs_path).unwrap()).unwrap();
        assert_eq!(persisted, before);
        assert!(fake
            .value(
                PROFILE_KEYCHAIN_SERVICE,
                &SettingsStore::destination_account(&profile)
            )
            .is_none());
        assert!(store.configuration().is_ok());
    }

    #[test]
    fn expanded_languages_persist_and_reload_with_route_validation() {
        let fake = FakeSecretStore::default();
        let directory = tempfile::tempdir().unwrap();
        let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        store
            .save_preferences_for_active_profile(|prefs| {
                prefs.source_language = SourceLanguage::Filipino;
                prefs.target_language = TargetLanguage::Persian;
            })
            .unwrap();
        let restarted = SettingsStore::at_path(directory.path().into(), Box::new(fake));
        assert_eq!(
            restarted.preferences().source_language,
            SourceLanguage::Filipino
        );
        assert_eq!(
            restarted.preferences().target_language,
            TargetLanguage::Persian
        );
    }

    use std::cell::Cell;
    use std::sync::Arc;

    fn persistence_attributes(value: &str) -> HashMap<String, String> {
        HashMap::from([("persistence".to_string(), value.to_string())])
    }

    #[test]
    fn local_windows_credential_is_not_rewritten() {
        let reads = Cell::new(0);
        let rewrites = Cell::new(0);

        ensure_local_credential_persistence(
            "secret",
            || {
                reads.set(reads.get() + 1);
                Ok(persistence_attributes("Local"))
            },
            |_| {
                rewrites.set(rewrites.get() + 1);
                Ok(())
            },
            || {},
        )
        .unwrap();

        assert_eq!(reads.get(), 1);
        assert_eq!(rewrites.get(), 0);
    }

    #[test]
    fn enterprise_windows_credential_is_rewritten_and_verified_once() {
        let reads = Cell::new(0);
        let rewrites = Cell::new(0);

        ensure_local_credential_persistence(
            "secret",
            || {
                let read = reads.get();
                reads.set(read + 1);
                Ok(persistence_attributes(if read == 0 {
                    "Enterprise"
                } else {
                    "Local"
                }))
            },
            |value| {
                assert_eq!(value, "secret");
                rewrites.set(rewrites.get() + 1);
                Ok(())
            },
            || {},
        )
        .unwrap();

        assert_eq!(reads.get(), 2);
        assert_eq!(rewrites.get(), 1);
    }

    #[test]
    fn windows_credential_migration_fails_closed_when_local_state_is_not_verified() {
        let reads = Cell::new(0);
        let rewrites = Cell::new(0);

        let result = ensure_local_credential_persistence(
            "secret",
            || {
                reads.set(reads.get() + 1);
                Ok(persistence_attributes("Enterprise"))
            },
            |_| {
                rewrites.set(rewrites.get() + 1);
                Ok(())
            },
            || {},
        );

        assert_eq!(result, Err(SecretStoreError::Unavailable));
        assert_eq!(reads.get(), 4);
        assert_eq!(rewrites.get(), 1);
    }

    #[test]
    fn windows_credential_verification_retries_a_transient_attribute_failure() {
        let reads = Cell::new(0);
        let pauses = Cell::new(0);

        ensure_local_credential_persistence(
            "secret",
            || {
                let read = reads.get();
                reads.set(read + 1);
                match read {
                    0 => Ok(persistence_attributes("Enterprise")),
                    1 => Err(SecretStoreError::Unavailable),
                    _ => Ok(persistence_attributes("Local")),
                }
            },
            |_| Ok(()),
            || pauses.set(pauses.get() + 1),
        )
        .unwrap();

        assert_eq!(reads.get(), 3);
        assert_eq!(pauses.get(), 1);
    }

    #[test]
    fn local_windows_credential_retries_an_initial_attribute_failure_without_rewrite() {
        let reads = Cell::new(0);
        let rewrites = Cell::new(0);

        ensure_local_credential_persistence(
            "secret",
            || {
                let read = reads.get();
                reads.set(read + 1);
                if read == 0 {
                    Err(SecretStoreError::Unavailable)
                } else {
                    Ok(persistence_attributes("Local"))
                }
            },
            |_| {
                rewrites.set(rewrites.get() + 1);
                Ok(())
            },
            || {},
        )
        .unwrap();

        assert_eq!(reads.get(), 2);
        assert_eq!(rewrites.get(), 0);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_credential_manager_migrates_enterprise_to_local_without_changing_secret() {
        struct Cleanup {
            service: String,
            account: String,
        }

        impl Drop for Cleanup {
            fn drop(&mut self) {
                if let Ok(entry) = keyring_core::Entry::new(&self.service, &self.account) {
                    let _ = entry.delete_credential();
                }
            }
        }

        assert!(keyring::Entry::store_status().is_ok());
        let unique = uuid::Uuid::new_v4().simple().to_string();
        let service = format!("app.yuxino.mimi.test.{unique}");
        let account = format!("credential-local-migration-{unique}");
        let _cleanup = Cleanup {
            service: service.clone(),
            account: account.clone(),
        };
        let enterprise = keyring_core::Entry::new(&service, &account).unwrap();
        enterprise.set_password("migration-secret").unwrap();
        assert_eq!(
            enterprise.get_attributes().unwrap()["persistence"],
            "Enterprise"
        );

        assert_eq!(
            KeyringSecretStore.load(&service, &account).unwrap(),
            Some("migration-secret".to_string())
        );

        let observed = keyring_core::Entry::new(&service, &account).unwrap();
        assert_eq!(observed.get_password().unwrap(), "migration-secret");
        assert_eq!(observed.get_attributes().unwrap()["persistence"], "Local");
    }

    #[derive(Default)]
    struct FakeState {
        values: HashMap<SecretCacheKey, String>,
        failures: HashMap<SecretCacheKey, SecretStoreError>,
        unavailable: HashSet<SecretCacheKey>,
        unavailable_deletes: HashSet<SecretCacheKey>,
        loads: Vec<SecretCacheKey>,
    }

    #[derive(Clone, Default)]
    struct FakeSecretStore {
        state: Arc<Mutex<FakeState>>,
    }

    impl FakeSecretStore {
        fn put(&self, service: &str, account: &str, value: &str) {
            self.state
                .lock()
                .unwrap()
                .values
                .insert(cache_key(service, account), value.to_string());
        }

        fn value(&self, service: &str, account: &str) -> Option<String> {
            self.state
                .lock()
                .unwrap()
                .values
                .get(&cache_key(service, account))
                .cloned()
        }

        fn make_unavailable(&self, service: &str, account: &str) {
            self.state
                .lock()
                .unwrap()
                .unavailable
                .insert(cache_key(service, account));
        }

        fn make_delete_unavailable(&self, service: &str, account: &str) {
            self.state
                .lock()
                .unwrap()
                .unavailable_deletes
                .insert(cache_key(service, account));
        }

        fn load_count(&self, service: &str, account: &str) -> usize {
            self.state
                .lock()
                .unwrap()
                .loads
                .iter()
                .filter(|candidate| **candidate == cache_key(service, account))
                .count()
        }
    }

    impl SecretStore for FakeSecretStore {
        fn load(&self, service: &str, account: &str) -> Result<Option<String>, SecretStoreError> {
            let key = cache_key(service, account);
            let mut state = self.state.lock().unwrap();
            state.loads.push(key.clone());
            if let Some(error) = state.failures.get(&key) {
                return Err(*error);
            }
            if state.unavailable.contains(&key) {
                return Err(SecretStoreError::Unavailable);
            }
            Ok(state.values.get(&key).cloned())
        }

        fn save(&self, service: &str, account: &str, value: &str) -> Result<(), SecretStoreError> {
            let key = cache_key(service, account);
            let mut state = self.state.lock().unwrap();
            if let Some(error) = state.failures.get(&key) {
                return Err(*error);
            }
            if state.unavailable.contains(&key) {
                return Err(SecretStoreError::Unavailable);
            }
            state.values.insert(key, value.to_string());
            Ok(())
        }

        fn delete(&self, service: &str, account: &str) -> Result<(), SecretStoreError> {
            let key = cache_key(service, account);
            let mut state = self.state.lock().unwrap();
            if let Some(error) = state.failures.get(&key) {
                return Err(*error);
            }
            if state.unavailable.contains(&key) || state.unavailable_deletes.contains(&key) {
                return Err(SecretStoreError::Unavailable);
            }
            state.values.remove(&key);
            Ok(())
        }
    }

    fn settings(fake: &FakeSecretStore) -> SettingsStore {
        SettingsStore::in_memory(Box::new(fake.clone()), false)
    }

    #[test]
    fn credential_reveal_is_selected_profile_only_and_does_not_touch_migration() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let default = ServiceProfile::alibaba_default();
        let other = store
            .create_profile(ProviderKind::OpenAIRealtime, "Other")
            .unwrap();
        fake.put(
            PROFILE_KEYCHAIN_SERVICE,
            &credential_account(&default),
            "synthetic-default",
        );
        fake.put(
            PROFILE_KEYCHAIN_SERVICE,
            &credential_account(&other),
            "synthetic-other",
        );
        fake.put(
            LEGACY_KEYCHAIN_SERVICE_V3,
            LEGACY_KEYCHAIN_ACCOUNT,
            "synthetic-legacy",
        );
        assert_eq!(
            store
                .reveal_credential(&other.id, CredentialRevealField::ApiKey, None)
                .unwrap()
                .as_deref(),
            Some("synthetic-other")
        );
        assert_eq!(
            fake.load_count(PROFILE_KEYCHAIN_SERVICE, &credential_account(&default)),
            0
        );
        assert_eq!(
            store
                .reveal_credential(&default.id, CredentialRevealField::ApiKey, None)
                .unwrap()
                .as_deref(),
            Some("synthetic-default")
        );
        assert_eq!(
            fake.load_count(LEGACY_KEYCHAIN_SERVICE_V3, LEGACY_KEYCHAIN_ACCOUNT),
            0
        );
        assert_eq!(
            fake.load_count(PROFILE_KEYCHAIN_SERVICE, LEGACY_MIGRATION_TOMBSTONE_ACCOUNT),
            0
        );
        assert_eq!(store.active_profile().unwrap().id, default.id);
        let snapshot =
            serde_json::to_string(&(store.profile_catalog().unwrap(), store.preferences()))
                .unwrap();
        assert!(!snapshot.contains("synthetic-"));
    }

    #[test]
    fn credential_reveal_has_safe_missing_failure_and_mismatch_results() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = store
            .create_profile(ProviderKind::OpenAIRealtime, "Missing")
            .unwrap();
        assert_eq!(
            store
                .reveal_credential(
                    &profile.id,
                    CredentialRevealField::Token,
                    Some(TextTranslation::DeepLX)
                )
                .unwrap_err(),
            "credential_reveal_field_mismatch"
        );
        assert_eq!(
            fake.load_count(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile)),
            0
        );
        assert_eq!(
            store.reveal_credential(&profile.id, CredentialRevealField::ApiKey, None),
            Ok(None)
        );
        let failing = store
            .create_profile(ProviderKind::OpenAIRealtime, "Unavailable")
            .unwrap();
        fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &credential_account(&failing));
        assert_eq!(
            store
                .reveal_credential(&failing.id, CredentialRevealField::ApiKey, None)
                .unwrap_err(),
            CREDENTIAL_STORE_UNAVAILABLE
        );
    }

    #[test]
    fn credential_reveal_destination_is_saved_route_scoped_and_independent_of_asr() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = store
            .create_profile(ProviderKind::AlibabaCloud, "Speech")
            .unwrap();
        store
            .save_credentials(
                &profile.id,
                &translation_request(
                    TextTranslation::DeepL,
                    "synthetic-asr",
                    "",
                    "synthetic-official:fx",
                ),
            )
            .unwrap();
        let profile = store.profile(&profile.id).unwrap();
        let account = SettingsStore::destination_account(&profile);
        let count = fake.load_count(PROFILE_KEYCHAIN_SERVICE, &account);
        assert_eq!(
            store
                .reveal_credential(
                    &profile.id,
                    CredentialRevealField::Token,
                    Some(TextTranslation::DeepLX)
                )
                .unwrap_err(),
            "credential_reveal_field_mismatch"
        );
        assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, &account), count);
        assert_eq!(
            store
                .reveal_credential(
                    &profile.id,
                    CredentialRevealField::Token,
                    Some(TextTranslation::DeepL)
                )
                .unwrap()
                .as_deref(),
            Some("synthetic-official:fx")
        );
        // A broken destination must not make the independent ASR key unreadable.
        fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &account);
        store
            .secret_cache
            .lock()
            .unwrap()
            .remove(&cache_key(PROFILE_KEYCHAIN_SERVICE, &account));
        assert_eq!(
            store
                .reveal_credential(&profile.id, CredentialRevealField::ApiKey, None)
                .unwrap()
                .as_deref(),
            Some("synthetic-asr")
        );
        assert_eq!(
            store
                .reveal_credential(
                    &profile.id,
                    CredentialRevealField::Token,
                    Some(TextTranslation::DeepL)
                )
                .unwrap_err(),
            CREDENTIAL_STORE_UNAVAILABLE
        );
    }

    #[test]
    fn linux_storage_access_errors_are_classified_without_exposing_payloads() {
        let private_detail = "synthetic-private-native-detail";
        let access = keyring_core::Error::NoStorageAccess(Box::new(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            private_detail,
        )));
        assert_eq!(
            secret_service_operation_error(access),
            SecretStoreError::AccessDenied
        );
        let failure =
            keyring_core::Error::PlatformFailure(Box::new(std::io::Error::other(private_detail)));
        for error in [
            failure,
            keyring_core::Error::BadEncoding(private_detail.as_bytes().to_vec()),
            keyring_core::Error::NoDefaultStore,
        ] {
            let safe = secret_service_operation_error(error);
            assert_eq!(safe, SecretStoreError::Unavailable);
            assert!(!safe.public_error().contains(private_detail));
        }
    }

    #[test]
    fn credential_failures_preserve_categories_and_recover_without_plaintext_storage() {
        for (failure, diagnostic, public_error) in [
            (
                SecretStoreError::Unavailable,
                "unavailable",
                CREDENTIAL_STORE_UNAVAILABLE,
            ),
            (
                SecretStoreError::ServiceUnavailable,
                "serviceUnavailable",
                CREDENTIAL_SERVICE_UNAVAILABLE,
            ),
            (
                SecretStoreError::AccessDenied,
                "accessDenied",
                CREDENTIAL_STORE_ACCESS_DENIED,
            ),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let fake = FakeSecretStore::default();
            let store = SettingsStore::load_with_secret(
                directory.path().to_path_buf(),
                false,
                Box::new(fake.clone()),
                PROFILE_KEYCHAIN_SERVICE,
                false,
            );
            let profile = store.active_profile().unwrap();
            let slot = cache_key(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile));
            fake.state
                .lock()
                .unwrap()
                .failures
                .insert(slot.clone(), failure);
            assert_eq!(
                store.credential_state(&profile),
                CredentialState::Unavailable
            );
            assert_eq!(store.credential_diagnostic(&profile), diagnostic);
            assert_eq!(store.configuration().unwrap_err().to_string(), public_error);
            assert_eq!(
                store
                    .save_api_key(&profile.id, "synthetic-recovery-value")
                    .unwrap_err(),
                public_error
            );
            assert_eq!(store.delete_api_key(&profile.id).unwrap_err(), public_error);
            assert_eq!(store.active_profile().unwrap().id, profile.id);
            assert_eq!(
                fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile)),
                None
            );

            fake.state.lock().unwrap().failures.remove(&slot);
            assert_eq!(store.credential_diagnostic(&profile), "missing");
            assert_eq!(store.credential_state(&profile), CredentialState::Missing);
            store
                .save_api_key(&profile.id, "synthetic-recovery-value")
                .unwrap();
            assert_eq!(store.credential_diagnostic(&profile), "present");
            for path in [store.prefs_path.clone(), store.catalog_path.clone()] {
                if let Ok(json) = std::fs::read_to_string(path) {
                    assert!(!json.contains("synthetic-recovery-value"));
                }
            }
            store.delete_api_key(&profile.id).unwrap();
            assert_eq!(store.credential_diagnostic(&profile), "missing");
        }
    }

    fn translation_request(
        translation: TextTranslation,
        key: &str,
        endpoint: &str,
        token: &str,
    ) -> ProviderCredentials {
        ProviderCredentials::AlibabaTranslation {
            api_key: key.into(),
            text_translation: translation,
            endpoint: endpoint.into(),
            token: token.into(),
            model: String::new(),
        }
    }

    fn openai_compatible_request(
        asr_key: &str,
        endpoint: &str,
        api_key: &str,
        model: &str,
    ) -> ProviderCredentials {
        ProviderCredentials::AlibabaTranslation {
            api_key: asr_key.into(),
            text_translation: TextTranslation::OpenAICompatible,
            endpoint: endpoint.into(),
            token: api_key.into(),
            model: model.into(),
        }
    }

    #[test]
    fn openai_compatible_destination_survives_restart_and_route_switches_without_crossing_secrets()
    {
        for provider in [ProviderKind::AlibabaCloud, ProviderKind::DeepLX] {
            let directory = tempfile::tempdir().unwrap();
            let fake = FakeSecretStore::default();
            let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
            let profile = store.create_profile(provider, "Speech").unwrap();
            if provider == ProviderKind::DeepLX {
                store
                    .save_credentials(
                        &profile.id,
                        &ProviderCredentials::DeepLX {
                            asr_api_key: "synthetic-asr".into(),
                            endpoint: "https://example.com/translate".into(),
                            token: "synthetic-deeplx".into(),
                        },
                    )
                    .unwrap();
            } else {
                store.save_api_key(&profile.id, "synthetic-asr").unwrap();
                store
                    .save_credentials(
                        &profile.id,
                        &translation_request(
                            TextTranslation::DeepLX,
                            "",
                            "https://example.com/translate",
                            "synthetic-deeplx",
                        ),
                    )
                    .unwrap();
            }
            store.select_profile(&profile.id).unwrap();
            let account = credential_account(&profile);
            let original_asr_item = fake.value(PROFILE_KEYCHAIN_SERVICE, &account).unwrap();
            fake.make_delete_unavailable(PROFILE_KEYCHAIN_SERVICE, &account);
            fake.make_delete_unavailable(
                PROFILE_KEYCHAIN_SERVICE,
                &SettingsStore::destination_account(&profile),
            );
            store
                .save_credentials(
                    &profile.id,
                    &translation_request(TextTranslation::DeepL, "", "", "synthetic-deepl:fx"),
                )
                .unwrap();
            store
                .save_credentials(
                    &profile.id,
                    &openai_compatible_request(
                        "",
                        "https://third-party.example/v1",
                        "synthetic-openai",
                        "synthetic-model",
                    ),
                )
                .unwrap();
            let destination_before = fake
                .value(
                    PROFILE_KEYCHAIN_SERVICE,
                    &SettingsStore::destination_account(&profile),
                )
                .unwrap();
            for private in [
                "synthetic-asr",
                "synthetic-deepl",
                "synthetic-openai",
                "synthetic-model",
                "third-party.example",
            ] {
                for path in [&store.prefs_path, &store.catalog_path] {
                    assert!(!std::fs::read_to_string(path).unwrap().contains(private));
                }
            }
            drop(store);
            fake.state.lock().unwrap().loads.clear();
            let reloaded = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
            assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, &account), 0);
            for route in [
                TextTranslation::OpenAICompatible,
                TextTranslation::DeepL,
                TextTranslation::DeepLX,
                TextTranslation::FollowService,
                TextTranslation::OpenAICompatible,
            ] {
                let request = translation_request(route, "", "", "");
                reloaded.save_credentials(&profile.id, &request).unwrap();
                let credentials = reloaded.configuration().unwrap().credentials;
                assert_eq!(credentials.alibaba_key(), Some("synthetic-asr"));
                match route {
                    TextTranslation::OpenAICompatible => {
                        let ProviderCredentials::OpenAICompatible {
                            endpoint,
                            api_key,
                            model,
                            ..
                        } = &credentials
                        else {
                            panic!("expected OpenAI-compatible credentials")
                        };
                        assert!(endpoint.starts_with("https://third-party.example/v1"));
                        assert_eq!(api_key, "synthetic-openai");
                        assert_eq!(model, "synthetic-model");
                    }
                    TextTranslation::DeepL => assert!(
                        matches!(credentials, ProviderCredentials::DeepL { api_key, .. } if api_key == "synthetic-deepl:fx")
                    ),
                    TextTranslation::DeepLX => assert!(
                        matches!(credentials, ProviderCredentials::DeepLX { token, endpoint, .. } if token == "synthetic-deeplx" && endpoint == "https://example.com/translate")
                    ),
                    TextTranslation::FollowService => {
                        assert_eq!(credentials.direct_api_key(), Some("synthetic-asr"))
                    }
                }
                assert_eq!(
                    fake.value(PROFILE_KEYCHAIN_SERVICE, &account).as_deref(),
                    Some(original_asr_item.as_str())
                );
                assert_eq!(
                    fake.value(
                        PROFILE_KEYCHAIN_SERVICE,
                        &SettingsStore::destination_account(&profile)
                    )
                    .as_deref(),
                    Some(destination_before.as_str())
                );
            }
            assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, &account), 1);
        }
    }

    #[test]
    fn missing_openai_compatible_fields_leave_route_and_secret_items_unchanged() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = store.active_profile().unwrap();
        store.save_api_key(&profile.id, "synthetic-asr").unwrap();
        for request in [
            openai_compatible_request("", "", "synthetic-key", "synthetic-model"),
            openai_compatible_request("", "https://example.com/v1", "", "synthetic-model"),
            openai_compatible_request("", "https://example.com/v1", "synthetic-key", ""),
        ] {
            assert!(store.save_credentials(&profile.id, &request).is_err());
            assert_eq!(
                store.profile(&profile.id).unwrap().text_translation(),
                TextTranslation::FollowService
            );
            assert_eq!(
                fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile))
                    .as_deref(),
                Some("synthetic-asr")
            );
            assert!(fake
                .value(
                    PROFILE_KEYCHAIN_SERVICE,
                    &SettingsStore::destination_account(&profile)
                )
                .is_none());
        }
        let before_loads = fake.state.lock().unwrap().loads.len();
        for request in [
            openai_compatible_request(
                "",
                "http://example.com/v1",
                "synthetic-key",
                "synthetic-model",
            ),
            openai_compatible_request(
                "",
                "https://example.com/v1",
                "synthetic-key",
                "invalid\nmodel",
            ),
        ] {
            let error = store.save_credentials(&profile.id, &request).unwrap_err();
            assert!(!error.contains("synthetic"));
        }
        assert_eq!(fake.state.lock().unwrap().loads.len(), before_loads);
    }

    #[test]
    fn read_only_local_credentials_do_not_supply_or_read_an_openai_compatible_destination() {
        struct ReadOnlyCredentials;
        impl SecretStore for ReadOnlyCredentials {
            fn load(&self, _: &str, _: &str) -> Result<Option<String>, SecretStoreError> {
                panic!("independent routes must not read the Alibaba-only development file or OS slots")
            }
            fn save(&self, _: &str, _: &str, _: &str) -> Result<(), SecretStoreError> {
                Err(SecretStoreError::ReadOnly)
            }
            fn delete(&self, _: &str, _: &str) -> Result<(), SecretStoreError> {
                Err(SecretStoreError::ReadOnly)
            }
            fn is_read_only(&self) -> bool {
                true
            }
        }
        let store = SettingsStore::in_memory(Box::new(ReadOnlyCredentials), false);
        let mut profile = store.active_profile().unwrap();
        profile.text_translation = Some(TextTranslation::OpenAICompatible);
        assert!(store.credentials_for_profile(&profile).unwrap().is_none());
        assert_eq!(store.credential_state(&profile), CredentialState::Missing);
        assert_eq!(
            store.configuration_for_profile(&profile).unwrap_err(),
            crate::core::credentials::ProviderCredentialsError::Missing(ProviderKind::AlibabaCloud)
                .to_string()
        );
        assert_eq!(
            store.save_credentials(
                &profile.id,
                &openai_compatible_request(
                    "synthetic-asr",
                    "https://example.com/v1",
                    "synthetic-key",
                    "synthetic-model"
                )
            ),
            Err("local_dev_credentials_read_only".into())
        );
    }

    #[test]
    fn openai_compatible_new_endpoint_requires_a_new_key_and_model_only_updates_reuse_the_saved_key(
    ) {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = store.active_profile().unwrap();
        store
            .save_credentials(
                &profile.id,
                &openai_compatible_request(
                    "synthetic-asr",
                    "https://first.example/v1",
                    "synthetic-first",
                    "first-model",
                ),
            )
            .unwrap();
        let before = fake.value(
            PROFILE_KEYCHAIN_SERVICE,
            &SettingsStore::destination_account(&profile),
        );
        assert!(store
            .save_credentials(
                &profile.id,
                &openai_compatible_request("", "https://second.example/v1", "", "second-model")
            )
            .is_err());
        assert_eq!(
            fake.value(
                PROFILE_KEYCHAIN_SERVICE,
                &SettingsStore::destination_account(&profile)
            ),
            before
        );
        store
            .save_credentials(
                &profile.id,
                &openai_compatible_request("", "", "", "updated-model"),
            )
            .unwrap();
        let config = store.configuration().unwrap();
        let ProviderCredentials::OpenAICompatible {
            endpoint,
            api_key,
            model,
            ..
        } = &config.credentials
        else {
            panic!("expected OpenAI-compatible credentials")
        };
        assert!(endpoint.starts_with("https://first.example/"));
        assert_eq!(api_key, "synthetic-first");
        assert_eq!(model, "updated-model");
    }

    #[test]
    fn failed_openai_compatible_destination_or_catalog_write_preserves_the_working_configuration() {
        for fail_destination in [false, true] {
            let fake = FakeSecretStore::default();
            let mut store = settings(&fake);
            let profile = store.active_profile().unwrap();
            store
                .save_credentials(
                    &profile.id,
                    &openai_compatible_request(
                        "synthetic-old-asr",
                        "https://old.example/v1",
                        "synthetic-old-key",
                        "old-model",
                    ),
                )
                .unwrap();
            let before_key = fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile));
            let before_destination = fake.value(
                PROFILE_KEYCHAIN_SERVICE,
                &SettingsStore::destination_account(&profile),
            );
            let before_prefs = store.preferences();
            let blocked_catalog = tempfile::tempdir().unwrap();
            if fail_destination {
                fake.make_unavailable(
                    PROFILE_KEYCHAIN_SERVICE,
                    &SettingsStore::destination_account(&profile),
                );
            } else {
                store.catalog_path = blocked_catalog.path().into();
            }
            assert!(store
                .save_credentials(
                    &profile.id,
                    &openai_compatible_request(
                        "synthetic-new-asr",
                        "https://new.example/v1",
                        "synthetic-new-key",
                        "new-model"
                    )
                )
                .is_err());
            assert_eq!(store.preferences(), before_prefs);
            assert_eq!(
                store.profile(&profile.id).unwrap().text_translation(),
                TextTranslation::OpenAICompatible
            );
            assert_eq!(
                fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile)),
                before_key
            );
            assert_eq!(
                fake.value(
                    PROFILE_KEYCHAIN_SERVICE,
                    &SettingsStore::destination_account(&profile)
                ),
                before_destination
            );
            fake.state.lock().unwrap().unavailable.clear();
            assert_eq!(
                store.credential_diagnostic(&store.active_profile().unwrap()),
                "present"
            );
            assert!(
                matches!(store.configuration().unwrap().credentials, ProviderCredentials::OpenAICompatible { asr_api_key, api_key, model, .. } if asr_api_key == "synthetic-old-asr" && api_key == "synthetic-old-key" && model == "old-model")
            );
        }
    }

    #[test]
    fn deepl_preset_and_custom_destination_keep_their_secrets_separate() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = store
            .create_profile(ProviderKind::AlibabaCloud, "Speech")
            .unwrap();
        store.save_api_key(&profile.id, "synthetic-asr").unwrap();
        store.select_profile(&profile.id).unwrap();
        store
            .save_credentials(
                &profile.id,
                &translation_request(TextTranslation::DeepL, "", "", "synthetic-deepl:fx"),
            )
            .unwrap();
        assert!(matches!(store.configuration().unwrap().credentials,
            ProviderCredentials::DeepL { asr_api_key, api_key } if asr_api_key == "synthetic-asr" && api_key == "synthetic-deepl:fx"));
        store
            .save_credentials(
                &profile.id,
                &translation_request(
                    TextTranslation::DeepLX,
                    "",
                    "https://example.com/translate",
                    "synthetic-custom",
                ),
            )
            .unwrap();
        assert!(matches!(store.configuration().unwrap().credentials,
            ProviderCredentials::DeepLX { token, .. } if token == "synthetic-custom"));
        store
            .save_credentials(
                &profile.id,
                &translation_request(TextTranslation::FollowService, "", "", ""),
            )
            .unwrap();
        assert_eq!(
            store.configuration().unwrap().credentials.alibaba_key(),
            Some("synthetic-asr")
        );
        store
            .save_credentials(
                &profile.id,
                &translation_request(TextTranslation::DeepL, "", "", ""),
            )
            .unwrap();
        assert!(matches!(store.configuration().unwrap().credentials,
            ProviderCredentials::DeepL { api_key, .. } if api_key == "synthetic-deepl:fx"));
        store
            .save_credentials(
                &profile.id,
                &translation_request(TextTranslation::DeepLX, "", "", ""),
            )
            .unwrap();
        assert!(matches!(store.configuration().unwrap().credentials,
            ProviderCredentials::DeepLX { endpoint, token, .. } if endpoint == "https://example.com/translate" && token == "synthetic-custom"));
        let snapshot = serde_json::to_string(&store.profile_catalog().unwrap()).unwrap();
        for private in [
            "synthetic-asr",
            "synthetic-deepl",
            "synthetic-custom",
            "example.com",
        ] {
            assert!(!snapshot.contains(private));
        }
        let profile = store.profile(&profile.id).unwrap();
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile))
                .as_deref(),
            Some("synthetic-asr")
        );
    }

    #[test]
    fn legacy_custom_profiles_keep_both_destinations_without_rewriting_the_asr_item() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = store
            .create_profile(ProviderKind::DeepLX, "Existing")
            .unwrap();
        store
            .save_credentials(
                &profile.id,
                &ProviderCredentials::DeepLX {
                    asr_api_key: "synthetic-asr".into(),
                    endpoint: "https://example.com/translate".into(),
                    token: "synthetic-custom".into(),
                },
            )
            .unwrap();
        store.select_profile(&profile.id).unwrap();
        let account = credential_account(&profile);
        let before = fake.value(PROFILE_KEYCHAIN_SERVICE, &account).unwrap();
        store
            .save_credentials(
                &profile.id,
                &translation_request(TextTranslation::DeepL, "", "", "synthetic-deepl:fx"),
            )
            .unwrap();
        assert!(
            matches!(store.configuration().unwrap().credentials, ProviderCredentials::DeepL { api_key, .. } if api_key == "synthetic-deepl:fx")
        );
        for route in [
            TextTranslation::DeepLX,
            TextTranslation::FollowService,
            TextTranslation::DeepL,
            TextTranslation::DeepLX,
        ] {
            store
                .save_credentials(&profile.id, &translation_request(route, "", "", ""))
                .unwrap();
            match route {
                TextTranslation::DeepL => assert!(
                    matches!(store.configuration().unwrap().credentials, ProviderCredentials::DeepL { api_key, .. } if api_key == "synthetic-deepl:fx")
                ),
                TextTranslation::DeepLX => assert!(
                    matches!(store.configuration().unwrap().credentials, ProviderCredentials::DeepLX { endpoint, token, .. } if endpoint == "https://example.com/translate" && token == "synthetic-custom")
                ),
                TextTranslation::FollowService => assert_eq!(
                    store.configuration().unwrap().credentials.direct_api_key(),
                    Some("synthetic-asr")
                ),
                TextTranslation::OpenAICompatible => unreachable!(),
            }
            assert_eq!(
                fake.value(PROFILE_KEYCHAIN_SERVICE, &account).as_deref(),
                Some(before.as_str())
            );
        }
    }

    #[test]
    fn missing_deepl_key_does_not_replace_the_working_route_or_write_credentials() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = store
            .create_profile(ProviderKind::AlibabaCloud, "Speech")
            .unwrap();
        store.save_api_key(&profile.id, "synthetic-asr").unwrap();
        assert!(store
            .save_credentials(
                &profile.id,
                &translation_request(TextTranslation::DeepL, "", "", "")
            )
            .is_err());
        assert_eq!(
            store.profile(&profile.id).unwrap().text_translation(),
            TextTranslation::FollowService
        );
        assert!(fake
            .value(
                PROFILE_KEYCHAIN_SERVICE,
                &SettingsStore::destination_account(&profile)
            )
            .is_none());
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile))
                .as_deref(),
            Some("synthetic-asr")
        );
    }

    #[test]
    fn alibaba_advanced_translation_reuses_one_raw_key_and_retains_the_destination() {
        let fake = FakeSecretStore::default();
        let store = SettingsStore::in_memory(Box::new(fake.clone()), false);
        let profile = store
            .create_profile(ProviderKind::AlibabaCloud, "Work")
            .unwrap();
        store.save_api_key(&profile.id, "synthetic-asr").unwrap();
        let account = credential_account(&profile);
        let destination_account = SettingsStore::destination_account(&profile);
        store
            .save_credentials(
                &profile.id,
                &translation_request(
                    TextTranslation::DeepLX,
                    "",
                    "https://example.com/api",
                    "synthetic-token",
                ),
            )
            .unwrap();
        store.select_profile(&profile.id).unwrap();
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, &account).as_deref(),
            Some("synthetic-asr")
        );
        let destination = fake
            .value(PROFILE_KEYCHAIN_SERVICE, &destination_account)
            .unwrap();
        assert!(!destination.contains("synthetic-asr"));
        assert!(destination.contains("synthetic-token"));
        assert_eq!(
            store.configuration().unwrap().provider,
            ProviderKind::DeepLX
        );
        let snapshot = serde_json::to_string(&store.profile_catalog().unwrap()).unwrap();
        for secret in ["synthetic-asr", "synthetic-token", "example.com"] {
            assert!(!snapshot.contains(secret));
        }
        store.update_profile(&profile.id, "Renamed").unwrap();
        assert_eq!(
            store.profile(&profile.id).unwrap().text_translation(),
            TextTranslation::DeepLX
        );
        store
            .save_credentials(
                &profile.id,
                &translation_request(TextTranslation::FollowService, "", "", ""),
            )
            .unwrap();
        store.select_profile(&profile.id).unwrap();
        assert_eq!(
            store.configuration().unwrap().provider,
            ProviderKind::AlibabaCloud
        );
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, &destination_account)
                .as_deref(),
            Some(destination.as_str())
        );
        store
            .save_credentials(
                &profile.id,
                &translation_request(TextTranslation::DeepLX, "", "", "new-synthetic-token"),
            )
            .unwrap();
        store.select_profile(&profile.id).unwrap();
        assert!(
            matches!(store.configuration().unwrap().credentials, ProviderCredentials::DeepLX { asr_api_key, endpoint, token } if asr_api_key == "synthetic-asr" && endpoint == "https://example.com/api/translate" && token == "new-synthetic-token")
        );
        store.delete_api_key(&profile.id).unwrap();
        assert_eq!(fake.value(PROFILE_KEYCHAIN_SERVICE, &account), None);
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, &destination_account),
            None
        );
    }

    #[test]
    fn legacy_deeplx_profile_keeps_its_id_account_and_secret_on_default_loading() {
        let fake = FakeSecretStore::default();
        let store = SettingsStore::in_memory(Box::new(fake.clone()), false);
        let profile = store
            .create_profile(ProviderKind::DeepLX, "Existing custom name")
            .unwrap();
        store
            .save_credentials(
                &profile.id,
                &ProviderCredentials::DeepLX {
                    asr_api_key: "synthetic-asr".into(),
                    endpoint: "https://example.com".into(),
                    token: "synthetic-token".into(),
                },
            )
            .unwrap();
        let account = credential_account(&profile);
        let original = fake.value(PROFILE_KEYCHAIN_SERVICE, &account);
        store.select_profile(&profile.id).unwrap();
        assert_eq!(
            store.configuration().unwrap().provider,
            ProviderKind::DeepLX
        );
        assert_eq!(store.profile(&profile.id).unwrap().text_translation, None);
        assert_eq!(fake.value(PROFILE_KEYCHAIN_SERVICE, &account), original);
        store
            .save_credentials(
                &profile.id,
                &translation_request(TextTranslation::FollowService, "", "", ""),
            )
            .unwrap();
        store.select_profile(&profile.id).unwrap();
        assert_eq!(
            store.configuration().unwrap().provider,
            ProviderKind::AlibabaCloud
        );
        assert_eq!(fake.value(PROFILE_KEYCHAIN_SERVICE, &account), original);
        store
            .save_credentials(
                &profile.id,
                &translation_request(TextTranslation::DeepLX, "", "", ""),
            )
            .unwrap();
        store.select_profile(&profile.id).unwrap();
        assert_eq!(
            store.configuration().unwrap().provider,
            ProviderKind::DeepLX
        );
        assert_eq!(
            store.profile(&profile.id).unwrap().provider,
            ProviderKind::DeepLX
        );
        assert_eq!(fake.value(PROFILE_KEYCHAIN_SERVICE, &account), original);
    }

    #[test]
    fn advanced_translation_rejects_unsupported_providers_invalid_destinations_and_missing_keys() {
        let fake = FakeSecretStore::default();
        let store = SettingsStore::in_memory(Box::new(fake.clone()), false);
        let other = store
            .create_profile(ProviderKind::OpenAIRealtime, "Other")
            .unwrap();
        assert!(store
            .save_credentials(
                &other.id,
                &translation_request(
                    TextTranslation::DeepLX,
                    "synthetic",
                    "https://example.com",
                    ""
                )
            )
            .is_err());
        let profile = store
            .create_profile(ProviderKind::AlibabaCloud, "Ali")
            .unwrap();
        let before_loads = fake.state.lock().unwrap().loads.len();
        assert!(store
            .save_credentials(
                &profile.id,
                &translation_request(TextTranslation::DeepLX, "synthetic", "bad", "")
            )
            .is_err());
        assert_eq!(fake.state.lock().unwrap().loads.len(), before_loads);
        assert!(store
            .save_credentials(
                &profile.id,
                &translation_request(TextTranslation::DeepLX, "", "https://example.com", "")
            )
            .is_err());
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile)),
            None
        );
        assert_eq!(
            fake.value(
                PROFILE_KEYCHAIN_SERVICE,
                &SettingsStore::destination_account(&profile)
            ),
            None
        );
        assert_eq!(store.profile(&profile.id).unwrap().text_translation, None);
    }

    #[test]
    fn advanced_translation_survives_restart_without_secret_metadata() {
        let fake = FakeSecretStore::default();
        let directory = std::env::temp_dir().join(format!(
            "mimi-text-translation-test-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let store = SettingsStore::at_path(directory.clone(), Box::new(fake.clone()));
        let profile = store
            .create_profile(ProviderKind::AlibabaCloud, "Ali")
            .unwrap();
        store
            .save_credentials(
                &profile.id,
                &translation_request(
                    TextTranslation::DeepLX,
                    "synthetic-asr",
                    "http://localhost:1188",
                    "",
                ),
            )
            .unwrap();
        store.select_profile(&profile.id).unwrap();
        drop(store);
        let reloaded = SettingsStore::at_path(directory.clone(), Box::new(fake.clone()));
        assert_eq!(
            reloaded.configuration().unwrap().provider,
            ProviderKind::DeepLX
        );
        assert_eq!(reloaded.active_profile().unwrap().id, profile.id);
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile))
                .as_deref(),
            Some("synthetic-asr")
        );
        let metadata = std::fs::read_to_string(directory.join(PROFILE_CATALOG_FILE)).unwrap();
        for secret in ["synthetic-asr", "localhost", "endpoint", "token"] {
            assert!(!metadata.contains(secret));
        }
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn failed_advanced_catalog_write_restores_key_destination_and_selection() {
        let fake = FakeSecretStore::default();
        let store = SettingsStore::in_memory(Box::new(fake.clone()), false);
        let profile = store
            .create_profile(ProviderKind::AlibabaCloud, "Ali")
            .unwrap();
        store.save_api_key(&profile.id, "synthetic-old").unwrap();
        store
            .save_credentials(
                &profile.id,
                &translation_request(
                    TextTranslation::DeepLX,
                    "",
                    "https://example.com/old",
                    "old-token",
                ),
            )
            .unwrap();
        let previous_key = fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile));
        let previous_destination = fake.value(
            PROFILE_KEYCHAIN_SERVICE,
            &SettingsStore::destination_account(&profile),
        );
        let directory = std::env::temp_dir().join(format!(
            "mimi-text-write-test-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let mut store = store;
        // A directory cannot be replaced by the atomic catalog file write.
        store.catalog_path = directory.clone();
        assert!(store
            .save_credentials(
                &profile.id,
                &translation_request(
                    TextTranslation::DeepLX,
                    "synthetic-new",
                    "https://example.com/new",
                    "new-token"
                )
            )
            .is_err());
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile)),
            previous_key
        );
        assert_eq!(
            fake.value(
                PROFILE_KEYCHAIN_SERVICE,
                &SettingsStore::destination_account(&profile)
            ),
            previous_destination
        );
        assert_eq!(
            store.profile(&profile.id).unwrap().text_translation(),
            TextTranslation::DeepLX
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn failed_destination_delete_preserves_profile_and_key() {
        let fake = FakeSecretStore::default();
        let store = SettingsStore::in_memory(Box::new(fake.clone()), false);
        let profile = store
            .create_profile(ProviderKind::AlibabaCloud, "Ali")
            .unwrap();
        store
            .save_credentials(
                &profile.id,
                &translation_request(
                    TextTranslation::DeepLX,
                    "synthetic-asr",
                    "https://example.com",
                    "",
                ),
            )
            .unwrap();
        fake.make_delete_unavailable(
            PROFILE_KEYCHAIN_SERVICE,
            &SettingsStore::destination_account(&profile),
        );
        assert!(store.delete_profile(&profile.id).is_err());
        assert!(store.profile(&profile.id).is_ok());
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile))
                .as_deref(),
            Some("synthetic-asr")
        );
        assert!(fake
            .value(
                PROFILE_KEYCHAIN_SERVICE,
                &SettingsStore::destination_account(&profile)
            )
            .is_some());
    }

    fn openai_profile(store: &SettingsStore, name: &str) -> ServiceProfile {
        store
            .create_profile(ProviderKind::OpenAIRealtime, name)
            .unwrap()
    }

    #[test]
    fn deeplx_values_use_profile_secure_storage_and_never_snapshot_json() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = store
            .create_profile(ProviderKind::DeepLX, "Third-party translation")
            .unwrap();
        let credentials = ProviderCredentials::DeepLX {
            asr_api_key: "synthetic-asr".into(),
            endpoint: "https://example.com/translate".into(),
            token: "synthetic-token".into(),
        };
        store.save_credentials(&profile.id, &credentials).unwrap();
        store.select_profile(&profile.id).unwrap();
        assert_eq!(store.configuration().unwrap().credentials, credentials);
        assert_eq!(store.credential_state(&profile), CredentialState::Present);
        let snapshot =
            serde_json::to_string(&(store.profile_catalog().unwrap(), store.preferences()))
                .unwrap();
        for value in ["synthetic-asr", "synthetic-token", "https://example.com"] {
            assert!(!snapshot.contains(value));
        }
        store.delete_api_key(&profile.id).unwrap();
        assert_eq!(store.credential_state(&profile), CredentialState::Missing);
    }

    fn private_ui_fixture() -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("mimi-ui-test-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir(&directory).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        directory
    }

    #[test]
    fn ordinary_ui_test_does_not_read_or_write_existing_user_config() {
        let directory = private_ui_fixture();
        let preferences = directory.join("preferences.json");
        let catalog = directory.join(PROFILE_CATALOG_FILE);
        std::fs::write(&preferences, br#"{"pulse_style":"ribbon","font_size":20}"#).unwrap();
        std::fs::write(&catalog, b"existing fixture catalog").unwrap();
        let store =
            SettingsStore::load(directory.clone(), true, DEVELOPMENT_APPLICATION_IDENTIFIER);
        assert_eq!(store.preferences(), Preferences::default());
        store
            .save_preferences(|preferences| preferences.pulse_style = PulseStyle::Syllable)
            .unwrap();
        assert_eq!(
            std::fs::read(&preferences).unwrap(),
            br#"{"pulse_style":"ribbon","font_size":20}"#
        );
        assert_eq!(
            std::fs::read(&catalog).unwrap(),
            b"existing fixture catalog"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn explicit_ui_fixture_restarts_non_secret_preferences_only() {
        let directory = private_ui_fixture();
        let store = SettingsStore::load_ui_test_preferences(directory.clone()).unwrap();
        store
            .save_preferences(|preferences| {
                preferences.pulse_style = PulseStyle::Ribbon;
                preferences.pulse_animation = Some(false);
                preferences.subtitle_animation = Some(true);
                preferences.ui_language = Some("ja".into());
                preferences.windows_audio_source = "safe-fixture-endpoint".into();
            })
            .unwrap();
        store
            .create_profile(ProviderKind::OpenAIRealtime, "Synthetic fixture")
            .unwrap();
        let expected = store.preferences();
        drop(store);
        let restarted = SettingsStore::load_ui_test_preferences(directory.clone()).unwrap();
        assert_eq!(restarted.preferences(), expected);
        assert!(restarted.is_ui_test());
        assert_eq!(restarted.profile_catalog().unwrap().1.len(), 1);
        assert!(!directory.join(PROFILE_CATALOG_FILE).exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn explicit_ui_fixture_rejects_unbounded_or_invalid_preferences() {
        let directory = private_ui_fixture();
        let path = directory.join("preferences.json");
        std::fs::write(&path, vec![b' '; 65537]).unwrap();
        assert!(SettingsStore::load_ui_test_preferences(directory.clone()).is_err());
        std::fs::write(&path, b"invalid").unwrap();
        assert!(SettingsStore::load_ui_test_preferences(directory.clone()).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn explicit_ui_fixture_rejects_symlinks_and_shared_directories() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let directory = private_ui_fixture();
        let path = directory.join("preferences.json");
        let target = directory.join("fixture.json");
        std::fs::write(&target, b"{}").unwrap();
        symlink(&target, &path).unwrap();
        assert!(SettingsStore::load_ui_test_preferences(directory.clone()).is_err());
        std::fs::remove_file(&path).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(SettingsStore::load_ui_test_preferences(directory.clone()).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn credentials_are_isolated_by_profile_and_provider() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let first = openai_profile(&store, "Work");
        let second = openai_profile(&store, "Personal");

        store.save_api_key(&first.id, "sk-first").unwrap();
        store.save_api_key(&second.id, "sk-second").unwrap();

        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&first))
                .as_deref(),
            Some("sk-first")
        );
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&second))
                .as_deref(),
            Some("sk-second")
        );
        assert_ne!(credential_account(&first), credential_account(&second));
    }

    #[test]
    fn creating_a_profile_never_copies_the_active_secret() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        store
            .save_api_key(DEFAULT_ALIBABA_PROFILE_ID, "sk-alibaba")
            .unwrap();
        let profile = openai_profile(&store, "OpenAI");

        assert_eq!(store.credential_state(&profile), CredentialState::Missing);
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile)),
            None
        );
    }

    #[test]
    fn development_scope_does_not_read_or_migrate_release_credentials() {
        let fake = FakeSecretStore::default();
        fake.put(
            PROFILE_KEYCHAIN_SERVICE,
            &credential_account(&ServiceProfile::alibaba_default()),
            "sk-release-profile",
        );
        fake.put(
            LEGACY_KEYCHAIN_SERVICE_V3,
            LEGACY_KEYCHAIN_ACCOUNT,
            "sk-release-legacy",
        );
        let store = SettingsStore::in_memory_with_scope(
            Box::new(fake.clone()),
            false,
            DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE,
            false,
        );

        assert_eq!(store.load_api_key().unwrap(), None);
        store
            .save_api_key(DEFAULT_ALIBABA_PROFILE_ID, "sk-development")
            .unwrap();
        assert_eq!(
            fake.value(
                DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE,
                &credential_account(&ServiceProfile::alibaba_default())
            )
            .as_deref(),
            Some("sk-development")
        );
        assert_eq!(
            fake.value(
                PROFILE_KEYCHAIN_SERVICE,
                &credential_account(&ServiceProfile::alibaba_default())
            )
            .as_deref(),
            Some("sk-release-profile")
        );
    }

    #[test]
    fn legacy_key_migrates_only_to_default_alibaba_and_writes_tombstone() {
        let fake = FakeSecretStore::default();
        fake.put(
            LEGACY_KEYCHAIN_SERVICE_V3,
            LEGACY_KEYCHAIN_ACCOUNT,
            "sk-legacy",
        );
        let store = settings(&fake);
        let openai = openai_profile(&store, "OpenAI");

        assert_eq!(store.credential_state(&openai), CredentialState::Missing);
        assert_eq!(store.load_api_key().unwrap().as_deref(), Some("sk-legacy"));
        assert_eq!(
            fake.value(
                PROFILE_KEYCHAIN_SERVICE,
                &credential_account(&ServiceProfile::alibaba_default())
            )
            .as_deref(),
            Some("sk-legacy")
        );
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, LEGACY_MIGRATION_TOMBSTONE_ACCOUNT)
                .as_deref(),
            Some(LEGACY_MIGRATION_TOMBSTONE_VALUE)
        );
    }

    #[test]
    fn tombstone_failure_does_not_turn_a_verified_key_save_into_an_error() {
        let fake = FakeSecretStore::default();
        fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, LEGACY_MIGRATION_TOMBSTONE_ACCOUNT);
        let default = ServiceProfile::alibaba_default();
        let store = settings(&fake);

        store
            .save_api_key(DEFAULT_ALIBABA_PROFILE_ID, "sk-replacement")
            .unwrap();
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&default))
                .as_deref(),
            Some("sk-replacement")
        );
        assert_eq!(
            store.load_api_key().unwrap().as_deref(),
            Some("sk-replacement")
        );

        // A fresh process reads only the authoritative profile slot. Explicit
        // deletion remains fail-closed because it must persist the marker
        // before removing the key.
        let restarted = settings(&fake);
        assert_eq!(
            restarted.load_api_key().unwrap().as_deref(),
            Some("sk-replacement")
        );
        assert_eq!(
            fake.load_count(PROFILE_KEYCHAIN_SERVICE, LEGACY_MIGRATION_TOMBSTONE_ACCOUNT),
            1
        );
        assert_eq!(
            restarted
                .delete_api_key(DEFAULT_ALIBABA_PROFILE_ID)
                .unwrap_err(),
            CREDENTIAL_STORE_UNAVAILABLE
        );
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&default))
                .as_deref(),
            Some("sk-replacement")
        );
    }

    #[test]
    fn existing_profile_credential_does_not_read_migration_items() {
        let fake = FakeSecretStore::default();
        let default = ServiceProfile::alibaba_default();
        fake.put(
            PROFILE_KEYCHAIN_SERVICE,
            &credential_account(&default),
            "sk-profile",
        );
        fake.put(
            PROFILE_KEYCHAIN_SERVICE,
            LEGACY_MIGRATION_TOMBSTONE_ACCOUNT,
            LEGACY_MIGRATION_TOMBSTONE_VALUE,
        );
        fake.put(
            LEGACY_KEYCHAIN_SERVICE_V3,
            LEGACY_KEYCHAIN_ACCOUNT,
            "sk-legacy",
        );
        let store = settings(&fake);

        assert_eq!(store.credential_state(&default), CredentialState::Present);
        assert_eq!(store.load_api_key().unwrap().as_deref(), Some("sk-profile"));
        assert_eq!(
            fake.load_count(PROFILE_KEYCHAIN_SERVICE, &credential_account(&default)),
            1
        );
        assert_eq!(
            fake.load_count(PROFILE_KEYCHAIN_SERVICE, LEGACY_MIGRATION_TOMBSTONE_ACCOUNT),
            0
        );
        assert_eq!(
            fake.load_count(LEGACY_KEYCHAIN_SERVICE_V3, LEGACY_KEYCHAIN_ACCOUNT),
            0
        );
    }

    #[test]
    fn explicit_deletion_prevents_legacy_credential_resurrection() {
        let fake = FakeSecretStore::default();
        fake.put(
            LEGACY_KEYCHAIN_SERVICE_V3,
            LEGACY_KEYCHAIN_ACCOUNT,
            "sk-legacy",
        );
        let store = settings(&fake);
        assert_eq!(store.load_api_key().unwrap().as_deref(), Some("sk-legacy"));
        store.delete_api_key(DEFAULT_ALIBABA_PROFILE_ID).unwrap();

        assert_eq!(
            fake.value(LEGACY_KEYCHAIN_SERVICE_V3, LEGACY_KEYCHAIN_ACCOUNT),
            None
        );

        // A new process has an empty cache but sees the durable tombstone.
        let restarted = settings(&fake);
        assert_eq!(restarted.load_api_key().unwrap(), None);
        assert_eq!(
            fake.load_count(LEGACY_KEYCHAIN_SERVICE_V3, LEGACY_KEYCHAIN_ACCOUNT),
            1
        );
    }

    #[test]
    fn failed_legacy_cleanup_keeps_the_current_default_credential() {
        let fake = FakeSecretStore::default();
        fake.put(
            LEGACY_KEYCHAIN_SERVICE_V3,
            LEGACY_KEYCHAIN_ACCOUNT,
            "sk-legacy",
        );
        let store = settings(&fake);
        assert_eq!(store.load_api_key().unwrap().as_deref(), Some("sk-legacy"));
        fake.make_delete_unavailable(LEGACY_KEYCHAIN_SERVICE_V3, LEGACY_KEYCHAIN_ACCOUNT);

        assert_eq!(
            store
                .delete_api_key(DEFAULT_ALIBABA_PROFILE_ID)
                .unwrap_err(),
            CREDENTIAL_STORE_UNAVAILABLE
        );
        assert_eq!(store.load_api_key().unwrap().as_deref(), Some("sk-legacy"));
    }

    #[test]
    fn missing_legacy_credential_does_not_suppress_a_future_import() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        assert_eq!(store.load_api_key().unwrap(), None);
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, LEGACY_MIGRATION_TOMBSTONE_ACCOUNT)
                .as_deref(),
            None
        );

        fake.put(
            LEGACY_KEYCHAIN_SERVICE_V3,
            LEGACY_KEYCHAIN_ACCOUNT,
            "sk-created-by-older-version",
        );
        let restarted = settings(&fake);
        assert_eq!(
            restarted.load_api_key().unwrap().as_deref(),
            Some("sk-created-by-older-version")
        );
    }

    #[test]
    fn blank_legacy_slot_does_not_hide_a_later_valid_credential() {
        let fake = FakeSecretStore::default();
        fake.put(LEGACY_KEYCHAIN_SERVICE_V3, LEGACY_KEYCHAIN_ACCOUNT, "   ");
        fake.put(
            LEGACY_KEYCHAIN_SERVICE_V2,
            LEGACY_KEYCHAIN_ACCOUNT,
            "sk-valid",
        );
        let store = settings(&fake);

        assert_eq!(store.load_api_key().unwrap().as_deref(), Some("sk-valid"));
    }

    #[test]
    fn only_blank_legacy_values_fail_closed_without_a_tombstone() {
        let fake = FakeSecretStore::default();
        fake.put(LEGACY_KEYCHAIN_SERVICE_V3, LEGACY_KEYCHAIN_ACCOUNT, "   ");
        let store = settings(&fake);

        assert_eq!(
            store.load_api_key().unwrap_err(),
            CREDENTIAL_STORE_UNAVAILABLE
        );
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, LEGACY_MIGRATION_TOMBSTONE_ACCOUNT),
            None
        );
    }

    #[test]
    fn unavailable_credential_store_is_not_reported_as_missing() {
        let fake = FakeSecretStore::default();
        let default = ServiceProfile::alibaba_default();
        fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &credential_account(&default));
        let store = settings(&fake);

        assert_eq!(
            store.credential_state(&default),
            CredentialState::Unavailable
        );
        assert_eq!(
            store.load_api_key().unwrap_err(),
            CREDENTIAL_STORE_UNAVAILABLE
        );
        // All windows share the first failed read, avoiding repeated OS
        // authorization prompts for the same slot during one app launch.
        assert_eq!(
            fake.load_count(PROFILE_KEYCHAIN_SERVICE, &credential_account(&default)),
            1
        );
    }

    #[test]
    fn explicit_diagnostic_recovers_failed_storage_without_restart() {
        let fake = FakeSecretStore::default();
        let profile = ServiceProfile::alibaba_default();
        let account = credential_account(&profile);
        fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &account);
        let store = settings(&fake);
        assert_eq!(store.credential_diagnostic(&profile), "unavailable");
        fake.state.lock().unwrap().unavailable.clear();
        assert_eq!(store.credential_diagnostic(&profile), "missing");
        store
            .save_api_key(&profile.id, "synthetic-test-value")
            .unwrap();
        assert_eq!(store.credential_diagnostic(&profile), "present");
    }

    #[test]
    fn explicit_diagnostic_recovers_legacy_deeplx_destination_after_unlock() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = store
            .create_profile(ProviderKind::DeepLX, "Existing")
            .unwrap();
        let account = credential_account(&profile);
        let destination_account = SettingsStore::destination_account(&profile);
        let legacy = ProviderCredentials::DeepLX {
            asr_api_key: "synthetic-asr".into(),
            endpoint: "https://example.com/legacy".into(),
            token: "synthetic-legacy-token".into(),
        }
        .encode_for_keychain(ProviderKind::DeepLX)
        .unwrap();
        fake.put(PROFILE_KEYCHAIN_SERVICE, &account, &legacy);
        fake.put(
            PROFILE_KEYCHAIN_SERVICE,
            &destination_account,
            &serde_json::to_string(&TextTranslationDestination {
                endpoint: "https://example.com/translate".into(),
                token: "synthetic-custom-token".into(),
                deep_l_api_key: None,
                open_ai_compatible: None,
            })
            .unwrap(),
        );
        fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &destination_account);
        assert_eq!(
            store.credential_state(&profile),
            CredentialState::Unavailable
        );

        fake.state.lock().unwrap().unavailable.clear();
        // Normal reads keep the failure cached until the user explicitly retries.
        assert_eq!(
            store.credential_state(&profile),
            CredentialState::Unavailable
        );
        assert_eq!(
            fake.load_count(PROFILE_KEYCHAIN_SERVICE, &destination_account),
            1
        );
        assert_eq!(store.credential_diagnostic(&profile), "present");
        assert!(matches!(
            store.credentials_for_profile(&profile).unwrap(),
            Some(ProviderCredentials::DeepLX { asr_api_key, endpoint, token })
                if asr_api_key == "synthetic-asr"
                    && endpoint == "https://example.com/translate"
                    && token == "synthetic-custom-token"
        ));
        assert_eq!(
            fake.load_count(PROFILE_KEYCHAIN_SERVICE, &destination_account),
            2
        );
        assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, &account), 1);
    }

    #[test]
    fn diagnostic_does_not_retry_another_profiles_failed_slot() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let other = openai_profile(&store, "Synthetic other profile");
        let account = credential_account(&other);
        fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &account);
        assert_eq!(store.credential_state(&other), CredentialState::Unavailable);
        store.credential_diagnostic(&ServiceProfile::alibaba_default());
        assert_eq!(store.credential_state(&other), CredentialState::Unavailable);
        assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, &account), 1);
    }

    #[test]
    fn profile_crud_preserves_catalog_invariants() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let created = openai_profile(&store, "  Work  ");
        assert_eq!(created.name, "Work");
        store.update_profile(&created.id, "Production").unwrap();
        store.select_profile(&created.id).unwrap();
        assert_eq!(store.active_profile().unwrap().id, created.id);

        store.delete_profile(DEFAULT_ALIBABA_PROFILE_ID).unwrap();
        assert_eq!(store.profile_catalog().unwrap().1.len(), 1);
        assert_eq!(store.delete_profile(&created.id).unwrap_err(), LAST_PROFILE);
    }

    #[test]
    fn connection_check_configuration_reads_selected_profile_without_switching() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let active_id = store.active_profile().unwrap().id;
        let profile = openai_profile(&store, "Synthetic check profile");
        store
            .save_api_key(&profile.id, "fixture-check-only")
            .unwrap();
        {
            let mut prefs = store.prefs.lock().unwrap();
            prefs.source_language = SourceLanguage::Chinese;
            prefs.target_language = TargetLanguage::Original;
        }
        let before = store.preferences();
        assert!(store.configuration_for_profile(&profile).is_err());
        let configuration = store.configuration_for_profile_probe(&profile).unwrap();
        assert_eq!(configuration.provider, ProviderKind::OpenAIRealtime);
        assert_eq!(configuration.source_language, SourceLanguage::Automatic);
        assert!(configuration.target_language.translates_audio());
        assert_eq!(store.preferences(), before);
        assert_eq!(
            configuration.credentials.direct_api_key(),
            Some("fixture-check-only")
        );
        assert_eq!(store.active_profile().unwrap().id, active_id);
        assert_eq!(
            fake.load_count(
                PROFILE_KEYCHAIN_SERVICE,
                &credential_account(&ServiceProfile::alibaba_default())
            ),
            0
        );
    }

    #[test]
    fn deleting_a_profile_without_a_saved_credential_is_idempotent() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = openai_profile(&store, "No credential");

        store.delete_profile(&profile.id).unwrap();

        assert!(store
            .profile_catalog()
            .unwrap()
            .1
            .iter()
            .all(|candidate| candidate.id != profile.id));
    }

    #[test]
    fn structured_credentials_remain_keychain_only_and_resolve_configuration() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = store
            .create_profile(ProviderKind::AzureOpenAIRealtime, "Azure")
            .unwrap();
        let secret = "azure-private-value";
        let credentials = ProviderCredentials::AzureOpenAI {
            endpoint: "https://mimi.openai.azure.com".into(),
            deployment: "translate".into(),
            transcription_deployment: "transcribe".into(),
            api_key: secret.into(),
        };

        store.save_credentials(&profile.id, &credentials).unwrap();
        assert_eq!(store.credential_state(&profile), CredentialState::Present);
        store.select_profile(&profile.id).unwrap();
        let configuration = store.configuration().unwrap();
        assert_eq!(configuration.credentials, credentials);

        let catalog = serde_json::to_string(&store.profile_catalog().unwrap()).unwrap();
        assert!(!catalog.contains(secret));
        assert!(!catalog.contains("mimi.openai.azure.com"));
    }

    #[test]
    fn profile_count_is_bounded() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        for index in 1..MAXIMUM_PROFILE_COUNT {
            store
                .create_profile(ProviderKind::OpenAIRealtime, &format!("Profile {index}"))
                .unwrap();
        }
        assert!(store
            .create_profile(ProviderKind::OpenAIRealtime, "One too many")
            .is_err());
        assert_eq!(
            store.profile_catalog().unwrap().1.len(),
            MAXIMUM_PROFILE_COUNT
        );
    }

    #[test]
    fn failed_credential_delete_rolls_profile_metadata_back() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = openai_profile(&store, "Work");
        store.save_api_key(&profile.id, "sk-work").unwrap();
        let account = credential_account(&profile);
        fake.make_delete_unavailable(PROFILE_KEYCHAIN_SERVICE, &account);

        assert_eq!(
            store.delete_profile(&profile.id).unwrap_err(),
            CREDENTIAL_STORE_UNAVAILABLE
        );
        assert!(store
            .profile_catalog()
            .unwrap()
            .1
            .iter()
            .any(|candidate| candidate.id == profile.id));
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, &account).as_deref(),
            Some("sk-work")
        );
    }

    #[test]
    fn failed_catalog_commit_does_not_delete_a_profile_credential() {
        let fake = FakeSecretStore::default();
        let directory = std::env::temp_dir().join(format!(
            "mimi-profile-commit-failure-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let store = SettingsStore::at_path(directory.clone(), Box::new(fake.clone()));
        let profile = store
            .create_profile(ProviderKind::OpenAIRealtime, "Work")
            .unwrap();
        store.save_api_key(&profile.id, "sk-work").unwrap();
        let account = credential_account(&profile);

        let catalog_path = directory.join(PROFILE_CATALOG_FILE);
        std::fs::remove_file(&catalog_path).unwrap();
        std::fs::create_dir(&catalog_path).unwrap();

        assert_eq!(
            store.delete_profile(&profile.id).unwrap_err(),
            PROFILE_CATALOG_UNAVAILABLE
        );
        assert!(store
            .profile_catalog()
            .unwrap()
            .1
            .iter()
            .any(|candidate| candidate.id == profile.id));
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, &account).as_deref(),
            Some("sk-work")
        );

        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn catalog_is_atomic_json_and_contains_no_secret() {
        let fake = FakeSecretStore::default();
        let directory = std::env::temp_dir().join(format!(
            "mimi-profile-test-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let store = SettingsStore::at_path(directory.clone(), Box::new(fake));
        let profile = store
            .create_profile(ProviderKind::OpenAIRealtime, "Work")
            .unwrap();
        store
            .save_api_key(&profile.id, "super-secret-value")
            .unwrap();

        let catalog_path = directory.join(PROFILE_CATALOG_FILE);
        let text = std::fs::read_to_string(&catalog_path).unwrap();
        let catalog: ProfileCatalog = serde_json::from_str(&text).unwrap();
        assert_eq!(catalog.schema_version, PROFILE_CATALOG_SCHEMA_VERSION);
        assert!(!text.contains("super-secret-value"));
        assert!(!text.contains("apiKey"));
        assert!(std::fs::read_dir(&directory).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));

        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn failed_preference_write_leaves_memory_unchanged() {
        let fake = FakeSecretStore::default();
        let directory = std::env::temp_dir().join(format!(
            "mimi-preferences-failure-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let store = SettingsStore::at_path(directory.clone(), Box::new(fake));
        let before = store.preferences();

        std::fs::remove_file(directory.join(PROFILE_CATALOG_FILE)).unwrap();
        std::fs::remove_dir(&directory).unwrap();
        std::fs::write(&directory, "not a directory").unwrap();

        assert_eq!(
            store
                .save_preferences(|prefs| prefs.font_size = 20.0)
                .unwrap_err(),
            PREFERENCES_UNAVAILABLE
        );
        assert_eq!(store.preferences(), before);

        let _ = std::fs::remove_file(directory);
    }

    #[test]
    fn legacy_preferences_default_to_centered_card_presentation() {
        let preferences: Preferences = serde_json::from_str("{}").unwrap();

        assert_eq!(preferences.subtitle_color, SubtitleColor::White);
        assert_eq!(preferences.subtitle_alignment, SubtitleAlignment::Center);
        assert_eq!(
            preferences.subtitle_display_mode,
            SubtitleDisplayMode::Translation
        );
        assert!(!preferences.subtitle_blends_with_background);
        assert!(!preferences.show_subtitle_dividers);
        assert_eq!(preferences.pulse_style, PulseStyle::Ribbon);
    }

    #[test]
    fn pulse_style_migration_preserves_existing_motion_choices() {
        for json in [
            r#"{"font_size":19,"pulse_animation":false,"subtitle_animation":true}"#,
            r#"{"font_size":19,"pulse_animation":false,"subtitle_animation":true,"pulse_style":"classic"}"#,
            r#"{"font_size":19,"pulse_animation":false,"subtitle_animation":true,"pulse_style":"futureStyle"}"#,
        ] {
            let prefs: Preferences = serde_json::from_str(json).unwrap();
            assert_eq!(prefs.pulse_style, PulseStyle::Ribbon);
            assert_eq!(serde_json::to_value(prefs.pulse_style).unwrap(), "ribbon");
            assert_eq!(prefs.font_size, 19.0);
            assert_eq!(prefs.pulse_animation, Some(false));
            assert_eq!(prefs.subtitle_animation, Some(true));
        }
    }

    #[test]
    fn legacy_translation_modes_migrate_to_turbo_without_loading_credentials() {
        for mode in ["lowLatency", "highQuality", "turbo"] {
            let directory = tempfile::tempdir().unwrap();
            let fake = FakeSecretStore::default();
            std::fs::write(
                directory.path().join("preferences.json"),
                serde_json::to_vec(&serde_json::json!({
                    "translation_mode": mode,
                    "pulse_style": "classic",
                    "pulse_animation": false,
                    "font_size": 19
                }))
                .unwrap(),
            )
            .unwrap();
            let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
            let prefs = store.preferences();
            assert_eq!(prefs.translation_mode, TranslationMode::Turbo);
            assert_eq!(prefs.pulse_style, PulseStyle::Ribbon);
            assert_eq!(prefs.pulse_animation, Some(false));
            assert_eq!(prefs.font_size, 19.0);
            store
                .save_preferences(|prefs| prefs.translation_mode = TranslationMode::HighQuality)
                .unwrap();
            let persisted: Preferences = serde_json::from_slice(
                &std::fs::read(directory.path().join("preferences.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(persisted.translation_mode, TranslationMode::Turbo);
            assert_eq!(persisted.pulse_style, PulseStyle::Ribbon);
            assert!(fake.state.lock().unwrap().loads.is_empty());
        }
    }

    #[test]
    fn pulse_styles_survive_disk_reload_and_unrelated_updates_without_secret_access() {
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        for style in [PulseStyle::Syllable, PulseStyle::Ribbon] {
            let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
            store
                .save_preferences(|prefs| {
                    prefs.pulse_animation = Some(false);
                    prefs.subtitle_animation = Some(true);
                })
                .unwrap();
            store
                .save_preferences_for_active_profile(|prefs| prefs.pulse_style = style)
                .unwrap();
            store
                .save_preferences_for_active_profile(|prefs| prefs.font_size = 19.0)
                .unwrap();
            let restored = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
            let prefs = restored.preferences();
            assert_eq!(prefs.pulse_style, style);
            assert_eq!(prefs.font_size, 19.0);
            assert_eq!(prefs.pulse_animation, Some(false));
            assert_eq!(prefs.subtitle_animation, Some(true));
        }
        assert!(fake.state.lock().unwrap().loads.is_empty());
    }

    #[test]
    fn subtitle_colors_persist_without_changing_provider_configuration() {
        let directory = std::env::temp_dir().join(format!(
            "mimi-subtitle-color-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let fake = FakeSecretStore::default();
        let store = SettingsStore::at_path(directory.clone(), Box::new(fake.clone()));
        store
            .save_api_key(DEFAULT_ALIBABA_PROFILE_ID, "test-key")
            .unwrap();
        let original_configuration = store.configuration().unwrap();
        for color in [
            SubtitleColor::White,
            SubtitleColor::Teal,
            SubtitleColor::Yellow,
            SubtitleColor::Green,
            SubtitleColor::Pink,
            SubtitleColor::Custom([0x12, 0x34, 0x56]),
        ] {
            store
                .save_preferences_for_active_profile(|prefs| prefs.subtitle_color = color)
                .unwrap();
            assert_eq!(store.configuration().unwrap(), original_configuration);
            let reloaded = SettingsStore::at_path(directory.clone(), Box::new(fake.clone()));
            assert_eq!(reloaded.preferences().subtitle_color, color);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn display_modes_persist_without_changing_provider_configuration() {
        let directory = std::env::temp_dir().join(format!(
            "mimi-display-mode-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let fake = FakeSecretStore::default();
        let store = SettingsStore::at_path(directory.clone(), Box::new(fake.clone()));
        store
            .save_api_key(DEFAULT_ALIBABA_PROFILE_ID, "test-key")
            .unwrap();
        let original_configuration = store.configuration().unwrap();
        for mode in [
            SubtitleDisplayMode::Bilingual,
            SubtitleDisplayMode::Original,
            SubtitleDisplayMode::Translation,
        ] {
            store
                .save_preferences_for_active_profile(|prefs| prefs.subtitle_display_mode = mode)
                .unwrap();
            assert_eq!(store.configuration().unwrap(), original_configuration);
            let reloaded = SettingsStore::at_path(directory.clone(), Box::new(fake.clone()));
            assert_eq!(reloaded.preferences().subtitle_display_mode, mode);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn subtitle_dividers_persist_without_changing_provider_configuration() {
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        store
            .save_api_key(DEFAULT_ALIBABA_PROFILE_ID, "synthetic-asr")
            .unwrap();
        let original_configuration = store.configuration().unwrap();
        assert!(!store.preferences().show_subtitle_dividers);
        for enabled in [true, false] {
            store
                .save_preferences_for_active_profile(|prefs| prefs.show_subtitle_dividers = enabled)
                .unwrap();
            assert_eq!(store.configuration().unwrap(), original_configuration);
            let reloaded = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
            assert_eq!(reloaded.preferences().show_subtitle_dividers, enabled);
        }
    }

    #[test]
    fn network_proxy_persists_globally_and_probe_and_session_use_identical_configuration() {
        use crate::core::network_proxy::ProxyMode;
        let legacy: Preferences = serde_json::from_str(r#"{"ui_language":"ja"}"#).unwrap();
        assert_eq!(legacy.network_proxy, ProxyConfig::default());
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        store
            .save_api_key(DEFAULT_ALIBABA_PROFILE_ID, "synthetic-asr")
            .unwrap();
        let profile = store.active_profile().unwrap();
        let original = store.configuration().unwrap();
        for proxy in [
            ProxyConfig {
                mode: ProxyMode::Custom,
                url: Some("socks5h://127.0.0.1:1080".into()),
            },
            ProxyConfig {
                mode: ProxyMode::Direct,
                url: None,
            },
            ProxyConfig::default(),
        ] {
            store
                .save_preferences_for_active_profile(|prefs| prefs.network_proxy = proxy.clone())
                .unwrap();
            assert_eq!(store.configuration().unwrap().network_proxy, proxy);
            assert_eq!(
                store
                    .configuration_for_profile_probe(&profile)
                    .unwrap()
                    .network_proxy,
                proxy
            );
            let other = store
                .create_profile(ProviderKind::OpenAIRealtime, "Synthetic")
                .unwrap();
            store.select_profile(&other.id).unwrap();
            assert_eq!(store.preferences().network_proxy, proxy);
            store.select_profile(&profile.id).unwrap();
            let reloaded = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
            assert_eq!(reloaded.preferences().network_proxy, proxy);
        }
        assert_eq!(original.network_proxy, ProxyConfig::default());
    }

    #[test]
    fn invalid_proxy_save_rolls_back_memory_and_disk_without_storing_authentication() {
        use crate::core::network_proxy::ProxyMode;
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        store
            .save_preferences(|prefs| prefs.font_size = 19.0)
            .unwrap();
        let before = store.preferences();
        let result = store.save_preferences(|prefs| {
            prefs.font_size = 20.0;
            prefs.network_proxy = ProxyConfig {
                mode: ProxyMode::Custom,
                url: Some("http://synthetic-user:synthetic-password@127.0.0.1:8888".into()),
            };
        });
        assert!(result.is_err());
        assert_eq!(store.preferences(), before);
        let reloaded = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        assert_eq!(reloaded.preferences(), before);
        assert!(fake.state.lock().unwrap().values.is_empty());
    }

    #[test]
    fn loading_normalizes_preferences_for_the_active_provider() {
        let fake = FakeSecretStore::default();
        let directory = std::env::temp_dir().join(format!(
            "mimi-preferences-normalize-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&directory).unwrap();

        let openai = ServiceProfile::new("openai", "OpenAI", ProviderKind::OpenAIRealtime).unwrap();
        let catalog = ProfileCatalog {
            schema_version: PROFILE_CATALOG_SCHEMA_VERSION,
            active_profile_id: openai.id.clone(),
            profiles: vec![ServiceProfile::alibaba_default(), openai.clone()],
        };
        std::fs::write(
            directory.join(PROFILE_CATALOG_FILE),
            serde_json::to_vec_pretty(&catalog).unwrap(),
        )
        .unwrap();
        let stale = Preferences {
            source_language: SourceLanguage::Japanese,
            target_language: TargetLanguage::Original,
            translation_mode: TranslationMode::HighQuality,
            ..Preferences::default()
        };
        std::fs::write(
            directory.join("preferences.json"),
            serde_json::to_vec_pretty(&stale).unwrap(),
        )
        .unwrap();
        fake.put(
            PROFILE_KEYCHAIN_SERVICE,
            &credential_account(&openai),
            "sk-openai-test",
        );

        let store = SettingsStore::at_path(directory.clone(), Box::new(fake));
        let normalized = store.preferences();
        assert_eq!(normalized.source_language, SourceLanguage::Automatic);
        assert_eq!(
            normalized.target_language,
            TargetLanguage::SimplifiedChinese
        );
        assert_eq!(normalized.translation_mode, TranslationMode::Turbo);
        assert_eq!(
            store.configuration().unwrap().provider,
            ProviderKind::OpenAIRealtime
        );

        let persisted: Preferences =
            serde_json::from_slice(&std::fs::read(directory.join("preferences.json")).unwrap())
                .unwrap();
        assert_eq!(persisted, normalized);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn loading_preserves_explicit_chinese_translation_target() {
        let fake = FakeSecretStore::default();
        let directory = std::env::temp_dir().join(format!(
            "mimi-preferences-chinese-original-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let stale = Preferences {
            source_language: SourceLanguage::Chinese,
            target_language: TargetLanguage::English,
            ..Preferences::default()
        };
        std::fs::write(
            directory.join("preferences.json"),
            serde_json::to_vec_pretty(&stale).unwrap(),
        )
        .unwrap();

        let store = SettingsStore::at_path(directory.clone(), Box::new(fake));
        let normalized = store.preferences();
        assert_eq!(normalized.source_language, SourceLanguage::Chinese);
        assert_eq!(normalized.target_language, TargetLanguage::English);

        let persisted: Preferences =
            serde_json::from_slice(&std::fs::read(directory.join("preferences.json")).unwrap())
                .unwrap();
        assert_eq!(persisted, normalized);

        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn explicit_provider_preserves_chinese_translation_target() {
        let mut preferences = Preferences {
            source_language: SourceLanguage::Chinese,
            target_language: TargetLanguage::English,
            ..Preferences::default()
        };

        normalize_preferences_value(
            &mut preferences,
            &ServiceProfile::new("tencent", "Tencent", ProviderKind::TencentCloud).unwrap(),
        );
        assert_eq!(preferences.source_language, SourceLanguage::Chinese);
        assert_eq!(preferences.target_language, TargetLanguage::English);

        preferences.target_language = TargetLanguage::SimplifiedChinese;
        normalize_preferences_value(
            &mut preferences,
            &ServiceProfile::new("tencent", "Tencent", ProviderKind::TencentCloud).unwrap(),
        );
        assert_eq!(preferences.target_language, TargetLanguage::English);
    }

    #[test]
    fn failed_profile_selection_rolls_preferences_back() {
        let fake = FakeSecretStore::default();
        let directory = std::env::temp_dir().join(format!(
            "mimi-profile-selection-rollback-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let store = SettingsStore::at_path(directory.clone(), Box::new(fake));
        let openai = store
            .create_profile(ProviderKind::OpenAIRealtime, "OpenAI")
            .unwrap();
        store
            .save_preferences(|prefs| {
                prefs.source_language = SourceLanguage::Japanese;
                prefs.target_language = TargetLanguage::Original;
                prefs.translation_mode = TranslationMode::HighQuality;
            })
            .unwrap();
        let before = store.preferences();

        let catalog_path = directory.join(PROFILE_CATALOG_FILE);
        std::fs::remove_file(&catalog_path).unwrap();
        std::fs::create_dir(&catalog_path).unwrap();

        assert_eq!(
            store.select_profile(&openai.id).unwrap_err(),
            PROFILE_CATALOG_UNAVAILABLE
        );
        assert_eq!(store.preferences(), before);
        assert_eq!(
            store.active_profile().unwrap().id,
            DEFAULT_ALIBABA_PROFILE_ID
        );
        let persisted: Preferences =
            serde_json::from_slice(&std::fs::read(directory.join("preferences.json")).unwrap())
                .unwrap();
        assert_eq!(persisted, before);

        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn invalid_catalog_is_preserved_and_blocks_mutation() {
        let fake = FakeSecretStore::default();
        let directory = std::env::temp_dir().join(format!(
            "mimi-profile-invalid-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join(PROFILE_CATALOG_FILE);
        std::fs::write(&path, r#"{"schemaVersion":99,"profiles":[]}"#).unwrap();
        let store = SettingsStore::at_path(directory.clone(), Box::new(fake));

        assert_eq!(
            store
                .create_profile(ProviderKind::OpenAIRealtime, "Work")
                .unwrap_err(),
            PROFILE_CATALOG_UNAVAILABLE
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            r#"{"schemaVersion":99,"profiles":[]}"#
        );
        let fallback = crate::commands::SettingsSnapshotPayload::from_store(&store);
        assert!(fallback
            .profiles
            .iter()
            .all(|profile| profile.credential_state == CredentialState::Unavailable));
        let fallback_json = serde_json::to_value(fallback).unwrap();
        assert!(fallback_json.get("error").is_none());
        assert!(fallback_json.get("credentialError").is_none());

        let _ = std::fs::remove_dir_all(directory);
    }
}
