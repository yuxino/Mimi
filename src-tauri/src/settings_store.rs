//! Non-secret preferences, service-profile metadata, and local credentials.
//!
//! General preferences and the profile catalog are separate JSON documents.
//! API keys live in a separate private local file. OS stores are compatibility
//! readers for one-time upgrade import only. The dev preset file stays isolated.

mod file_credentials;
#[cfg(any(all(feature = "local-dev-credentials", target_os = "macos"), test))]
mod local_dev_credentials;

use crate::core::audio_input::AudioInput;
use crate::core::configuration::LiveTranslationConfiguration;
use crate::core::credentials::{
    CredentialRevealField, ProviderCredentials, TextTranslationCredentials,
};
use crate::core::models::{
    SourceLanguage, SubtitleColor, SubtitleDisplayMode, TargetLanguage, TranslationMode,
};
use crate::core::network_proxy::ProxyConfig;
use crate::core::provider::{
    CustomSpeechLanguagesPatch, ProviderKind, ProviderPreferences, ServiceProfile, TextTranslation,
    TextTranslationName, DEFAULT_ALIBABA_PROFILE_ID,
};
use crate::core::subtitle_font::normalize_family_name;
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
const LOCAL_DEV_ALIBABA_PROFILE_ID: &str = "alibaba-local-dev";
const LOCAL_DEV_GEMINI_PROFILE_ID: &str = "gemini-local-dev";

fn is_local_dev_profile_id(id: &str) -> bool {
    matches!(
        id,
        LOCAL_DEV_ALIBABA_PROFILE_ID | LOCAL_DEV_GEMINI_PROFILE_ID
    )
}
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

/// Editor-local configuration and field availability, never a settings snapshot.
/// Endpoints can include private path segments, so do not log this payload.
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialEditorState {
    pub saved_fields: Vec<CredentialRevealField>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcription_deployment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
}

impl CredentialEditorState {
    fn from_text_credentials(credentials: TextTranslationCredentials) -> Self {
        let mut state = Self::default();
        let token = match credentials {
            TextTranslationCredentials::Apple => return state,
            TextTranslationCredentials::DeepL { api_key } => api_key,
            TextTranslationCredentials::DeepLX { endpoint, token } => {
                state.endpoint = Some(endpoint);
                token
            }
            TextTranslationCredentials::OpenAICompatible {
                endpoint,
                model,
                api_key,
            }
            | TextTranslationCredentials::ChatMock {
                endpoint,
                model,
                api_key,
            } => {
                state.endpoint = Some(endpoint);
                state.model = Some(model);
                api_key
            }
        };
        if !token.is_empty() {
            state.saved_fields.push(CredentialRevealField::Token);
        }
        state
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TextTranslationDestination {
    endpoint: String,
    token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    deep_l_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    open_ai_compatible: Option<OpenAICompatibleDestination>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    chat_mock: Option<OpenAICompatibleDestination>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenAICompatibleDestination {
    endpoint: String,
    api_key: String,
    model: String,
}

impl TextTranslationDestination {
    fn credentials(&self, route: TextTranslation) -> Option<TextTranslationCredentials> {
        match route {
            TextTranslation::FollowService | TextTranslation::Apple => None,
            TextTranslation::DeepL => Some(TextTranslationCredentials::DeepL {
                api_key: self.deep_l_api_key.clone().unwrap_or_default(),
            }),
            TextTranslation::DeepLX => Some(TextTranslationCredentials::DeepLX {
                endpoint: self.endpoint.clone(),
                token: self.token.clone(),
            }),
            TextTranslation::OpenAICompatible => {
                let value = self.open_ai_compatible.clone().unwrap_or_default();
                Some(TextTranslationCredentials::OpenAICompatible {
                    endpoint: value.endpoint,
                    model: value.model,
                    api_key: value.api_key,
                })
            }
            TextTranslation::ChatMock => {
                let value = self.chat_mock.clone().unwrap_or_default();
                Some(TextTranslationCredentials::ChatMock {
                    endpoint: value.endpoint,
                    model: value.model,
                    api_key: value.api_key,
                })
            }
        }
    }

    fn set_credentials(&mut self, credentials: TextTranslationCredentials) {
        match credentials {
            TextTranslationCredentials::Apple => {}
            TextTranslationCredentials::DeepL { api_key } => self.deep_l_api_key = Some(api_key),
            TextTranslationCredentials::DeepLX { endpoint, token } => {
                self.endpoint = endpoint;
                self.token = token;
            }
            TextTranslationCredentials::OpenAICompatible {
                endpoint,
                model,
                api_key,
            } => {
                self.open_ai_compatible = Some(OpenAICompatibleDestination {
                    endpoint,
                    model,
                    api_key,
                });
            }
            TextTranslationCredentials::ChatMock {
                endpoint,
                model,
                api_key,
            } => {
                self.chat_mock = Some(OpenAICompatibleDestination {
                    endpoint,
                    model,
                    api_key,
                });
            }
        }
    }
}

pub const MAXIMUM_PROFILE_COUNT: usize = 20;
pub const FONT_SIZE_RANGE: std::ops::RangeInclusive<f64> = 14.0..=20.0;
pub const DEFAULT_FONT_SIZE: f64 = crate::core::overlay_layout::DEFAULT_FONT_SIZE;

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
    /// Empty follows the system font stack; otherwise one installed family name.
    pub subtitle_font_family: String,
    /// Background opacity in percent; independent of subtitle text.
    pub subtitle_background_opacity: u8,
    pub subtitle_color: SubtitleColor,
    pub microphone_subtitle_color: SubtitleColor,
    pub subtitle_alignment: SubtitleAlignment,
    pub subtitle_display_mode: SubtitleDisplayMode,
    pub show_intermediate_subtitles: bool,
    pub show_subtitle_dividers: bool,
    pub show_subtitle_timestamps: bool,
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
    /// Explicitly selected inputs; legacy preferences remain system-only.
    pub audio_input: AudioInput,
    /// Empty means follow the Windows default output, including live changes.
    pub windows_audio_source: String,
    pub system_audio_target: crate::core::system_audio_target::SystemAudioTarget,
    /// macOS Dock/Cmd-Tab presence. Missing preferences show Mimi in the Dock.
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
            subtitle_font_family: String::new(),
            subtitle_background_opacity: 80,
            subtitle_color: SubtitleColor::White,
            microphone_subtitle_color: SubtitleColor::Yellow,
            subtitle_alignment: SubtitleAlignment::Center,
            subtitle_display_mode: SubtitleDisplayMode::Translation,
            show_intermediate_subtitles: true,
            show_subtitle_dividers: false,
            show_subtitle_timestamps: false,
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
            audio_input: AudioInput::System,
            windows_audio_source: String::new(),
            system_audio_target: Default::default(),
            show_in_dock: true,
            network_proxy: ProxyConfig::default(),
        }
    }
}

impl Preferences {
    pub fn apply_system_audio_target(
        &mut self,
        target: crate::core::system_audio_target::SystemAudioTarget,
    ) {
        if self.system_audio_target != target {
            self.record_session_audio = false;
        }
        self.system_audio_target = target;
    }

    /// A recording opt-in belongs to the selected sources. Changing selection
    /// always requires a fresh opt-in, even in a combined settings draft.
    pub fn apply_audio_preferences(&mut self, input: Option<AudioInput>, recording: Option<bool>) {
        if let Some(recording) = recording {
            self.record_session_audio = recording;
        }
        if let Some(input) = input {
            if self.audio_input != input {
                self.record_session_audio = false;
            }
            self.audio_input = input;
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

impl CredentialState {
    pub fn combined(self, other: Self) -> Self {
        if self == Self::Unavailable || other == Self::Unavailable {
            Self::Unavailable
        } else if self == Self::Missing || other == Self::Missing {
            Self::Missing
        } else {
            Self::Present
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretStoreError {
    Unavailable,
    ReadOnly,
    MigrationRequired,
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
            Self::MigrationRequired => CREDENTIAL_STORE_UNAVAILABLE,
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

/// Keyed credentials. Production uses a private local file; the native adapter
/// exists only for one-time legacy import and focused compatibility tests.
pub trait SecretStore: Send + Sync {
    fn load(&self, service: &str, account: &str) -> Result<Option<String>, SecretStoreError>;
    /// Availability only; production reads the local credential document.
    fn contains(&self, service: &str, account: &str) -> Result<bool, SecretStoreError> {
        self.load(service, account).map(|value| value.is_some())
    }
    fn save(&self, service: &str, account: &str, value: &str) -> Result<(), SecretStoreError>;
    /// Normal Save is file-backed. The native override is retained only for
    /// legacy compatibility tests and must not be selected by production.
    fn save_from_user_action(
        &self,
        service: &str,
        account: &str,
        value: &str,
    ) -> Result<(), SecretStoreError> {
        self.save(service, account, value)
    }
    fn delete(&self, service: &str, account: &str) -> Result<(), SecretStoreError>;
    fn uses_local_file(&self) -> bool {
        false
    }
    fn pending_imports(&self) -> Result<usize, SecretStoreError> {
        Ok(0)
    }
    fn migrate_legacy(&self) -> Result<(), SecretStoreError> {
        Ok(())
    }
    fn is_read_only(&self) -> bool {
        false
    }
    fn local_dev_profile_ids(&self) -> Vec<&'static str> {
        Vec::new()
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

#[cfg(target_os = "linux")]
fn save_linux_password(entry: &keyring_core::Entry, value: &str) -> Result<(), SecretStoreError> {
    let error = match entry.set_password(value) {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    // The backend cannot create its default collection: an absent alias is
    // reported as NoStorageAccess(NoResult). Only this failed explicit save
    // may request collection creation, never a settings read or diagnostic.
    if !matches!(
        &error,
        keyring_core::Error::NoStorageAccess(source)
            if matches!(source.downcast_ref::<secret_service::Error>(), Some(secret_service::Error::NoResult))
    ) {
        return Err(keyring_operation_error(error));
    }
    let service =
        secret_service::blocking::SecretService::connect(secret_service::EncryptionType::Dh)
            .map_err(|_| SecretStoreError::ServiceUnavailable)?;
    match service.get_default_collection() {
        // NoResult can also arise elsewhere in the backend. Do not turn a
        // failed unlock/read into a new collection or a second prompt.
        Ok(_) => return Err(keyring_operation_error(error)),
        Err(secret_service::Error::NoResult) => {
            // The provider owns the native authorization/password prompt and
            // encrypted storage policy. Cancellation remains an access error.
            service
                .create_collection("Default", "default")
                .map_err(|error| {
                    keyring_operation_error(
                        zbus_secret_service_keyring_store::errors::decode_error(error),
                    )
                })?;
        }
        Err(error) => {
            return Err(keyring_operation_error(
                zbus_secret_service_keyring_store::errors::decode_error(error),
            ));
        }
    }
    entry.set_password(value).map_err(keyring_operation_error)
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
    #[cfg(target_os = "macos")]
    fn contains(&self, service: &str, account: &str) -> Result<bool, SecretStoreError> {
        use security_framework::item::{ItemClass, ItemSearchOptions};
        use security_framework::os::macos::keychain::{SecKeychain, SecPreferencesDomain};
        let keychain = SecKeychain::default_for_domain(SecPreferencesDomain::User)
            .map_err(|_| SecretStoreError::Unavailable)?;
        // Same User keychain and exact account as keyring's legacy backend.
        // Return attributes only; never password bytes or altered access rules.
        match ItemSearchOptions::new()
            .keychains(&[keychain])
            .class(ItemClass::generic_password())
            .service(service)
            .account(account)
            .load_attributes(true)
            .search()
        {
            Ok(items) => Ok(!items.is_empty()),
            Err(error) if error.code() == -25300 => Ok(false),
            Err(_) => Err(SecretStoreError::Unavailable),
        }
    }

    fn load(&self, service: &str, account: &str) -> Result<Option<String>, SecretStoreError> {
        let entry = credential_entry(service, account)?;
        match entry.get_password() {
            Ok(password) => Ok(Some(password)),
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

    fn save_from_user_action(
        &self,
        service: &str,
        account: &str,
        value: &str,
    ) -> Result<(), SecretStoreError> {
        #[cfg(target_os = "linux")]
        {
            save_linux_password(&credential_entry(service, account)?, value)
        }
        #[cfg(not(target_os = "linux"))]
        {
            self.save(service, account, value)
        }
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
            active_profile_id: String::new(),
            profiles: Vec::new(),
        }
    }
}

impl ProfileCatalog {
    /// Only pre-catalog installations and explicit test fixtures use this profile.
    fn legacy_alibaba() -> Self {
        Self {
            active_profile_id: DEFAULT_ALIBABA_PROFILE_ID.to_string(),
            profiles: vec![ServiceProfile::alibaba_default()],
            ..Self::default()
        }
    }

    fn with_local_dev_profiles(mut self, enabled: &[&str]) -> Self {
        let previous = self.profiles.clone();
        self.profiles.retain(|p| !is_local_dev_profile_id(&p.id));
        let mut presets = Vec::new();
        for (id, provider, name) in [
            (
                LOCAL_DEV_ALIBABA_PROFILE_ID,
                ProviderKind::AlibabaCloud,
                "Alibaba Cloud · dev",
            ),
            (
                LOCAL_DEV_GEMINI_PROFILE_ID,
                ProviderKind::GoogleGeminiLive,
                "Google Gemini · dev",
            ),
        ] {
            if !enabled.contains(&id) {
                continue;
            }
            let mut profile = previous
                .iter()
                .find(|p| p.id == id)
                .cloned()
                .unwrap_or_else(|| ServiceProfile {
                    id: id.into(),
                    name: name.into(),
                    ..ServiceProfile::alibaba_default()
                });
            profile.provider = provider;
            profile.text_translation = Some(TextTranslation::FollowService);
            if id == LOCAL_DEV_ALIBABA_PROFILE_ID
                && !previous.iter().any(|p| p.id == id)
                && self.active_profile_id == DEFAULT_ALIBABA_PROFILE_ID
            {
                self.active_profile_id = LOCAL_DEV_ALIBABA_PROFILE_ID.into();
            }
            presets.push(profile);
        }
        presets.append(&mut self.profiles);
        self.profiles = presets;
        if !self.profiles.iter().any(|p| p.id == self.active_profile_id) {
            self.active_profile_id = self
                .profiles
                .first()
                .map(|p| p.id.clone())
                .unwrap_or_default();
        }
        self
    }

    fn validated(self) -> Result<Self, ()> {
        if self.schema_version != PROFILE_CATALOG_SCHEMA_VERSION
            || self
                .profiles
                .iter()
                .filter(|p| !is_local_dev_profile_id(&p.id))
                .count()
                > MAXIMUM_PROFILE_COUNT
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
        if (profiles.is_empty() && !self.active_profile_id.is_empty())
            || (!profiles.is_empty() && !ids.contains(&self.active_profile_id))
        {
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
                false,
            );
        }
        let secret: Box<dyn SecretStore> =
            Box::new(file_credentials::FileCredentialStore::for_app(
                &app_config_dir,
                profile_keychain_service,
                !is_development,
            ));
        #[cfg(any(all(feature = "local-dev-credentials", target_os = "macos"), test))]
        let secret =
            local_dev_credentials::select(&app_config_dir, is_ui_test, application_identifier)
                .unwrap_or(secret);
        // One-time upgrade work. Each completed slot is persisted before the
        // next native read. Steady-state windows use only the local file.
        if secret.pending_imports().unwrap_or(0) > 0 && secret.migrate_legacy().is_err() {
            tracing::warn!("credential import incomplete label=legacy_import_pending");
        }
        let is_file_mode = secret.uses_local_file()
            || secret.is_read_only()
            || !secret.local_dev_profile_ids().is_empty();
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
            let store = Self::in_memory_with_scope(secret, true, profile_keychain_service, false);
            *store.catalog.lock().unwrap() = ProfileCatalog::default();
            return store;
        }
        let prefs_path = app_config_dir.join("preferences.json");
        let prefs = std::fs::read_to_string(&prefs_path)
            .ok()
            .and_then(|text| serde_json::from_str::<Preferences>(&text).ok())
            .unwrap_or_default();
        let font_size = prefs
            .font_size
            .clamp(*FONT_SIZE_RANGE.start(), *FONT_SIZE_RANGE.end());
        let prefs = Preferences {
            font_size,
            subtitle_font_family: normalize_family_name(&prefs.subtitle_font_family)
                .unwrap_or_default()
                .to_owned(),
            subtitle_background_opacity: prefs.subtitle_background_opacity.min(100),
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

        let normalized = catalog
            .clone()
            .with_local_dev_profiles(&secret.local_dev_profile_ids());
        let should_create_catalog = should_create_catalog || normalized != catalog;
        let store = Self {
            prefs_path,
            prefs: Mutex::new(prefs),
            catalog_path,
            catalog: Mutex::new(normalized),
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
        Self::in_memory_with_scope(secret, is_ui_test, PROFILE_KEYCHAIN_SERVICE, true)
    }

    fn in_memory_with_scope(
        secret: Box<dyn SecretStore>,
        is_ui_test: bool,
        profile_keychain_service: &'static str,
        migrate_legacy_alibaba: bool,
    ) -> Self {
        let catalog = ProfileCatalog::legacy_alibaba().with_local_dev_profiles(&if is_ui_test {
            Vec::new()
        } else {
            secret.local_dev_profile_ids()
        });
        Self {
            prefs_path: PathBuf::new(),
            prefs: Mutex::new(Preferences::default()),
            catalog_path: PathBuf::new(),
            catalog: Mutex::new(catalog),
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
        *store.prefs.lock().unwrap() = Preferences {
            font_size,
            subtitle_font_family: normalize_family_name(&prefs.subtitle_font_family)
                .unwrap_or_default()
                .to_owned(),
            subtitle_background_opacity: prefs.subtitle_background_opacity.min(100),
            ..prefs
        };
        Ok(store)
    }

    #[cfg(test)]
    fn at_path(app_config_dir: PathBuf, secret: Box<dyn SecretStore>) -> Self {
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
        next.subtitle_background_opacity = next.subtitle_background_opacity.min(100);
        next.subtitle_font_family = normalize_family_name(&next.subtitle_font_family)?.to_owned();
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
        let (active_id, profiles) = self.profile_catalog()?;
        let profile = profiles.into_iter().find(|profile| profile.id == active_id);
        self.save_preferences_validated(update, |previous, next| {
            let Some(profile) = &profile else { return Ok(()); };
            let capabilities = profile.capabilities(next.target_language).for_source(next.source_language);
            if next.target_language != previous.target_language
                && !capabilities.target_languages.contains(&next.target_language) {
                return Err(crate::core::configuration::LiveTranslationConfigurationError::UnsupportedTargetLanguage.to_string());
            }
            if next.source_language != previous.source_language
                && !capabilities.source_languages.contains(&next.source_language) {
                return Err(crate::core::configuration::LiveTranslationConfigurationError::UnsupportedSourceLanguage.to_string());
            }
            normalize_preferences_value(next, profile);
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
        let profile = ServiceProfile::new(
            format!("profile-{}", uuid::Uuid::new_v4().simple()),
            name,
            provider,
        )
        .map_err(|error| error.to_string())?;
        self.mutate_catalog_with_normalization(Some(&profile.id), |catalog| {
            if catalog
                .profiles
                .iter()
                .filter(|p| !is_local_dev_profile_id(&p.id))
                .count()
                >= MAXIMUM_PROFILE_COUNT
            {
                return Err("No more service profiles can be added.".to_string());
            }
            if catalog.profiles.is_empty() {
                catalog.active_profile_id = profile.id.clone();
            }
            catalog.profiles.push(profile.clone());
            Ok(profile.clone())
        })
    }

    #[cfg(test)]
    pub fn update_profile(&self, profile_id: &str, name: &str) -> Result<ServiceProfile, String> {
        self.update_profile_options(profile_id, Some(name), None, None, None, None, None, None)
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)] // Test helper for existing metadata patches.
    pub fn update_profile_options(
        &self,
        profile_id: &str,
        name: Option<&str>,
        speech_network_proxy: Option<ProxyConfig>,
        text_network_proxy: Option<ProxyConfig>,
        text_translation_name: Option<TextTranslationName>,
        speech_recognition_name: Option<&str>,
        custom_speech_languages_patch: Option<CustomSpeechLanguagesPatch>,
        language_preset_patch: Option<crate::core::provider::ProfileLanguagePresetPatch>,
    ) -> Result<ServiceProfile, String> {
        self.update_profile_options_with_model(
            profile_id,
            name,
            speech_network_proxy,
            text_network_proxy,
            text_translation_name,
            speech_recognition_name,
            custom_speech_languages_patch,
            language_preset_patch,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_profile_options_with_model(
        &self,
        profile_id: &str,
        name: Option<&str>,
        speech_network_proxy: Option<ProxyConfig>,
        text_network_proxy: Option<ProxyConfig>,
        text_translation_name: Option<TextTranslationName>,
        speech_recognition_name: Option<&str>,
        custom_speech_languages_patch: Option<CustomSpeechLanguagesPatch>,
        language_preset_patch: Option<crate::core::provider::ProfileLanguagePresetPatch>,
        qwen_mt_model: Option<crate::core::protocols::qwen_mt::QwenMTModel>,
    ) -> Result<ServiceProfile, String> {
        if text_translation_name.is_some()
            || speech_recognition_name.is_some()
            || language_preset_patch.is_some()
        {
            self.require_writable_credentials(profile_id)?;
        }
        let normalize_profile = custom_speech_languages_patch.as_ref().map(|_| profile_id);
        self.mutate_catalog_with_normalization(normalize_profile, |catalog| {
            let current = catalog
                .profiles
                .iter_mut()
                .find(|profile| profile.id == profile_id)
                .ok_or_else(|| PROFILE_NOT_FOUND.to_string())?;
            let mut updated = current.clone();
            if let Some(model) = qwen_mt_model {
                if current.provider != ProviderKind::AlibabaCloud {
                    return Err("provider-mismatch".into());
                }
                updated.qwen_mt_model = model;
            }
            if let Some(name) = name {
                updated.name = name.trim().to_string();
            }
            if let Some(proxy) = speech_network_proxy {
                updated.speech_network_proxy = Some(proxy);
            }
            if let Some(proxy) = text_network_proxy {
                updated.text_network_proxy = Some(proxy);
            }
            if let Some(patch) = text_translation_name {
                updated
                    .set_text_translation_name(patch.route, &patch.name)
                    .map_err(|error| error.to_string())?;
            }
            if let Some(patch) = custom_speech_languages_patch {
                updated
                    .set_custom_speech_source_languages(patch.languages)
                    .map_err(|error| error.to_string())?;
            }
            if let Some(name) = speech_recognition_name {
                updated
                    .set_speech_recognition_name(name)
                    .map_err(|error| error.to_string())?;
            }
            if let Some(patch) = language_preset_patch {
                if let Some(preset) = patch.preset {
                    updated.validate_language_preset(preset)?;
                }
                updated.language_preset = patch.preset;
            }
            let updated = updated.validated().map_err(|error| error.to_string())?;
            *current = updated.clone();
            Ok(updated)
        })
    }

    pub fn select_profile(&self, profile_id: &str) -> Result<(), String> {
        self.select_profile_with_source(profile_id, None)
    }

    /// Commit profile and an explicitly selected Apple language together. Runtime
    /// language-resource validation belongs to the guarded session mutation.
    pub fn select_profile_with_source(
        &self,
        profile_id: &str,
        source_language: Option<SourceLanguage>,
    ) -> Result<(), String> {
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
        let next_prefs =
            profile_selection_preferences(previous_prefs.clone(), &profile, source_language)?;

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
        if self.profile_uses_local_dev_credentials(profile_id) {
            return Err(SecretStoreError::ReadOnly.public_error());
        }
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
                .map(|value| self.save_api_key_for_profile(&profile, value, false))
                .transpose()
                .is_ok();
            if self.persist_preferences_value(&previous_prefs).is_err() {
                tracing::warn!("preferences unavailable label=profile_delete_rollback_failed");
            }
            let destination_restored = previous_destination
                .as_deref()
                .map(|value| self.write_destination_value(&profile, Some(value), false))
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
                .map(|value| self.save_api_key_for_profile(&profile, value, false))
                .transpose()
                .is_ok();
            if self.persist_preferences_value(&previous_prefs).is_err() {
                tracing::warn!("preferences unavailable label=profile_delete_rollback_failed");
            }
            let destination_restored = previous_destination
                .as_deref()
                .map(|value| self.write_destination_value(&profile, Some(value), false))
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
        self.retry_profile_credential_errors(profile, true, true);
        if let Some((speech, text)) = self.custom_credential_states(profile) {
            return match speech.combined(text) {
                CredentialState::Present => "present",
                CredentialState::Missing => "missing",
                CredentialState::Unavailable => "unavailable",
            };
        }
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

    fn retry_profile_credential_errors(&self, profile: &ServiceProfile, speech: bool, text: bool) {
        let speech = speech && profile.provider != ProviderKind::AppleSpeech;
        let account = credential_account(profile);
        let retry_legacy = speech && is_default_alibaba(profile) && self.migrate_legacy_alibaba;
        self.secret_cache
            .lock()
            .unwrap()
            .retain(|(service, slot), result| {
                let selected = service == self.profile_keychain_service
                    && ((speech && slot == &account)
                        || (text
                            && profile.provider.supports_text_translation()
                            && slot == &Self::destination_account(profile)));
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
    }

    pub fn credential_state(&self, profile: &ServiceProfile) -> CredentialState {
        if let Some((speech, text)) = self.custom_credential_states(profile) {
            return speech.combined(text);
        }
        match self.credentials_for_profile(profile) {
            Ok(Some(_)) => CredentialState::Present,
            Ok(None) => CredentialState::Missing,
            Err(_) => CredentialState::Unavailable,
        }
    }

    /// Settings show saved-item presence; selection does not authorize secrets.
    pub fn credential_state_for_snapshot(&self, profile: &ServiceProfile) -> CredentialState {
        if self.is_ui_test || (!self.secret.uses_local_file() && !cfg!(target_os = "macos")) {
            return self.credential_state(profile);
        }
        if let Some((speech, text)) = self.custom_credential_states_for_snapshot(profile) {
            return speech.combined(text);
        }
        self.speech_presence_for_snapshot(profile)
    }

    pub fn custom_credential_states_for_snapshot(
        &self,
        profile: &ServiceProfile,
    ) -> Option<(CredentialState, CredentialState)> {
        if !profile.provider.is_standalone_asr() {
            return None;
        }
        if self.is_ui_test || (!self.secret.uses_local_file() && !cfg!(target_os = "macos")) {
            return self.custom_credential_states(profile);
        }
        let speech = self.speech_presence_for_snapshot(profile);
        let text = if matches!(
            profile.text_translation(),
            TextTranslation::FollowService | TextTranslation::Apple
        ) {
            CredentialState::Present
        } else if self.secret.is_read_only() {
            CredentialState::Missing
        } else {
            Self::presence_state(self.secret_present(&Self::destination_account(profile)))
        };
        Some((speech, text))
    }

    fn speech_presence_for_snapshot(&self, profile: &ServiceProfile) -> CredentialState {
        if profile.provider == ProviderKind::AppleSpeech {
            return CredentialState::Present;
        }
        let presence = (|| {
            if self.secret_present(&credential_account(profile))? {
                return Ok(true);
            }
            // Discovery is metadata-only; actual legacy migration stays on use.
            if is_default_alibaba(profile)
                && self.migrate_legacy_alibaba
                && !self.secret_present(LEGACY_MIGRATION_TOMBSTONE_ACCOUNT)?
            {
                for service in [
                    LEGACY_KEYCHAIN_SERVICE_V3,
                    LEGACY_KEYCHAIN_SERVICE_V2,
                    LEGACY_KEYCHAIN_SERVICE,
                ] {
                    if self.secret.contains(service, LEGACY_KEYCHAIN_ACCOUNT)? {
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        })();
        Self::presence_state(presence)
    }

    fn secret_present(&self, account: &str) -> Result<bool, SecretStoreError> {
        let cached = self
            .secret_cache
            .lock()
            .unwrap()
            .get(&cache_key(self.profile_keychain_service, account))
            .cloned();
        match cached {
            Some(Ok(Some(value))) if value.trim().is_empty() => Err(SecretStoreError::Unavailable),
            Some(result) => result.map(|value| value.is_some()),
            None => self.secret.contains(self.profile_keychain_service, account),
        }
    }

    fn presence_state(presence: Result<bool, SecretStoreError>) -> CredentialState {
        match presence {
            Ok(true) => CredentialState::Present,
            Ok(false) => CredentialState::Missing,
            Err(_) => CredentialState::Unavailable,
        }
    }

    /// Resolves each custom pipeline slot once; public states never contain keys.
    pub fn custom_credential_states(
        &self,
        profile: &ServiceProfile,
    ) -> Option<(CredentialState, CredentialState)> {
        if !profile.provider.is_standalone_asr() {
            return None;
        }
        let speech = match self.credentials_for_profile(profile) {
            Ok(Some(_)) => CredentialState::Present,
            Ok(None) => CredentialState::Missing,
            Err(_) => CredentialState::Unavailable,
        };
        let text = if matches!(
            profile.text_translation(),
            TextTranslation::FollowService | TextTranslation::Apple
        ) {
            CredentialState::Present
        } else if self.secret.is_read_only() {
            CredentialState::Missing
        } else {
            match self.text_credentials_for_profile(profile) {
                Ok(Some(_)) => CredentialState::Present,
                Ok(None) => CredentialState::Missing,
                Err(_) => CredentialState::Unavailable,
            }
        };
        Some((speech, text))
    }

    /// Public source only; never exposes a file path or secret. This is not a
    /// persisted preference and cannot enable file mode from the frontend.
    pub fn credential_storage(&self) -> &'static str {
        if self.secret.is_read_only() || !self.secret.local_dev_profile_ids().is_empty() {
            "localDevFile"
        } else if self.secret.uses_local_file() {
            "localFile"
        } else {
            "keychain"
        }
    }

    pub fn profile_uses_local_dev_credentials(&self, profile_id: &str) -> bool {
        !self.is_ui_test && self.secret.local_dev_profile_ids().contains(&profile_id)
    }

    pub fn profile_credential_storage(&self, profile_id: &str) -> &'static str {
        if self.secret.is_read_only() || self.profile_uses_local_dev_credentials(profile_id) {
            "localDevFile"
        } else if self.secret.uses_local_file() {
            "localFile"
        } else {
            "keychain"
        }
    }

    #[cfg(test)]
    fn migrate_credentials(&self) -> Result<(), String> {
        if self.is_ui_test {
            return Ok(());
        }
        let result = self
            .secret
            .migrate_legacy()
            .map_err(SecretStoreError::public_error);
        self.secret_cache.lock().unwrap().clear();
        result
    }

    /// Hydrates only the selected editor's configuration and saved-field flags.
    /// API keys remain native until a separate explicit reveal request.
    pub fn credential_editor_state(
        &self,
        profile_id: &str,
        text_translation: Option<TextTranslation>,
    ) -> Result<CredentialEditorState, String> {
        self.require_writable_credentials(profile_id)?;
        let profile = self.profile(profile_id)?;
        if text_translation == Some(TextTranslation::Apple) {
            return Ok(CredentialEditorState::default());
        }
        if let Some(route) = text_translation {
            if !CredentialRevealField::Token.allowed_for(&profile, Some(route)) {
                return Err("credential_reveal_field_mismatch".into());
            }
            if let Some(value) = self.destination_value(&profile)? {
                let destination: TextTranslationDestination = serde_json::from_str(&value)
                    .map_err(|_| CREDENTIAL_STORE_UNAVAILABLE.to_string())?;
                let credentials = destination
                    .credentials(route)
                    .ok_or_else(|| CREDENTIAL_STORE_UNAVAILABLE.to_string())?
                    .validated()
                    .map_err(|_| CREDENTIAL_STORE_UNAVAILABLE.to_string())?;
                return Ok(CredentialEditorState::from_text_credentials(credentials));
            }
            // Historical DeepLX profiles may keep both slots in one record.
            // Other providers must never borrow their speech credentials here.
            if profile.provider != ProviderKind::DeepLX {
                return Ok(CredentialEditorState::default());
            }
        }
        let Some(value) = self
            .load_api_key_for_profile(&profile)
            .map_err(SecretStoreError::public_error)?
        else {
            return Ok(CredentialEditorState::default());
        };
        let credentials = ProviderCredentials::decode_for_profile(&profile, &value)
            .map_err(|_| CREDENTIAL_STORE_UNAVAILABLE.to_string())?;
        if let Some(route) = text_translation {
            let text = match (route, credentials) {
                (
                    TextTranslation::DeepLX,
                    ProviderCredentials::DeepLX {
                        endpoint, token, ..
                    },
                ) => Some(TextTranslationCredentials::DeepLX { endpoint, token }),
                (TextTranslation::DeepL, ProviderCredentials::DeepL { api_key, .. }) => {
                    Some(TextTranslationCredentials::DeepL { api_key })
                }
                _ => None,
            };
            return Ok(text.map_or_else(
                CredentialEditorState::default,
                CredentialEditorState::from_text_credentials,
            ));
        }
        let mut state = CredentialEditorState::default();
        for field in [
            CredentialRevealField::ApiKey,
            CredentialRevealField::AsrApiKey,
            CredentialRevealField::SecretId,
            CredentialRevealField::SecretKey,
            CredentialRevealField::AppKey,
        ] {
            if field.allowed_for(&profile, None)
                && credentials
                    .revealed_field(&profile, field, None)
                    .map_err(|_| CREDENTIAL_STORE_UNAVAILABLE.to_string())?
                    .is_some()
            {
                state.saved_fields.push(field);
            }
        }
        match credentials {
            ProviderCredentials::CustomSpeech {
                endpoint, model, ..
            } => {
                state.endpoint = Some(endpoint);
                state.model = Some(model);
            }
            ProviderCredentials::AzureOpenAI {
                endpoint,
                deployment,
                transcription_deployment,
                ..
            } => {
                state.endpoint = Some(endpoint);
                state.deployment = Some(deployment);
                state.transcription_deployment = Some(transcription_deployment);
            }
            ProviderCredentials::TencentCloud { app_id, .. }
            | ProviderCredentials::BaiduTranslate { app_id, .. } => {
                state.app_id = Some(app_id);
            }
            _ => {}
        }
        Ok(state)
    }

    /// Explicit settings-window action only. Uses the existing profile-scoped
    /// private store/cache path and never changes preferences or emits a snapshot.
    pub fn reveal_credential(
        &self,
        profile_id: &str,
        field: CredentialRevealField,
        text_translation: Option<TextTranslation>,
    ) -> Result<Option<String>, String> {
        self.require_writable_credentials(profile_id)?;
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
                    TextTranslation::ChatMock => {
                        let destination = destination.chat_mock.unwrap_or_default();
                        ProviderCredentials::ChatMock {
                            asr_api_key: String::new(),
                            endpoint: destination.endpoint,
                            api_key: destination.api_key,
                            model: destination.model,
                        }
                    }
                    TextTranslation::FollowService | TextTranslation::Apple => unreachable!(),
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
        self.require_writable_credentials(profile_id)?;
        let profile = self.profile(profile_id)?;
        if let ProviderCredentials::CustomSpeech {
            endpoint,
            model,
            api_key,
        } = credentials
        {
            return self.save_custom_speech(&profile, endpoint, model, api_key);
        }
        if let ProviderCredentials::AlibabaTranslation {
            api_key,
            text_translation,
            endpoint,
            token,
            model,
            clear_token,
        } = credentials
        {
            if *clear_token
                && (!text_translation.uses_chat_completions() || !token.trim().is_empty())
            {
                return Err(
                    crate::core::credentials::ProviderCredentialsError::InvalidField.to_string(),
                );
            }
            return self.save_text_translation(
                &profile,
                api_key,
                *text_translation,
                endpoint,
                (!token.trim().is_empty() || *clear_token).then_some(token.as_str()),
                model,
            );
        }
        if let ProviderCredentials::OpenAICompatible {
            asr_api_key,
            endpoint,
            api_key,
            model,
        }
        | ProviderCredentials::ChatMock {
            asr_api_key,
            endpoint,
            api_key,
            model,
        } = credentials
        {
            let translation = if matches!(credentials, ProviderCredentials::ChatMock { .. }) {
                TextTranslation::ChatMock
            } else {
                TextTranslation::OpenAICompatible
            };
            return self.save_text_translation(
                &profile,
                asr_api_key,
                translation,
                endpoint,
                (!api_key.trim().is_empty()).then_some(api_key.as_str()),
                model,
            );
        }
        let value = credentials
            .encode_for_keychain(profile.provider)
            .map_err(|error| error.to_string())?;
        self.save_api_key_for_profile(&profile, &value, true)
    }

    fn destination_account(profile: &ServiceProfile) -> String {
        format!(
            "provider-profile:{}:{}:text-translation",
            profile.id,
            profile.provider.wire_value()
        )
    }

    fn destination_value(&self, profile: &ServiceProfile) -> Result<Option<String>, String> {
        if !profile.provider.supports_text_translation() {
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
        allow_collection_creation: bool,
    ) -> Result<(), String> {
        let account = Self::destination_account(profile);
        match value {
            Some(value) => {
                self.save_secret(
                    self.profile_keychain_service,
                    &account,
                    value,
                    allow_collection_creation,
                )
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
        if profile.provider == ProviderKind::AppleSpeech {
            return Ok(Some(ProviderCredentials::AppleSpeech));
        }
        if self.secret.is_read_only()
            && (profile.provider.is_custom_speech()
                || profile.text_translation() != TextTranslation::FollowService)
        {
            // The local file supplies an Alibaba key only, not an independent
            // MT key/endpoint. Never borrow an OS-store destination instead.
            return Ok(None);
        }
        if self.profile_uses_local_dev_credentials(&profile.id)
            && (!matches!(
                (profile.id.as_str(), profile.provider),
                (LOCAL_DEV_ALIBABA_PROFILE_ID, ProviderKind::AlibabaCloud)
                    | (LOCAL_DEV_GEMINI_PROFILE_ID, ProviderKind::GoogleGeminiLive)
            ) || profile.text_translation() != TextTranslation::FollowService)
        {
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
        ) && !matches!(
            profile.text_translation(),
            TextTranslation::FollowService | TextTranslation::Apple
        ) {
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
                TextTranslation::ChatMock => {
                    let destination = destination.chat_mock.unwrap_or_default();
                    ProviderCredentials::ChatMock {
                        asr_api_key,
                        endpoint: destination.endpoint,
                        api_key: destination.api_key,
                        model: destination.model,
                    }
                }
                TextTranslation::FollowService | TextTranslation::Apple => unreachable!(),
            };
            return credentials
                .validated_for(profile.effective_provider())
                .map(Some)
                .map_err(|error| error.to_string());
        }
        Ok(Some(credentials))
    }

    fn text_credentials_for_profile(
        &self,
        profile: &ServiceProfile,
    ) -> Result<Option<TextTranslationCredentials>, String> {
        if profile.text_translation() == TextTranslation::Apple {
            return Ok(Some(TextTranslationCredentials::Apple));
        }
        if self.secret.is_read_only()
            || profile.text_translation() == TextTranslation::FollowService
        {
            return Ok(None);
        }
        let Some(value) = self.destination_value(profile)? else {
            return Ok(None);
        };
        let destination: TextTranslationDestination =
            serde_json::from_str(&value).map_err(|_| {
                crate::core::credentials::ProviderCredentialsError::InvalidStoredValue.to_string()
            })?;
        destination
            .credentials(profile.text_translation())
            .map(|credentials| credentials.validated())
            .transpose()
            .map_err(|error| error.to_string())
    }

    fn save_custom_speech(
        &self,
        profile: &ServiceProfile,
        endpoint: &str,
        model: &str,
        api_key: &str,
    ) -> Result<(), String> {
        use crate::core::credentials::ProviderCredentialsError as Error;
        if !profile.provider.is_custom_speech() {
            return Err(Error::ProviderMismatch.to_string());
        }
        if self.catalog_write_blocked {
            return Err(PROFILE_CATALOG_UNAVAILABLE.into());
        }
        let entered_endpoint = if endpoint.trim().is_empty() {
            None
        } else {
            Some(
                crate::core::protocols::custom_speech::endpoint(endpoint, profile.provider)
                    .map_err(|_| Error::InvalidCustomSpeechEndpoint.to_string())?
                    .to_string(),
            )
        };
        let entered_model = if model.trim().is_empty() {
            None
        } else {
            Some(
                crate::core::protocols::custom_speech::validate_model(model)
                    .map_err(|_| Error::InvalidCustomSpeechModel.to_string())?,
            )
        };
        self.retry_profile_credential_errors(profile, true, false);
        let previous = self
            .load_api_key_for_profile(profile)
            .map_err(SecretStoreError::public_error)?;
        let previous_credentials = previous
            .as_deref()
            .map(|value| ProviderCredentials::decode_for_profile(profile, value))
            .transpose()
            .map_err(|error| error.to_string())?;
        let value = ProviderCredentials::CustomSpeech {
            endpoint: entered_endpoint.unwrap_or_default(),
            model: entered_model.unwrap_or_default(),
            api_key: api_key.into(),
        }
        .resolve_draft(profile.provider, previous_credentials.as_ref())
        .and_then(|credentials| credentials.encode_for_keychain(profile.provider))
        .map_err(|error| error.to_string())?;
        if previous.as_deref() == Some(value.as_str()) {
            return Ok(());
        }
        if let Err(error) = self.save_api_key_for_profile(profile, &value, true) {
            let restored = match previous {
                Some(value) => self.save_api_key_for_profile(profile, &value, false),
                None => self.delete_api_key_for_profile(profile),
            };
            if restored.is_err() {
                tracing::warn!("credential update rollback failed label=custom_speech");
            }
            return Err(error);
        }
        Ok(())
    }

    fn save_custom_text_translation(
        &self,
        profile: &ServiceProfile,
        translation: TextTranslation,
        endpoint: &str,
        token_update: Option<&str>,
        model: &str,
    ) -> Result<(), String> {
        use crate::core::credentials::ProviderCredentialsError as Error;
        if self.catalog_write_blocked {
            return Err(PROFILE_CATALOG_UNAVAILABLE.into());
        }
        let entered_endpoint = if endpoint.trim().is_empty() {
            None
        } else if translation.uses_chat_completions() {
            Some(
                crate::core::protocols::openai_compatible::endpoint(endpoint)
                    .map_err(|_| Error::InvalidOpenAICompatibleEndpoint.to_string())?
                    .to_string(),
            )
        } else if translation == TextTranslation::DeepLX {
            Some(
                crate::core::protocols::deeplx::endpoint(endpoint)
                    .map_err(|_| Error::InvalidDeepLXEndpoint.to_string())?
                    .to_string(),
            )
        } else {
            None
        };
        let entered_model = if translation.uses_chat_completions() && !model.trim().is_empty() {
            Some(
                crate::core::protocols::openai_compatible::validate_model(model)
                    .map_err(|_| Error::InvalidOpenAICompatibleModel.to_string())?,
            )
        } else {
            None
        };
        self.retry_profile_credential_errors(profile, false, true);
        let previous_destination = self.destination_value(profile)?;
        let mut destination = previous_destination
            .as_deref()
            .map(serde_json::from_str::<TextTranslationDestination>)
            .transpose()
            .map_err(|_| Error::InvalidStoredValue.to_string())?;
        if translation != TextTranslation::FollowService {
            let value = destination.get_or_insert_with(TextTranslationDestination::default);
            let credentials = TextTranslationCredentials::resolve_update(
                translation,
                value.credentials(translation).as_ref(),
                entered_endpoint.as_deref().unwrap_or_default(),
                token_update,
                entered_model.as_deref().unwrap_or_default(),
            )
            .map_err(|error| error.to_string())?;
            value.set_credentials(credentials);
        }
        let destination_value = destination
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|_| CREDENTIAL_STORE_UNAVAILABLE.to_string())?;
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
        let write_result = (|| {
            if prefs_changed {
                self.persist_preferences_value(&next_prefs)?;
            }
            if destination_changed {
                self.write_destination_value(profile, destination_value.as_deref(), true)?;
            }
            self.persist_catalog_value(&next)
        })();
        if let Err(error) = write_result {
            if prefs_changed && self.persist_preferences_value(&previous_prefs).is_err() {
                tracing::warn!("preferences unavailable label=custom_text_route_rollback_failed");
            }
            if destination_changed
                && self
                    .write_destination_value(profile, previous_destination.as_deref(), false)
                    .is_err()
            {
                tracing::warn!("credential update rollback failed label=custom_text_destination");
            }
            return Err(error);
        }
        *prefs = next_prefs;
        *catalog = next;
        Ok(())
    }

    /// Apple has no text secret. Only an explicitly supplied speech key may
    /// access its own slot; switching text routes leaves all other slots intact.
    fn save_apple_text_translation(
        &self,
        profile: &ServiceProfile,
        api_key: &str,
    ) -> Result<(), String> {
        use crate::core::credentials::ProviderCredentialsError as Error;
        if !profile.provider.supports_text_translation()
            || (profile.provider.is_standalone_asr() && !api_key.trim().is_empty())
        {
            return Err(Error::ProviderMismatch.to_string());
        }
        if self.catalog_write_blocked {
            return Err(PROFILE_CATALOG_UNAVAILABLE.into());
        }
        let speech_update = if api_key.trim().is_empty() {
            None
        } else {
            self.retry_profile_credential_errors(profile, true, false);
            let previous = self
                .load_api_key_for_profile(profile)
                .map_err(SecretStoreError::public_error)?;
            let key = ProviderCredentials::api_key(api_key)
                .encode_for_keychain(ProviderKind::AlibabaCloud)
                .map_err(|error| error.to_string())?;
            // Legacy DeepLX profiles may hold either historical combined
            // destination. Change only their speech key; Alibaba uses a raw key.
            let updated = match previous
                .as_deref()
                .map(|value| ProviderCredentials::decode_for_profile(profile, value))
                .transpose()
                .map_err(|error| error.to_string())?
            {
                Some(ProviderCredentials::DeepLX {
                    endpoint, token, ..
                }) => ProviderCredentials::DeepLX {
                    asr_api_key: key,
                    endpoint,
                    token,
                }
                .encode_for_keychain(ProviderKind::DeepLX),
                Some(ProviderCredentials::DeepL { api_key, .. }) => ProviderCredentials::DeepL {
                    asr_api_key: key,
                    api_key,
                }
                .encode_for_keychain(ProviderKind::AlibabaCloud),
                _ => Ok(key),
            }
            .map_err(|error| error.to_string())?;
            Some((previous, updated))
        };
        let mut catalog = self.catalog.lock().unwrap();
        let mut next = catalog.clone();
        let updated = next
            .profiles
            .iter_mut()
            .find(|candidate| candidate.id == profile.id)
            .ok_or_else(|| PROFILE_NOT_FOUND.to_string())?;
        updated.text_translation = Some(TextTranslation::Apple);
        let mut prefs = self.prefs.lock().unwrap();
        let previous_prefs = prefs.clone();
        let mut next_prefs = previous_prefs.clone();
        if next.active_profile_id == profile.id {
            normalize_preferences_value(&mut next_prefs, updated);
        }
        let prefs_changed = next_prefs != previous_prefs;
        let result = (|| {
            if prefs_changed {
                self.persist_preferences_value(&next_prefs)?;
            }
            if let Some((previous, value)) = &speech_update {
                if previous.as_deref() != Some(value) {
                    self.save_api_key_for_profile(profile, value, true)?;
                }
            }
            self.persist_catalog_value(&next)
        })();
        if let Err(error) = result {
            if prefs_changed && self.persist_preferences_value(&previous_prefs).is_err() {
                tracing::warn!("preferences unavailable label=apple_text_route_rollback_failed");
            }
            if let Some((previous, _)) = &speech_update {
                let restored = match previous {
                    Some(previous) => self.save_api_key_for_profile(profile, previous, false),
                    None => self.delete_api_key_for_profile(profile),
                };
                if restored.is_err() {
                    tracing::warn!("credential rollback failed label=apple_text_speech");
                }
            }
            return Err(error);
        }
        *prefs = next_prefs;
        *catalog = next;
        Ok(())
    }

    fn save_text_translation(
        &self,
        profile: &ServiceProfile,
        api_key: &str,
        translation: TextTranslation,
        endpoint: &str,
        token_update: Option<&str>,
        model: &str,
    ) -> Result<(), String> {
        if translation == TextTranslation::Apple {
            if !endpoint.trim().is_empty()
                || token_update.is_some_and(|token| !token.trim().is_empty())
                || !model.trim().is_empty()
            {
                return Err(
                    crate::core::credentials::ProviderCredentialsError::InvalidField.to_string(),
                );
            }
            return self.save_apple_text_translation(profile, api_key);
        }
        if profile.provider.is_standalone_asr() {
            if !api_key.is_empty() {
                return Err("custom_speech_text_update_contains_speech_key".into());
            }
            return self.save_custom_text_translation(
                profile,
                translation,
                endpoint,
                token_update,
                model,
            );
        }
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
        if translation.uses_chat_completions() {
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
        self.retry_profile_credential_errors(profile, true, true);
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
                            chat_mock: None,
                        }),
                        ProviderCredentials::DeepL { api_key, .. } => {
                            Some(TextTranslationDestination {
                                endpoint: String::new(),
                                token: String::new(),
                                deep_l_api_key: Some(api_key.clone()),
                                open_ai_compatible: None,
                                chat_mock: None,
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
        if translation != TextTranslation::FollowService {
            let value = destination.get_or_insert_with(TextTranslationDestination::default);
            let credentials = TextTranslationCredentials::resolve_update(
                translation,
                value.credentials(translation).as_ref(),
                endpoint,
                token_update,
                model,
            )
            .map_err(|error| error.to_string())?;
            value.set_credentials(credentials);
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
                self.write_destination_value(profile, destination_value.as_deref(), true)?;
            }
            if key_changed {
                self.save_api_key_for_profile(profile, &key_value, true)?;
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
                    .write_destination_value(profile, previous_destination.as_deref(), false)
                    .is_err()
            {
                tracing::warn!("credential update rollback failed label=text_destination");
            }
            if key_changed {
                let rollback = match previous {
                    Some(previous) => self.save_api_key_for_profile(profile, &previous, false),
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
        self.require_writable_credentials(&profile.id)?;
        let destination = self.destination_value(profile)?;
        if destination.is_some() {
            self.write_destination_value(profile, None, false)?;
        }
        if let Err(error) = self.delete_api_key_for_profile(profile) {
            if let Some(destination) = destination {
                if self
                    .write_destination_value(profile, Some(&destination), false)
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

    /// Reads only the requested profile's private credentials. A connection
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

    /// Validates a language switch without saving it or reloading speech credentials.
    /// A recognition-only configuration omits independent text credentials, so
    /// restore the selected route directly rather than probing the old target.
    pub fn configuration_for_target_switch(
        &self,
        profile: &ServiceProfile,
        mut configuration: LiveTranslationConfiguration,
        selection: ProviderPreferences,
    ) -> Result<LiveTranslationConfiguration, String> {
        configuration.source_language = selection.source_language;
        configuration.target_language = selection.target_language;
        configuration.translation_mode = selection.translation_mode;
        if (configuration.provider.is_standalone_asr()
            || profile.text_translation() == TextTranslation::Apple)
            && selection.target_language.translates_audio()
            && configuration.text_credentials.is_none()
        {
            configuration.text_credentials = self.text_credentials_for_profile(profile)?;
        }
        configuration.validated().map_err(|error| error.to_string())
    }

    /// Resolves the exact listening configuration that selecting this profile
    /// would save, without selecting it. Unlike a connection probe, Original
    /// remains recognition-only and does not require text credentials.
    pub fn configuration_for_profile_selection(
        &self,
        profile: &ServiceProfile,
    ) -> Result<LiveTranslationConfiguration, String> {
        self.configuration_for_profile_selection_with_source(profile, None)
    }

    pub fn preferences_for_profile_selection(
        &self,
        profile: &ServiceProfile,
        source_language: Option<SourceLanguage>,
    ) -> Result<Preferences, String> {
        profile_selection_preferences(self.preferences(), profile, source_language)
    }

    pub fn configuration_for_profile_selection_with_source(
        &self,
        profile: &ServiceProfile,
        source_language: Option<SourceLanguage>,
    ) -> Result<LiveTranslationConfiguration, String> {
        let prefs = self.preferences_for_profile_selection(profile, source_language)?;
        self.configuration_with_profile_preferences(profile, prefs)
    }

    /// Build an ephemeral speech/integrated-service check. Complete drafts do
    /// not need saved credentials; partial drafts resolve only the speech slot.
    pub fn configuration_for_speech_draft_probe(
        &self,
        profile: &ServiceProfile,
        draft: &ProviderCredentials,
        speech_only: bool,
    ) -> Result<LiveTranslationConfiguration, String> {
        use crate::core::credentials::ProviderCredentialsError as Error;
        self.require_writable_credentials(&profile.id)?;
        if !speech_only && profile.provider.supports_text_translation() {
            return Err("draft_connection_check_stage_required".into());
        }
        let provider = if profile.provider.supports_text_translation()
            && !profile.provider.is_standalone_asr()
        {
            ProviderKind::AlibabaCloud
        } else {
            profile.provider
        };
        let draft = match draft {
            ProviderCredentials::AlibabaTranslation { api_key, .. }
                if provider == ProviderKind::AlibabaCloud =>
            {
                ProviderCredentials::api_key(api_key)
            }
            ProviderCredentials::AlibabaTranslation { .. } => {
                return Err(Error::ProviderMismatch.to_string())
            }
            other => other.clone(),
        };
        let credentials = match draft.validated_for(provider) {
            Ok(complete) => complete,
            Err(_) => {
                self.retry_profile_credential_errors(profile, true, false);
                let saved = self
                    .load_api_key_for_profile(profile)
                    .map_err(SecretStoreError::public_error)?
                    .map(|value| ProviderCredentials::decode_for_profile(profile, &value))
                    .transpose()
                    .map_err(|error| error.to_string())?;
                draft
                    .resolve_draft(provider, saved.as_ref())
                    .map_err(|error| error.to_string())?
            }
        };
        let prefs = self.preferences();
        let target_language = if speech_only && profile.provider.supports_text_translation() {
            TargetLanguage::Original
        } else {
            prefs.target_language
        };
        let capabilities = if provider.is_custom_speech() {
            profile.capabilities(TargetLanguage::Original)
        } else {
            provider.capabilities()
        };
        let normalized = capabilities.normalize(ProviderPreferences {
            source_language: prefs.source_language,
            target_language,
            translation_mode: prefs.translation_mode,
        });
        LiveTranslationConfiguration::with_credentials(
            provider,
            credentials,
            normalized.source_language,
            normalized.target_language,
            normalized.translation_mode,
        )
        .with_network_proxy(
            profile
                .speech_network_proxy
                .clone()
                .unwrap_or(prefs.network_proxy),
        )
        .validated()
        .map_err(|error| error.to_string())
    }

    /// Draft text checks read only the requested route's saved destination.
    /// No profile selection, preferences, catalog or credential data are written.
    pub fn configuration_for_text_draft_probe(
        &self,
        profile: &ServiceProfile,
        draft: &ProviderCredentials,
    ) -> Result<crate::core::configuration::TextTranslationProbeConfiguration, String> {
        use crate::core::configuration::TextTranslationProbeCredentials;
        use crate::core::credentials::ProviderCredentialsError as Error;
        self.require_writable_credentials(&profile.id)?;
        if !profile.provider.supports_text_translation() {
            return Err(Error::ProviderMismatch.to_string());
        }
        let (route, endpoint, token_update, model, speech_key) = match draft {
            ProviderCredentials::AlibabaTranslation {
                api_key,
                text_translation,
                endpoint,
                token,
                model,
                clear_token,
            } => {
                if *clear_token
                    && (!text_translation.uses_chat_completions() || !token.trim().is_empty())
                {
                    return Err(Error::InvalidField.to_string());
                }
                (
                    *text_translation,
                    endpoint.as_str(),
                    (!token.trim().is_empty() || *clear_token).then_some(token.as_str()),
                    model.as_str(),
                    api_key.as_str(),
                )
            }
            ProviderCredentials::OpenAICompatible {
                asr_api_key,
                endpoint,
                api_key,
                model,
            } => (
                TextTranslation::OpenAICompatible,
                endpoint.as_str(),
                (!api_key.trim().is_empty()).then_some(api_key.as_str()),
                model.as_str(),
                asr_api_key.as_str(),
            ),
            ProviderCredentials::ChatMock {
                asr_api_key,
                endpoint,
                api_key,
                model,
            } => (
                TextTranslation::ChatMock,
                endpoint.as_str(),
                (!api_key.trim().is_empty()).then_some(api_key.as_str()),
                model.as_str(),
                asr_api_key.as_str(),
            ),
            _ => return Err(Error::ProviderMismatch.to_string()),
        };
        let credentials = if route == TextTranslation::Apple {
            if !endpoint.trim().is_empty()
                || token_update.is_some_and(|token| !token.trim().is_empty())
                || !model.trim().is_empty()
            {
                return Err(Error::InvalidField.to_string());
            }
            TextTranslationProbeCredentials::Independent(TextTranslationCredentials::Apple)
        } else if route == TextTranslation::FollowService {
            if profile.provider.is_standalone_asr() {
                return Err("text_translation_not_configured".into());
            }
            let speech = self.configuration_for_speech_draft_probe(
                profile,
                &ProviderCredentials::api_key(speech_key),
                true,
            )?;
            TextTranslationProbeCredentials::Qwen {
                api_key: speech.credentials.alibaba_key().unwrap_or_default().into(),
            }
        } else {
            // Validate supplied addresses/models before any saved-value access.
            if !endpoint.trim().is_empty() {
                if route.uses_chat_completions() {
                    crate::core::protocols::openai_compatible::endpoint(endpoint)
                        .map_err(|_| Error::InvalidOpenAICompatibleEndpoint.to_string())?;
                } else if route == TextTranslation::DeepLX {
                    crate::core::protocols::deeplx::endpoint(endpoint)
                        .map_err(|_| Error::InvalidDeepLXEndpoint.to_string())?;
                }
            }
            if route.uses_chat_completions() && !model.trim().is_empty() {
                crate::core::protocols::openai_compatible::validate_model(model)
                    .map_err(|_| Error::InvalidOpenAICompatibleModel.to_string())?;
            }
            let complete = token_update.and_then(|_| {
                TextTranslationCredentials::resolve_update(
                    route,
                    None,
                    endpoint,
                    token_update,
                    model,
                )
                .ok()
            });
            let merged = if let Some(complete) = complete {
                complete
            } else {
                self.retry_profile_credential_errors(profile, false, true);
                let destination = self
                    .destination_value(profile)?
                    .map(|value| serde_json::from_str::<TextTranslationDestination>(&value))
                    .transpose()
                    .map_err(|_| Error::InvalidStoredValue.to_string())?;
                let saved = destination.and_then(|value| value.credentials(route));
                TextTranslationCredentials::resolve_update(
                    route,
                    saved.as_ref(),
                    endpoint,
                    token_update,
                    model,
                )
                .map_err(|error| error.to_string())?
            };
            TextTranslationProbeCredentials::Independent(merged)
        };
        let mut temporary = profile.clone();
        temporary.text_translation = Some(route);
        self.text_probe_with_credentials(&temporary, credentials)
    }

    /// Only the speech slot is read. Missing or inaccessible MT credentials
    /// must not prevent an explicit recognition setup check.
    pub fn configuration_for_speech_probe(
        &self,
        profile: &ServiceProfile,
    ) -> Result<LiveTranslationConfiguration, String> {
        self.configuration_for_speech_probe_with_source(profile, None)
    }

    /// The language visible in the Apple editor can differ from the active
    /// profile. Apply it before validation, without changing either profile.
    pub fn configuration_for_speech_probe_with_source(
        &self,
        profile: &ServiceProfile,
        source_language: Option<SourceLanguage>,
    ) -> Result<LiveTranslationConfiguration, String> {
        if source_language.is_some() && profile.provider != ProviderKind::AppleSpeech {
            return Err("apple_speech_source_override_invalid".into());
        }
        if profile.provider == ProviderKind::AppleSpeech {
            if source_language == Some(SourceLanguage::Automatic) {
                return Err("apple_speech_language_unsupported".into());
            }
            let prefs = self.preferences();
            let normalized =
                profile
                    .capabilities(TargetLanguage::Original)
                    .normalize(ProviderPreferences {
                        source_language: prefs.source_language,
                        target_language: TargetLanguage::Original,
                        translation_mode: prefs.translation_mode,
                    });
            return LiveTranslationConfiguration::with_credentials(
                ProviderKind::AppleSpeech,
                ProviderCredentials::AppleSpeech,
                source_language.unwrap_or(normalized.source_language),
                TargetLanguage::Original,
                normalized.translation_mode,
            )
            .validated()
            .map_err(|error| error.to_string());
        }
        self.retry_profile_credential_errors(profile, true, false);
        if !profile.provider.supports_text_translation() {
            return self.configuration_for_profile_probe(profile);
        }
        if self.secret.is_read_only() && profile.provider.is_custom_speech() {
            return Err("custom_speech_credentials_missing".into());
        }
        let value = self
            .load_api_key_for_profile(profile)
            .map_err(SecretStoreError::public_error)?
            .ok_or("custom_speech_credentials_missing")?;
        let decoded = ProviderCredentials::decode_for_profile(profile, &value)
            .map_err(|error| error.to_string())?;
        let provider = if profile.provider.is_custom_speech() {
            profile.provider
        } else {
            ProviderKind::AlibabaCloud
        };
        let credentials = if provider.is_custom_speech() {
            decoded
        } else {
            ProviderCredentials::api_key(decoded.alibaba_key().unwrap_or_default())
        };
        let prefs = self.preferences();
        let capabilities = if provider.is_custom_speech() {
            profile.capabilities(TargetLanguage::Original)
        } else {
            provider.capabilities()
        };
        let normalized = capabilities.normalize(ProviderPreferences {
            source_language: prefs.source_language,
            target_language: TargetLanguage::Original,
            translation_mode: prefs.translation_mode,
        });
        LiveTranslationConfiguration::with_credentials(
            provider,
            credentials,
            normalized.source_language,
            TargetLanguage::Original,
            normalized.translation_mode,
        )
        .with_network_proxy(
            profile
                .speech_network_proxy
                .clone()
                .unwrap_or(prefs.network_proxy),
        )
        .validated()
        .map_err(|error| error.to_string())
    }

    /// Resolves just the saved translation destination. The recognizer may be
    /// unconfigured; its slot is never read for an independent text check.
    pub fn configuration_for_text_probe(
        &self,
        profile: &ServiceProfile,
    ) -> Result<crate::core::configuration::TextTranslationProbeConfiguration, String> {
        use crate::core::configuration::TextTranslationProbeCredentials;
        if !profile.provider.supports_text_translation()
            || (profile.provider.is_standalone_asr()
                && profile.text_translation() == TextTranslation::FollowService)
        {
            return Err("text_translation_not_configured".into());
        }
        if profile.text_translation() == TextTranslation::Apple {
            return self.text_probe_with_credentials(
                profile,
                TextTranslationProbeCredentials::Independent(TextTranslationCredentials::Apple),
            );
        }
        self.retry_profile_credential_errors(
            profile,
            profile.text_translation() == TextTranslation::FollowService,
            true,
        );
        let credentials = if profile.text_translation() == TextTranslation::FollowService {
            let value = self
                .load_api_key_for_profile(profile)
                .map_err(SecretStoreError::public_error)?
                .ok_or("text_translation_credentials_missing")?;
            let decoded = ProviderCredentials::decode_for_profile(profile, &value)
                .map_err(|error| error.to_string())?;
            TextTranslationProbeCredentials::Qwen {
                api_key: decoded.alibaba_key().unwrap_or_default().into(),
            }
        } else {
            let credentials = match self.text_credentials_for_profile(profile)? {
                Some(credentials) => credentials,
                None if profile.provider == ProviderKind::DeepLX && !self.secret.is_read_only() => {
                    // Legacy DeepLX profiles may keep both slots in one item.
                    // Do not use this path for any new/custom speech profile.
                    self.retry_profile_credential_errors(profile, true, false);
                    match self.credentials_for_profile(profile)? {
                        Some(ProviderCredentials::DeepLX {
                            endpoint, token, ..
                        }) => TextTranslationCredentials::DeepLX { endpoint, token },
                        Some(ProviderCredentials::DeepL { api_key, .. }) => {
                            TextTranslationCredentials::DeepL { api_key }
                        }
                        _ => return Err("text_translation_credentials_missing".into()),
                    }
                }
                None => return Err("text_translation_credentials_missing".into()),
            };
            TextTranslationProbeCredentials::Independent(credentials)
        };
        self.text_probe_with_credentials(profile, credentials)
    }

    fn text_probe_with_credentials(
        &self,
        profile: &ServiceProfile,
        credentials: crate::core::configuration::TextTranslationProbeCredentials,
    ) -> Result<crate::core::configuration::TextTranslationProbeConfiguration, String> {
        use crate::core::configuration::TextTranslationProbeConfiguration;
        let prefs = self.preferences();
        let native = matches!(
            credentials,
            crate::core::configuration::TextTranslationProbeCredentials::Independent(
                TextTranslationCredentials::Apple
            )
        );
        if native
            && (prefs.source_language == SourceLanguage::Automatic
                || prefs.target_language == TargetLanguage::Original)
        {
            return Err("apple_translation_language_unsupported".into());
        }
        let target_language = if prefs.target_language == TargetLanguage::Original {
            TargetLanguage::SimplifiedChinese
        } else {
            profile
                .capabilities(prefs.target_language)
                .normalize(ProviderPreferences {
                    source_language: SourceLanguage::Automatic,
                    target_language: prefs.target_language,
                    translation_mode: TranslationMode::Turbo,
                })
                .target_language
        };
        Ok(TextTranslationProbeConfiguration {
            credentials,
            qwen_mt_model: profile.qwen_mt_model,
            source_language: prefs.source_language,
            target_language,
            network_proxy: if native {
                ProxyConfig {
                    mode: crate::core::network_proxy::ProxyMode::Direct,
                    url: None,
                }
            } else {
                profile
                    .text_network_proxy
                    .as_ref()
                    .unwrap_or(&prefs.network_proxy)
                    .validate()
                    .map_err(|error| error.to_string())?
            },
        })
    }

    fn configuration_for_profile_options(
        &self,
        profile: &ServiceProfile,
        for_probe: bool,
    ) -> Result<LiveTranslationConfiguration, String> {
        let mut prefs = self.prefs.lock().unwrap().clone();
        if for_probe {
            // Explicit checks verify a configured text destination even when
            // listening is recognition-only. This local choice never persists.
            if profile.provider.is_standalone_asr()
                && !matches!(
                    profile.text_translation(),
                    TextTranslation::FollowService | TextTranslation::Apple
                )
                && prefs.target_language == TargetLanguage::Original
            {
                prefs.target_language = TargetLanguage::English;
            }
            let normalized = profile.normalize_preferences(ProviderPreferences {
                source_language: prefs.source_language,
                target_language: prefs.target_language,
                translation_mode: prefs.translation_mode,
            });
            if profile.text_translation() != TextTranslation::Apple {
                prefs.source_language = normalized.source_language;
            }
            prefs.target_language = normalized.target_language;
            prefs.translation_mode = normalized.translation_mode;
        }
        self.configuration_with_profile_preferences(profile, prefs)
    }

    fn configuration_with_profile_preferences(
        &self,
        profile: &ServiceProfile,
        prefs: Preferences,
    ) -> Result<LiveTranslationConfiguration, String> {
        if profile.text_translation() == TextTranslation::Apple
            && prefs.target_language.translates_audio()
            && prefs.source_language == SourceLanguage::Automatic
        {
            return Err("apple_translation_language_unsupported".into());
        }
        if profile.custom_speech_source_languages.is_some()
            && !profile
                .capabilities(prefs.target_language)
                .source_languages
                .contains(&prefs.source_language)
        {
            return Err(crate::core::configuration::LiveTranslationConfigurationError::UnsupportedSourceLanguage.to_string());
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
                TextTranslation::DeepL
                    | TextTranslation::OpenAICompatible
                    | TextTranslation::ChatMock
            ) {
            ProviderCredentials::api_key(credentials.alibaba_key().unwrap_or_default())
        } else {
            credentials
        };
        let mut configuration = LiveTranslationConfiguration::with_credentials(
            provider,
            credentials,
            prefs.source_language,
            prefs.target_language,
            prefs.translation_mode,
        )
        .with_qwen_mt_model(profile.qwen_mt_model)
        .with_stage_network_proxies(
            profile
                .speech_network_proxy
                .clone()
                .unwrap_or_else(|| prefs.network_proxy.clone()),
            profile
                .text_network_proxy
                .clone()
                .unwrap_or(prefs.network_proxy),
        );
        if profile.text_translation() == TextTranslation::Apple
            || (provider.is_standalone_asr() && prefs.target_language.translates_audio())
        {
            let text_credentials =
                self.text_credentials_for_profile(profile)?.ok_or_else(|| {
                    crate::core::credentials::ProviderCredentialsError::MissingTextTranslation
                        .to_string()
                })?;
            configuration = configuration.with_text_credentials(text_credentials);
        }
        configuration.validated().map_err(|error| error.to_string())
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

    fn mutate_catalog_with_normalization<T>(
        &self,
        normalize_profile: Option<&str>,
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
        if normalize_profile == Some(next.active_profile_id.as_str()) {
            let profile = next
                .profiles
                .iter()
                .find(|profile| profile.id == next.active_profile_id)
                .ok_or_else(|| PROFILE_NOT_FOUND.to_string())?;
            let mut prefs = self.prefs.lock().unwrap();
            let previous = prefs.clone();
            let mut normalized = previous.clone();
            normalize_preferences_value(&mut normalized, profile);
            self.persist_preferences_value(&normalized)?;
            if let Err(error) = self.persist_catalog_value(&next) {
                if self.persist_preferences_value(&previous).is_err() {
                    tracing::warn!("preferences unavailable label=profile_options_rollback_failed");
                }
                return Err(error);
            }
            *prefs = normalized;
        } else {
            self.persist_catalog_value(&next)?;
        }
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
        if profile.provider == ProviderKind::AppleSpeech {
            return Ok(None);
        }
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

            self.save_secret(self.profile_keychain_service, &account, &legacy, false)?;
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
        if profile.provider == ProviderKind::AppleSpeech {
            return Ok(());
        }
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
        allow_collection_creation: bool,
    ) -> Result<(), String> {
        if profile.provider == ProviderKind::AppleSpeech {
            return Err(
                crate::core::credentials::ProviderCredentialsError::ProviderMismatch.to_string(),
            );
        }
        let account = credential_account(profile);
        self.save_secret(
            self.profile_keychain_service,
            &account,
            value,
            allow_collection_creation,
        )
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
            false,
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
        from_user_action: bool,
    ) -> Result<(), SecretStoreError> {
        if !self.is_ui_test {
            let result = if from_user_action {
                self.secret.save_from_user_action(service, account, value)
            } else {
                self.secret.save(service, account, value)
            };
            if let Err(error) = result {
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

    fn require_writable_credentials(&self, profile_id: &str) -> Result<(), String> {
        if self.secret.is_read_only() || self.profile_uses_local_dev_credentials(profile_id) {
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

/// Normalize the destination target first, then validate an explicit Apple
/// source without letting generic normalization silently choose another locale.
fn profile_selection_preferences(
    mut prefs: Preferences,
    profile: &ServiceProfile,
    source_language: Option<SourceLanguage>,
) -> Result<Preferences, String> {
    if source_language.is_some() && profile.provider != ProviderKind::AppleSpeech {
        return Err("apple_speech_source_override_invalid".into());
    }
    if source_language.is_none() {
        if let Some(preset) = profile.language_preset {
            profile.validate_language_preset(preset)?;
            prefs.source_language = preset.source_language;
            prefs.target_language = preset.target_language;
            prefs.translation_mode = TranslationMode::Turbo;
            return Ok(prefs);
        }
    }
    normalize_preferences_value(&mut prefs, profile);
    if let Some(source) = source_language {
        if source == SourceLanguage::Automatic {
            return Err("apple_speech_language_unsupported".into());
        }
        if !profile
            .capabilities(prefs.target_language)
            .source_languages
            .contains(&source)
        {
            return Err("apple_speech_translation_language_unsupported".into());
        }
        prefs.source_language = source;
    }
    Ok(prefs)
}

fn normalize_preferences_value(prefs: &mut Preferences, profile: &ServiceProfile) {
    let original_source = prefs.source_language;
    let normalized = profile.normalize_preferences(ProviderPreferences {
        source_language: prefs.source_language,
        target_language: prefs.target_language,
        translation_mode: prefs.translation_mode,
    });
    prefs.source_language = if profile.text_translation() == TextTranslation::Apple
        && normalized.target_language.translates_audio()
        && original_source == SourceLanguage::Automatic
    {
        SourceLanguage::Automatic
    } else {
        normalized.source_language
    };
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
    fn fresh_store(directory: &Path, fake: &FakeSecretStore) -> SettingsStore {
        SettingsStore::load_with_secret(
            directory.into(),
            false,
            Box::new(fake.clone()),
            PROFILE_KEYCHAIN_SERVICE,
            false,
        )
    }

    #[test]
    fn fresh_install_stays_empty_after_saving_preferences_and_restarting() {
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let store = fresh_store(directory.path(), &fake);
        assert_eq!(store.profile_catalog().unwrap(), (String::new(), vec![]));
        store
            .save_preferences_for_active_profile(|prefs| prefs.font_size = 19.0)
            .unwrap();
        drop(store);
        let restarted = fresh_store(directory.path(), &fake);
        assert_eq!(
            restarted.profile_catalog().unwrap(),
            (String::new(), vec![])
        );
        assert_eq!(restarted.preferences().font_size, 19.0);
        assert!(ProfileCatalog::default().validated().is_ok());
        assert!(ProfileCatalog {
            active_profile_id: "missing".into(),
            ..ProfileCatalog::default()
        }
        .validated()
        .is_err());
    }

    #[test]
    fn first_confirmed_profile_becomes_active_and_survives_restart() {
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let store = fresh_store(directory.path(), &fake);
        let first = store
            .create_profile(ProviderKind::GoogleGeminiLive, "First service")
            .unwrap();
        assert_eq!(store.active_profile().unwrap(), first);
        let second = store
            .create_profile(ProviderKind::AlibabaCloud, "Second service")
            .unwrap();
        drop(store);
        let restarted = fresh_store(directory.path(), &fake);
        assert_eq!(restarted.active_profile().unwrap(), first);
        assert_eq!(restarted.profile_catalog().unwrap().1, vec![first, second]);
    }

    #[test]
    fn failed_first_profile_write_restores_the_empty_catalog_and_preferences() {
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let mut store = fresh_store(directory.path(), &fake);
        let original = store.preferences();
        store.catalog_path = directory.path().join("blocked-catalog");
        std::fs::create_dir(&store.catalog_path).unwrap();
        assert!(store
            .create_profile(ProviderKind::GoogleGeminiLive, "First service")
            .is_err());
        assert_eq!(store.profile_catalog().unwrap(), (String::new(), vec![]));
        assert_eq!(store.preferences(), original);
        drop(store);
        assert_eq!(
            fresh_store(directory.path(), &fake)
                .profile_catalog()
                .unwrap(),
            (String::new(), vec![])
        );
    }

    #[test]
    fn pre_catalog_preferences_preserve_the_legacy_alibaba_profile() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("preferences.json"),
            serde_json::to_vec(&Preferences::default()).unwrap(),
        )
        .unwrap();
        let store = fresh_store(directory.path(), &FakeSecretStore::default());
        assert_eq!(
            store.active_profile().unwrap().id,
            DEFAULT_ALIBABA_PROFILE_ID
        );
        assert_eq!(store.profile_catalog().unwrap().1.len(), 1);
    }

    #[test]
    fn qwen_model_metadata_survives_reopening_settings_without_reading_credentials() {
        use crate::core::protocols::qwen_mt::QwenMTModel;
        let directory = tempfile::tempdir().unwrap();
        atomic_write(
            &directory.path().join(PROFILE_CATALOG_FILE),
            &serde_json::to_vec(&ProfileCatalog::legacy_alibaba()).unwrap(),
        )
        .unwrap();
        let fake = FakeSecretStore::default();
        let open = || {
            SettingsStore::load_with_secret(
                directory.path().into(),
                false,
                Box::new(fake.clone()),
                PROFILE_KEYCHAIN_SERVICE,
                false,
            )
        };
        let store = open();
        store
            .update_profile_options_with_model(
                DEFAULT_ALIBABA_PROFILE_ID,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(QwenMTModel::Flash),
            )
            .unwrap();
        drop(store);
        assert_eq!(
            open().active_profile().unwrap().qwen_mt_model,
            QwenMTModel::Flash
        );
        assert_eq!(
            fake.load_count(
                PROFILE_KEYCHAIN_SERVICE,
                "provider-profile:alibaba-default:alibabaCloud:api-key"
            ),
            0
        );
    }

    #[test]
    fn qwen_model_selection_is_scoped_and_reaches_both_session_and_text_probe() {
        use crate::core::protocols::qwen_mt::QwenMTModel;
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        store
            .save_api_key(DEFAULT_ALIBABA_PROFILE_ID, "synthetic-asr")
            .unwrap();
        let second = store
            .create_profile(ProviderKind::AlibabaCloud, "Second")
            .unwrap();
        for model in [QwenMTModel::Flash, QwenMTModel::Plus, QwenMTModel::Lite] {
            let profile = store
                .update_profile_options_with_model(
                    DEFAULT_ALIBABA_PROFILE_ID,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(model),
                )
                .unwrap();
            assert_eq!(store.configuration().unwrap().qwen_mt_model, model);
            assert_eq!(
                store
                    .configuration_for_text_probe(&profile)
                    .unwrap()
                    .qwen_mt_model,
                model
            );
            assert_eq!(
                store
                    .profile_catalog()
                    .unwrap()
                    .1
                    .iter()
                    .find(|p| p.id == second.id)
                    .unwrap()
                    .qwen_mt_model,
                QwenMTModel::Lite
            );
        }
        let other = store
            .create_profile(ProviderKind::GoogleGeminiLive, "Other")
            .unwrap();
        assert!(store
            .update_profile_options_with_model(
                &other.id,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(QwenMTModel::Flash)
            )
            .is_err());
    }

    #[test]
    fn apple_text_route_is_metadata_only_preserves_auto_and_ignores_broken_text_storage() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        store
            .save_api_key(DEFAULT_ALIBABA_PROFILE_ID, "synthetic-asr")
            .unwrap();
        let profile = store.active_profile().unwrap();
        let destination = SettingsStore::destination_account(&profile);
        fake.put(
            PROFILE_KEYCHAIN_SERVICE,
            &destination,
            "corrupt legacy destination",
        );
        fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &destination);
        store.secret_cache.lock().unwrap().clear();
        fake.state.lock().unwrap().loads.clear();
        store
            .save_credentials(
                &profile.id,
                &translation_request(TextTranslation::Apple, "", "", ""),
            )
            .unwrap();
        let profile = store.active_profile().unwrap();
        assert_eq!(profile.text_translation(), TextTranslation::Apple);
        assert_eq!(
            store.preferences().source_language,
            SourceLanguage::Automatic
        );
        store.prepare_for_listening().unwrap();
        assert_eq!(
            store.preferences().source_language,
            SourceLanguage::Automatic
        );
        assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, &destination), 0);
        assert_eq!(store.credential_state(&profile), CredentialState::Present);
        assert!(store
            .credential_editor_state(&profile.id, Some(TextTranslation::Apple))
            .unwrap()
            .saved_fields
            .is_empty());
        assert!(!CredentialRevealField::Token.allowed_for(&profile, Some(TextTranslation::Apple)));
        store
            .save_preferences(|prefs| {
                prefs.source_language = SourceLanguage::Japanese;
                prefs.target_language = TargetLanguage::English;
                prefs.network_proxy = ProxyConfig {
                    mode: crate::core::network_proxy::ProxyMode::Direct,
                    url: None,
                };
            })
            .unwrap();
        let config = store.configuration().unwrap();
        assert_eq!(config.credentials.alibaba_key(), Some("synthetic-asr"));
        assert_eq!(
            config.text_credentials,
            Some(TextTranslationCredentials::Apple)
        );
        let probe = store.configuration_for_text_probe(&profile).unwrap();
        assert_eq!(probe.source_language, SourceLanguage::Japanese);
        assert_eq!(probe.target_language, TargetLanguage::English);
        assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, &destination), 0);
    }

    #[test]
    fn apple_idle_profile_selection_preserves_auto_until_an_explicit_source_is_chosen() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = store
            .create_profile(ProviderKind::AlibabaCloud, "Apple text")
            .unwrap();
        store
            .save_preferences(|prefs| {
                prefs.source_language = SourceLanguage::Automatic;
                prefs.target_language = TargetLanguage::English;
            })
            .unwrap();
        store
            .save_credentials(
                &profile.id,
                &translation_request(TextTranslation::Apple, "synthetic-asr", "", ""),
            )
            .unwrap();
        let profile = store.profile(&profile.id).unwrap();
        assert_eq!(
            store
                .preferences_for_profile_selection(&profile, None)
                .unwrap()
                .source_language,
            SourceLanguage::Automatic
        );
        store.select_profile(&profile.id).unwrap();
        assert_eq!(
            store.preferences().source_language,
            SourceLanguage::Automatic
        );
        assert_eq!(
            store.configuration().unwrap_err(),
            "apple_translation_language_unsupported"
        );
        store
            .save_preferences_for_active_profile(|prefs| {
                prefs.source_language = SourceLanguage::Japanese
            })
            .unwrap();
        assert_eq!(
            store.configuration().unwrap().source_language,
            SourceLanguage::Japanese
        );
    }

    #[test]
    fn apple_text_route_keeps_custom_recognition_credentials_and_original_bypasses_it() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        for provider in [ProviderKind::AppleSpeech, ProviderKind::CustomOpenAIASR] {
            let profile = store.create_profile(provider, "Local MT").unwrap();
            if provider.is_custom_speech() {
                store
                    .save_credentials(
                        &profile.id,
                        &custom_speech_request(
                            "wss://speech.example/realtime",
                            "synthetic",
                            "synthetic-speech",
                        ),
                    )
                    .unwrap();
            }
            store
                .save_credentials(
                    &profile.id,
                    &translation_request(TextTranslation::Apple, "", "", ""),
                )
                .unwrap();
            store.select_profile(&profile.id).unwrap();
            store
                .save_preferences(|prefs| {
                    prefs.source_language = SourceLanguage::English;
                    prefs.target_language = TargetLanguage::Japanese;
                })
                .unwrap();
            let config = store.configuration().unwrap();
            assert_eq!(config.provider, provider);
            assert_eq!(
                config.text_credentials,
                Some(TextTranslationCredentials::Apple)
            );
            store
                .save_preferences(|prefs| {
                    prefs.target_language = TargetLanguage::Original;
                    if provider.is_custom_speech() {
                        prefs.source_language = SourceLanguage::Automatic;
                    }
                })
                .unwrap();
            assert_eq!(
                store.configuration().unwrap().text_network_proxy.mode,
                crate::core::network_proxy::ProxyMode::Direct
            );
            assert_eq!(store.configuration().unwrap().text_credentials, None);
            assert_eq!(
                fake.load_count(
                    PROFILE_KEYCHAIN_SERVICE,
                    &SettingsStore::destination_account(&profile)
                ),
                0
            );
        }
    }

    #[test]
    fn apple_target_switch_restores_translation_after_original_without_credentials() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = store
            .create_profile(ProviderKind::AppleSpeech, "Apple speech and translation")
            .unwrap();
        store
            .save_credentials(
                &profile.id,
                &translation_request(TextTranslation::Apple, "", "", ""),
            )
            .unwrap();
        store.select_profile(&profile.id).unwrap();
        store
            .save_preferences(|prefs| {
                prefs.source_language = SourceLanguage::Japanese;
                prefs.target_language = TargetLanguage::SimplifiedChinese;
            })
            .unwrap();
        let profile = store.active_profile().unwrap();
        let translated = store.configuration().unwrap();
        let selection = ProviderPreferences {
            source_language: SourceLanguage::Japanese,
            target_language: TargetLanguage::Original,
            translation_mode: TranslationMode::Turbo,
        };
        let original = store
            .configuration_for_target_switch(&profile, translated, selection)
            .unwrap();
        assert_eq!(original.text_credentials, None);
        store
            .save_preferences_for_active_profile(|prefs| {
                prefs.target_language = TargetLanguage::Original
            })
            .unwrap();
        let before = store.preferences();
        fake.make_unavailable(
            PROFILE_KEYCHAIN_SERVICE,
            &SettingsStore::destination_account(&profile),
        );
        fake.state.lock().unwrap().loads.clear();

        let restored = store
            .configuration_for_target_switch(
                &profile,
                original,
                ProviderPreferences {
                    target_language: TargetLanguage::SimplifiedChinese,
                    ..selection
                },
            )
            .unwrap();

        assert_eq!(restored.source_language, SourceLanguage::Japanese);
        assert_eq!(restored.target_language, TargetLanguage::SimplifiedChinese);
        assert_eq!(restored.credentials, ProviderCredentials::AppleSpeech);
        assert_eq!(
            restored.text_credentials,
            Some(TextTranslationCredentials::Apple)
        );
        assert!(fake.state.lock().unwrap().loads.is_empty());
        assert_eq!(store.preferences(), before);
        assert_eq!(store.active_profile().unwrap(), profile);
    }

    #[test]
    fn target_switch_still_rejects_a_missing_independent_text_key_before_saving() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let mut profile = store
            .create_profile(ProviderKind::AppleSpeech, "Unconfigured text service")
            .unwrap();
        profile.text_translation = Some(TextTranslation::DeepL);
        let original = LiveTranslationConfiguration::with_credentials(
            ProviderKind::AppleSpeech,
            ProviderCredentials::AppleSpeech,
            SourceLanguage::Japanese,
            TargetLanguage::Original,
            TranslationMode::Turbo,
        )
        .validated()
        .unwrap();
        let before = store.preferences();

        assert_eq!(
            store
                .configuration_for_target_switch(
                    &profile,
                    original.clone(),
                    ProviderPreferences {
                        source_language: SourceLanguage::Japanese,
                        target_language: TargetLanguage::SimplifiedChinese,
                        translation_mode: TranslationMode::Turbo,
                    },
                )
                .unwrap_err(),
            crate::core::credentials::ProviderCredentialsError::MissingTextTranslation.to_string()
        );
        assert_eq!(store.preferences(), before);
        assert_eq!(original.target_language, TargetLanguage::Original);
        assert_eq!(original.text_credentials, None);
    }

    #[test]
    fn apple_text_route_explicit_speech_update_preserves_historical_translator_and_rolls_back() {
        for previous in [
            ProviderCredentials::DeepLX {
                asr_api_key: "old-asr".into(),
                endpoint: "https://example.com/translate".into(),
                token: "old-text".into(),
            },
            ProviderCredentials::DeepL {
                asr_api_key: "old-asr".into(),
                api_key: "old-text:fx".into(),
            },
        ] {
            let directory = tempfile::tempdir().unwrap();
            let fake = FakeSecretStore::default();
            let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
            let profile = store
                .create_profile(ProviderKind::DeepLX, "Historical")
                .unwrap();
            let account = credential_account(&profile);
            let previous_value = previous
                .encode_for_keychain(match previous {
                    ProviderCredentials::DeepL { .. } => ProviderKind::AlibabaCloud,
                    _ => ProviderKind::DeepLX,
                })
                .unwrap();
            fake.put(PROFILE_KEYCHAIN_SERVICE, &account, &previous_value);
            store.secret_cache.lock().unwrap().clear();
            store
                .save_credentials(
                    &profile.id,
                    &translation_request(TextTranslation::Apple, "new-asr", "", ""),
                )
                .unwrap();
            let updated_value = fake.value(PROFILE_KEYCHAIN_SERVICE, &account).unwrap();
            let updated =
                ProviderCredentials::decode_for_profile(&profile, &updated_value).unwrap();
            assert_eq!(updated.alibaba_key(), Some("new-asr"));
            assert!(updated_value.contains("old-text"));
            let selected = store.profile(&profile.id).unwrap();
            assert_eq!(selected.effective_provider(), ProviderKind::AlibabaCloud);
            assert_eq!(selected.text_translation(), TextTranslation::Apple);
            let old_route = match previous {
                ProviderCredentials::DeepLX { .. } => TextTranslation::DeepLX,
                ProviderCredentials::DeepL { .. } => TextTranslation::DeepL,
                _ => unreachable!(),
            };
            store
                .save_credentials(&profile.id, &translation_request(old_route, "", "", ""))
                .unwrap();
            assert!(store
                .text_credentials_for_profile(&store.profile(&profile.id).unwrap())
                .unwrap()
                .is_some());
            store
                .save_credentials(
                    &profile.id,
                    &translation_request(TextTranslation::Apple, "", "", ""),
                )
                .unwrap();
            let updated_value = fake.value(PROFILE_KEYCHAIN_SERVICE, &account).unwrap();
            std::fs::remove_file(&store.catalog_path).unwrap();
            std::fs::create_dir(&store.catalog_path).unwrap();
            assert!(store
                .save_credentials(
                    &profile.id,
                    &translation_request(TextTranslation::Apple, "failed-asr", "", "")
                )
                .is_err());
            assert_eq!(
                fake.value(PROFILE_KEYCHAIN_SERVICE, &account).as_deref(),
                Some(updated_value.as_str())
            );
        }
    }

    #[test]
    fn apple_text_route_updates_alibaba_key_without_accessing_separate_text_destinations() {
        for (route, request) in [
            (
                TextTranslation::DeepLX,
                translation_request(
                    TextTranslation::DeepLX,
                    "",
                    "https://example.com/translate",
                    "synthetic-text",
                ),
            ),
            (
                TextTranslation::DeepL,
                translation_request(TextTranslation::DeepL, "", "", "synthetic-text:fx"),
            ),
            (
                TextTranslation::OpenAICompatible,
                openai_compatible_request(
                    "",
                    "https://example.com/v1",
                    "synthetic-text",
                    "synthetic-model",
                ),
            ),
            (
                TextTranslation::ChatMock,
                chatmock_request(
                    "http://localhost:8000/v1",
                    "synthetic-text",
                    "synthetic-model",
                ),
            ),
        ] {
            let fake = FakeSecretStore::default();
            let store = settings(&fake);
            let profile = store.active_profile().unwrap();
            store.save_api_key(&profile.id, "old-asr").unwrap();
            store.save_credentials(&profile.id, &request).unwrap();
            let destination = SettingsStore::destination_account(&profile);
            let previous = fake.value(PROFILE_KEYCHAIN_SERVICE, &destination).unwrap();
            let reads = fake.load_count(PROFILE_KEYCHAIN_SERVICE, &destination);
            store
                .save_credentials(
                    &profile.id,
                    &translation_request(TextTranslation::Apple, "new-asr", "", ""),
                )
                .unwrap();
            assert_eq!(
                fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile))
                    .as_deref(),
                Some("new-asr")
            );
            assert_eq!(
                fake.value(PROFILE_KEYCHAIN_SERVICE, &destination)
                    .as_deref(),
                Some(previous.as_str())
            );
            assert_eq!(
                fake.load_count(PROFILE_KEYCHAIN_SERVICE, &destination),
                reads
            );
            store
                .save_credentials(&profile.id, &translation_request(route, "", "", ""))
                .unwrap();
            assert!(store
                .text_credentials_for_profile(&store.profile(&profile.id).unwrap())
                .unwrap()
                .is_some());
        }
    }

    #[test]
    fn apple_profile_lifecycle_never_reads_or_writes_a_speech_credential() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = store
            .create_profile(ProviderKind::AppleSpeech, "Local Apple")
            .unwrap();
        let speech_account = credential_account(&profile);
        fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &speech_account);
        store.select_profile(&profile.id).unwrap();
        assert_eq!(
            store.credential_state_for_snapshot(&profile),
            CredentialState::Present
        );
        assert_eq!(store.credential_diagnostic(&profile), "present");
        let config = store.configuration().unwrap();
        assert_eq!(config.credentials, ProviderCredentials::AppleSpeech);
        assert_eq!(config.target_language, TargetLanguage::Original);
        assert_ne!(config.source_language, SourceLanguage::Automatic);
        assert!(store.configuration_for_speech_probe(&profile).is_ok());
        assert!(store
            .configuration_for_speech_draft_probe(&profile, &ProviderCredentials::AppleSpeech, true)
            .is_ok());
        assert!(store
            .credential_editor_state(&profile.id, None)
            .unwrap()
            .saved_fields
            .is_empty());
        assert!(store
            .save_credentials(&profile.id, &ProviderCredentials::AppleSpeech)
            .is_err());
        assert!(store
            .save_api_key(&profile.id, "synthetic-unrelated")
            .is_err());
        assert_eq!(
            fake.load_count(PROFILE_KEYCHAIN_SERVICE, &speech_account),
            0
        );
        store.delete_profile(&profile.id).unwrap();
        assert_eq!(
            fake.load_count(PROFILE_KEYCHAIN_SERVICE, &speech_account),
            0
        );
        assert!(fake
            .value(PROFILE_KEYCHAIN_SERVICE, &speech_account)
            .is_none());
    }

    fn save_language_preset(
        store: &SettingsStore,
        id: &str,
        source: SourceLanguage,
        target: TargetLanguage,
    ) {
        store
            .update_profile_options(
                id,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(crate::core::provider::ProfileLanguagePresetPatch {
                    preset: Some(crate::core::provider::ProfileLanguagePreset {
                        source_language: source,
                        target_language: target,
                    }),
                }),
            )
            .unwrap();
    }

    #[test]
    fn profile_language_presets_restore_independent_pairs_without_tracking_temporary_changes() {
        for provider in [ProviderKind::AppleSpeech, ProviderKind::AlibabaCloud] {
            let directory = tempfile::tempdir().unwrap();
            let fake = FakeSecretStore::default();
            let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
            let japanese = store.create_profile(provider, "Same name").unwrap();
            let english = store.create_profile(provider, "Same name").unwrap();
            let target = if provider == ProviderKind::AppleSpeech {
                TargetLanguage::Original
            } else {
                TargetLanguage::SimplifiedChinese
            };
            let before = store.preferences();
            save_language_preset(&store, &japanese.id, SourceLanguage::Japanese, target);
            save_language_preset(&store, &english.id, SourceLanguage::English, target);
            assert_eq!(store.preferences(), before); // Saving is not activation.
            store.select_profile(&japanese.id).unwrap();
            assert_eq!(
                store.preferences().source_language,
                SourceLanguage::Japanese
            );
            store.select_profile(&english.id).unwrap();
            assert_eq!(store.preferences().source_language, SourceLanguage::English);
            store
                .save_preferences_for_active_profile(|prefs| {
                    prefs.source_language = SourceLanguage::French
                })
                .unwrap();
            store.select_profile(&english.id).unwrap(); // Reselect restores the preset.
            assert_eq!(store.preferences().source_language, SourceLanguage::English);
            store.update_profile(&japanese.id, "Renamed").unwrap();
            let reopened = SettingsStore::at_path(directory.path().into(), Box::new(fake));
            reopened.select_profile(&japanese.id).unwrap();
            assert_eq!(
                reopened.preferences().source_language,
                SourceLanguage::Japanese
            );
            assert_eq!(reopened.preferences().target_language, target);
            reopened
                .update_profile_options(
                    &japanese.id,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(crate::core::provider::ProfileLanguagePresetPatch { preset: None }),
                )
                .unwrap();
            reopened
                .save_preferences_for_active_profile(|prefs| {
                    prefs.source_language = SourceLanguage::English
                })
                .unwrap();
            reopened.select_profile(&japanese.id).unwrap();
            assert_eq!(
                reopened.preferences().source_language,
                SourceLanguage::English
            );
        }
    }

    #[test]
    fn profile_language_preset_failures_leave_metadata_and_selection_unchanged() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = SettingsStore::at_path(
            directory.path().into(),
            Box::new(FakeSecretStore::default()),
        );
        let apple = store
            .create_profile(ProviderKind::AppleSpeech, "Apple")
            .unwrap();
        save_language_preset(
            &store,
            &apple.id,
            SourceLanguage::Japanese,
            TargetLanguage::Original,
        );
        let before = store.preferences();
        let catalog = store.profile_catalog().unwrap();
        let invalid = crate::core::provider::ProfileLanguagePreset {
            source_language: SourceLanguage::Automatic,
            target_language: TargetLanguage::Original,
        };
        assert_eq!(
            store
                .update_profile_options(
                    &apple.id,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(crate::core::provider::ProfileLanguagePresetPatch {
                        preset: Some(invalid)
                    })
                )
                .unwrap_err(),
            "profile_language_preset_unsupported"
        );
        let blocked = directory.path().join("blocked-write");
        std::fs::create_dir(&blocked).unwrap();
        store.catalog_path = blocked;
        assert!(store.select_profile(&apple.id).is_err());
        assert_eq!(store.preferences(), before);
        assert_eq!(store.profile_catalog().unwrap(), catalog);
        assert!(store
            .update_profile_options(
                &apple.id,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(crate::core::provider::ProfileLanguagePresetPatch { preset: None })
            )
            .is_err());
        assert_eq!(store.profile_catalog().unwrap(), catalog);
    }

    #[test]
    fn explicit_apple_source_selection_overrides_a_preset_without_rewriting_it() {
        let directory = tempfile::tempdir().unwrap();
        let store = SettingsStore::at_path(
            directory.path().into(),
            Box::new(FakeSecretStore::default()),
        );
        let apple = store
            .create_profile(ProviderKind::AppleSpeech, "Apple")
            .unwrap();
        save_language_preset(
            &store,
            &apple.id,
            SourceLanguage::Japanese,
            TargetLanguage::Original,
        );
        store
            .save_preferences(|prefs| prefs.target_language = TargetLanguage::Original)
            .unwrap();
        store
            .select_profile_with_source(&apple.id, Some(SourceLanguage::English))
            .unwrap();
        assert_eq!(store.preferences().source_language, SourceLanguage::English);
        store.select_profile(&apple.id).unwrap();
        assert_eq!(
            store.preferences().source_language,
            SourceLanguage::Japanese
        );
    }

    #[test]
    fn apple_profile_selection_saves_the_chosen_source_with_the_profile() {
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        let apple = store
            .create_profile(ProviderKind::AppleSpeech, "Apple")
            .unwrap();
        store
            .save_preferences(|prefs| {
                prefs.source_language = SourceLanguage::Automatic;
                prefs.target_language = TargetLanguage::Original;
                prefs.audio_input = AudioInput::Both;
                prefs.retain_session_history = true;
                prefs.record_session_audio = true;
            })
            .unwrap();
        let before = store.preferences();
        let catalog = std::fs::read(&store.catalog_path).unwrap();
        let persisted = std::fs::read(&store.prefs_path).unwrap();
        let proposed = store
            .configuration_for_profile_selection_with_source(&apple, Some(SourceLanguage::English))
            .unwrap();
        assert_eq!(proposed.source_language, SourceLanguage::English);
        assert_eq!(proposed.target_language, TargetLanguage::Original);
        assert_eq!(store.preferences(), before);
        assert_eq!(std::fs::read(&store.catalog_path).unwrap(), catalog);
        assert_eq!(std::fs::read(&store.prefs_path).unwrap(), persisted);

        store
            .select_profile_with_source(&apple.id, Some(SourceLanguage::English))
            .unwrap();
        assert_eq!(store.configuration().unwrap(), proposed);
        assert_eq!(store.active_profile().unwrap().id, apple.id);
        let selected = store.preferences();
        assert_eq!(selected.audio_input, AudioInput::Both);
        assert!(selected.retain_session_history);
        assert!(selected.record_session_audio);
        // Reusing the active profile must still apply the requested language.
        store
            .select_profile_with_source(&apple.id, Some(SourceLanguage::Japanese))
            .unwrap();
        assert_eq!(
            store.preferences().source_language,
            SourceLanguage::Japanese
        );
        let reopened = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        assert_eq!(reopened.active_profile().unwrap().id, apple.id);
        assert_eq!(
            reopened.preferences().source_language,
            SourceLanguage::Japanese
        );
        assert_eq!(
            fake.load_count(PROFILE_KEYCHAIN_SERVICE, &credential_account(&apple)),
            0
        );
    }

    #[test]
    fn apple_explicit_profile_source_rejects_incompatible_routes_without_normalizing_it() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let apple = store
            .create_profile(ProviderKind::AppleSpeech, "Apple")
            .unwrap();
        store
            .save_credentials(
                &apple.id,
                &translation_request(TextTranslation::DeepL, "", "", "synthetic-text:fx"),
            )
            .unwrap();
        let apple = store.profile(&apple.id).unwrap();
        store
            .save_preferences(|prefs| prefs.target_language = TargetLanguage::English)
            .unwrap();
        let before = store.preferences();
        let active = store.active_profile().unwrap();
        for (profile, source, expected) in [
            (
                &apple,
                SourceLanguage::Automatic,
                "apple_speech_language_unsupported",
            ),
            (
                &apple,
                SourceLanguage::Khmer,
                "apple_speech_translation_language_unsupported",
            ),
            (
                &active,
                SourceLanguage::English,
                "apple_speech_source_override_invalid",
            ),
        ] {
            assert_eq!(
                store
                    .configuration_for_profile_selection_with_source(profile, Some(source))
                    .unwrap_err(),
                expected
            );
            assert_eq!(
                store
                    .select_profile_with_source(&profile.id, Some(source))
                    .unwrap_err(),
                expected
            );
            assert_eq!(store.preferences(), before);
            assert_eq!(store.active_profile().unwrap(), active);
        }
        // A recognition-only target does not impose the text provider's limits.
        store
            .save_preferences(|prefs| prefs.target_language = TargetLanguage::Original)
            .unwrap();
        store
            .select_profile_with_source(&apple.id, Some(SourceLanguage::Khmer))
            .unwrap();
        assert_eq!(store.preferences().source_language, SourceLanguage::Khmer);
    }

    #[test]
    fn apple_profile_source_selection_rolls_back_if_catalog_persistence_fails() {
        let directory = tempfile::tempdir().unwrap();
        let store = SettingsStore::at_path(
            directory.path().into(),
            Box::new(FakeSecretStore::default()),
        );
        let apple = store
            .create_profile(ProviderKind::AppleSpeech, "Apple")
            .unwrap();
        store
            .save_preferences(|prefs| {
                prefs.source_language = SourceLanguage::Automatic;
                prefs.target_language = TargetLanguage::Original;
            })
            .unwrap();
        let before = store.preferences();
        let persisted = std::fs::read(&store.prefs_path).unwrap();
        std::fs::remove_file(&store.catalog_path).unwrap();
        std::fs::create_dir(&store.catalog_path).unwrap();
        assert_eq!(
            store
                .select_profile_with_source(&apple.id, Some(SourceLanguage::Japanese))
                .unwrap_err(),
            PROFILE_CATALOG_UNAVAILABLE
        );
        assert_eq!(store.preferences(), before);
        assert_eq!(std::fs::read(&store.prefs_path).unwrap(), persisted);
        assert_eq!(
            store.active_profile().unwrap().id,
            DEFAULT_ALIBABA_PROFILE_ID
        );
    }

    #[test]
    fn apple_speech_probe_checks_the_visible_language_without_mutation_or_text_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        let apple = store
            .create_profile(ProviderKind::AppleSpeech, "Apple")
            .unwrap();
        store
            .save_credentials(
                &apple.id,
                &translation_request(TextTranslation::DeepL, "", "", "synthetic-text:fx"),
            )
            .unwrap();
        let apple = store.profile(&apple.id).unwrap();
        store
            .save_preferences(|prefs| {
                prefs.source_language = SourceLanguage::Automatic;
                prefs.target_language = TargetLanguage::English;
            })
            .unwrap();
        let before = store.preferences();
        let active = store.active_profile().unwrap();
        let catalog = std::fs::read(&store.catalog_path).unwrap();
        let persisted = std::fs::read(&store.prefs_path).unwrap();
        store.secret_cache.lock().unwrap().clear();
        fake.state.lock().unwrap().loads.clear();
        fake.make_unavailable(
            PROFILE_KEYCHAIN_SERVICE,
            &SettingsStore::destination_account(&apple),
        );
        for source in [
            SourceLanguage::English,
            SourceLanguage::Japanese,
            SourceLanguage::French,
        ] {
            let probe = store
                .configuration_for_speech_probe_with_source(&apple, Some(source))
                .unwrap();
            assert_eq!(probe.source_language, source);
            assert_eq!(probe.target_language, TargetLanguage::Original);
            assert_eq!(probe.credentials, ProviderCredentials::AppleSpeech);
            assert_eq!(probe.text_credentials, None);
        }
        assert_eq!(
            store
                .configuration_for_speech_probe_with_source(&apple, Some(SourceLanguage::Automatic))
                .unwrap_err(),
            "apple_speech_language_unsupported"
        );
        assert_eq!(
            store
                .configuration_for_speech_probe_with_source(&active, Some(SourceLanguage::English))
                .unwrap_err(),
            "apple_speech_source_override_invalid"
        );
        assert_eq!(store.preferences(), before);
        assert_eq!(store.active_profile().unwrap(), active);
        assert_eq!(std::fs::read(&store.catalog_path).unwrap(), catalog);
        assert_eq!(std::fs::read(&store.prefs_path).unwrap(), persisted);
        assert!(fake.state.lock().unwrap().loads.is_empty());
    }

    #[test]
    fn apple_independent_translation_owns_its_credentials_and_probes_without_speech() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = store
            .create_profile(ProviderKind::AppleSpeech, "Apple and local MT")
            .unwrap();
        let speech_account = credential_account(&profile);
        fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &speech_account);
        store
            .save_credentials(
                &profile.id,
                &openai_compatible_request(
                    "",
                    "http://localhost:8080/v1",
                    "",
                    "synthetic-local-model",
                ),
            )
            .unwrap();
        let profile = store.profile(&profile.id).unwrap();
        store.select_profile(&profile.id).unwrap();
        store
            .save_preferences_for_active_profile(|prefs| {
                prefs.source_language = SourceLanguage::English;
                prefs.target_language = TargetLanguage::SimplifiedChinese;
            })
            .unwrap();
        let config = store.configuration().unwrap();
        assert_eq!(config.credentials, ProviderCredentials::AppleSpeech);
        assert!(
            matches!(config.text_credentials, Some(TextTranslationCredentials::OpenAICompatible { api_key, .. }) if api_key.is_empty())
        );
        assert!(store.configuration_for_text_probe(&profile).is_ok());
        assert!(store.configuration_for_speech_probe(&profile).is_ok());
        assert_eq!(
            fake.load_count(PROFILE_KEYCHAIN_SERVICE, &speech_account),
            0
        );
        assert!(store
            .save_credentials(
                &profile.id,
                &openai_compatible_request(
                    "synthetic-forbidden-speech",
                    "http://localhost:8080/v1",
                    "",
                    "synthetic-model"
                )
            )
            .is_err());
        store.delete_profile(&profile.id).unwrap();
        assert!(fake
            .value(
                PROFILE_KEYCHAIN_SERVICE,
                &SettingsStore::destination_account(&profile)
            )
            .is_none());
        assert_eq!(
            fake.load_count(PROFILE_KEYCHAIN_SERVICE, &speech_account),
            0
        );
    }

    #[test]
    fn profile_selection_configuration_matches_saved_selection_without_mutating_it() {
        // Keep a target supported by the selected provider; normalize only
        // targets outside its dedicated realtime translation catalog.
        for (target, expected_target) in [
            (TargetLanguage::French, TargetLanguage::French),
            (
                TargetLanguage::TraditionalChinese,
                TargetLanguage::SimplifiedChinese,
            ),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let fake = FakeSecretStore::default();
            let store = SettingsStore::at_path(directory.path().into(), Box::new(fake));
            let next = store
                .create_profile(ProviderKind::OpenAIRealtime, "Synthetic next")
                .unwrap();
            store.save_api_key(&next.id, "synthetic-next").unwrap();
            store
                .save_preferences_for_active_profile(|prefs| {
                    prefs.source_language = SourceLanguage::Japanese;
                    prefs.target_language = target;
                    prefs.audio_input = AudioInput::Both;
                    prefs.retain_session_history = true;
                    prefs.record_session_audio = true;
                })
                .unwrap();
            let previous = store.preferences();
            let active = store.active_profile().unwrap();
            let catalog = std::fs::read(&store.catalog_path).unwrap();
            let persisted = std::fs::read(&store.prefs_path).unwrap();

            let proposed = store.configuration_for_profile_selection(&next).unwrap();
            assert_eq!(proposed.source_language, SourceLanguage::Automatic);
            assert_eq!(proposed.target_language, expected_target);
            assert_eq!(store.preferences(), previous);
            assert_eq!(store.active_profile().unwrap(), active);
            assert_eq!(std::fs::read(&store.catalog_path).unwrap(), catalog);
            assert_eq!(std::fs::read(&store.prefs_path).unwrap(), persisted);

            store.select_profile(&next.id).unwrap();
            assert_eq!(store.configuration().unwrap(), proposed);
            let selected = store.preferences();
            assert_eq!(selected.audio_input, AudioInput::Both);
            assert!(selected.retain_session_history);
            assert!(selected.record_session_audio);
        }
    }

    #[test]
    fn incomplete_profile_selection_leaves_current_preferences_and_catalog_untouched() {
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let store = SettingsStore::at_path(directory.path().into(), Box::new(fake));
        store
            .save_api_key(DEFAULT_ALIBABA_PROFILE_ID, "synthetic-current")
            .unwrap();
        let next = store
            .create_profile(ProviderKind::OpenAIRealtime, "Unconfigured")
            .unwrap();
        store
            .save_preferences_for_active_profile(|prefs| {
                prefs.source_language = SourceLanguage::Japanese
            })
            .unwrap();
        let current_configuration = store.configuration().unwrap();
        let previous = store.preferences();
        let active = store.active_profile().unwrap();
        let catalog = std::fs::read(&store.catalog_path).unwrap();
        let persisted = std::fs::read(&store.prefs_path).unwrap();

        assert!(store.configuration_for_profile_selection(&next).is_err());
        assert_eq!(store.configuration().unwrap(), current_configuration);
        assert_eq!(store.preferences(), previous);
        assert_eq!(store.active_profile().unwrap(), active);
        assert_eq!(std::fs::read(&store.catalog_path).unwrap(), catalog);
        assert_eq!(std::fs::read(&store.prefs_path).unwrap(), persisted);
    }

    #[test]
    fn profile_selection_preserves_original_without_reading_custom_text_credentials() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let next = store
            .create_profile(ProviderKind::CustomOpenAIASR, "Recognition only")
            .unwrap();
        store
            .save_credentials(
                &next.id,
                &custom_speech_request(
                    "wss://speech.example/realtime",
                    "synthetic-model",
                    "synthetic-key",
                ),
            )
            .unwrap();
        store
            .save_credentials(
                &next.id,
                &translation_request(TextTranslation::DeepL, "", "", "synthetic-text:fx"),
            )
            .unwrap();
        let next = store.profile(&next.id).unwrap();
        let destination = SettingsStore::destination_account(&next);
        store
            .secret_cache
            .lock()
            .unwrap()
            .remove(&cache_key(PROFILE_KEYCHAIN_SERVICE, &destination));
        fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &destination);
        let previous_reads = fake.load_count(PROFILE_KEYCHAIN_SERVICE, &destination);
        store
            .save_preferences_for_active_profile(|prefs| {
                prefs.target_language = TargetLanguage::Original
            })
            .unwrap();

        let proposed = store.configuration_for_profile_selection(&next).unwrap();
        assert_eq!(proposed.target_language, TargetLanguage::Original);
        assert_eq!(proposed.text_credentials, None);
        assert_eq!(
            fake.load_count(PROFILE_KEYCHAIN_SERVICE, &destination),
            previous_reads
        );
        assert_eq!(
            store.active_profile().unwrap().id,
            DEFAULT_ALIBABA_PROFILE_ID
        );
    }

    #[test]
    fn unsaved_text_probe_never_needs_asr_or_mutates_profile_preferences_or_secrets() {
        use crate::core::configuration::TextTranslationProbeCredentials;
        for provider in [ProviderKind::AlibabaCloud, ProviderKind::CustomOpenAIASR] {
            let directory = tempfile::tempdir().unwrap();
            let fake = FakeSecretStore::default();
            let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
            let profile = store.create_profile(provider, "Draft only").unwrap();
            let catalog = std::fs::read(&store.catalog_path).unwrap();
            let preferences = store.preferences();
            let active = store.active_profile().unwrap();
            let speech_account = credential_account(&profile);
            fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &speech_account);
            let draft =
                openai_compatible_request("", "https://draft.example/v1", "", "draft-model");
            let probe = store
                .configuration_for_text_draft_probe(&profile, &draft)
                .unwrap();
            assert!(
                matches!(probe.credentials, TextTranslationProbeCredentials::Independent(
                TextTranslationCredentials::OpenAICompatible { endpoint, model, api_key }
            ) if endpoint == "https://draft.example/v1/chat/completions" && model == "draft-model" && api_key.is_empty())
            );
            assert_eq!(
                fake.load_count(PROFILE_KEYCHAIN_SERVICE, &speech_account),
                0
            );
            assert_eq!(store.active_profile().unwrap(), active);
            assert_eq!(store.preferences(), preferences);
            assert_eq!(store.profile(&profile.id).unwrap(), profile);
            assert_eq!(std::fs::read(&store.catalog_path).unwrap(), catalog);
            assert!(fake.state.lock().unwrap().values.is_empty());
        }
    }

    #[test]
    fn text_draft_probe_merges_saved_destination_without_persisting_or_reusing_other_routes() {
        use crate::core::configuration::TextTranslationProbeCredentials;
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = store
            .create_profile(ProviderKind::CustomOpenAIASR, "Draft check")
            .unwrap();
        store
            .save_credentials(
                &profile.id,
                &openai_compatible_request(
                    "",
                    "https://saved.example/v1",
                    "synthetic-saved",
                    "saved-model",
                ),
            )
            .unwrap();
        let profile = store.profile(&profile.id).unwrap();
        let values = fake.state.lock().unwrap().values.clone();
        let preferences = store.preferences();
        for (endpoint, token, clear_token, route, expected_key) in [
            (
                "",
                "",
                false,
                TextTranslation::OpenAICompatible,
                "synthetic-saved",
            ),
            (
                "https://saved.example/v1",
                "",
                false,
                TextTranslation::OpenAICompatible,
                "synthetic-saved",
            ),
            (
                "https://changed.example/v1",
                "",
                false,
                TextTranslation::OpenAICompatible,
                "",
            ),
            ("", "", true, TextTranslation::OpenAICompatible, ""),
            (
                "",
                "synthetic-draft",
                false,
                TextTranslation::OpenAICompatible,
                "synthetic-draft",
            ),
            (
                "https://saved.example/v1",
                "",
                false,
                TextTranslation::ChatMock,
                "",
            ),
        ] {
            let draft = ProviderCredentials::AlibabaTranslation {
                api_key: String::new(),
                text_translation: route,
                endpoint: endpoint.into(),
                token: token.into(),
                model: "draft-model".into(),
                clear_token,
            };
            let probe = store
                .configuration_for_text_draft_probe(&profile, &draft)
                .unwrap();
            let TextTranslationProbeCredentials::Independent(credentials) = probe.credentials
            else {
                panic!("independent route expected")
            };
            assert_eq!(credentials.translation(), route);
            match credentials {
                TextTranslationCredentials::OpenAICompatible { api_key, model, .. }
                | TextTranslationCredentials::ChatMock { api_key, model, .. } => {
                    assert_eq!(api_key, expected_key);
                    assert_eq!(model, "draft-model");
                }
                _ => panic!("chat route expected"),
            }
            assert_eq!(store.profile(&profile.id).unwrap(), profile);
            assert_eq!(store.preferences(), preferences);
            assert_eq!(fake.state.lock().unwrap().values, values);
        }
    }

    #[test]
    fn complete_draft_probes_can_bypass_unavailable_saved_credentials() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = store
            .create_profile(ProviderKind::CustomOpenAIASR, "Unavailable saved fields")
            .unwrap();
        fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile));
        fake.make_unavailable(
            PROFILE_KEYCHAIN_SERVICE,
            &SettingsStore::destination_account(&profile),
        );
        let speech =
            custom_speech_request("wss://draft.example/realtime", "speech-model", "synthetic");
        assert!(store
            .configuration_for_speech_draft_probe(&profile, &speech, true)
            .is_ok());
        let text = openai_compatible_request(
            "",
            "https://draft.example/v1",
            "synthetic-text",
            "text-model",
        );
        assert!(store
            .configuration_for_text_draft_probe(&profile, &text)
            .is_ok());
        assert!(fake.state.lock().unwrap().loads.is_empty());
        assert!(fake.state.lock().unwrap().values.is_empty());
        let invalid = openai_compatible_request(
            "",
            "https://user:secret@invalid.example/v1",
            "synthetic",
            "model",
        );
        assert!(store
            .configuration_for_text_draft_probe(&profile, &invalid)
            .is_err());
        assert!(fake.state.lock().unwrap().loads.is_empty());
        assert!(store
            .configuration_for_text_draft_probe(&profile, &speech)
            .is_err());
        assert!(store
            .configuration_for_speech_draft_probe(&profile, &text, true)
            .is_err());
        assert_eq!(store.profile(&profile.id).unwrap(), profile);
    }

    #[test]
    fn speech_draft_reuses_saved_key_and_checks_new_profile_without_persistence() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = store
            .create_profile(ProviderKind::OpenAIRealtime, "Integrated draft")
            .unwrap();
        let new = store
            .configuration_for_speech_draft_probe(
                &profile,
                &ProviderCredentials::api_key("synthetic-new"),
                false,
            )
            .unwrap();
        assert_eq!(new.credentials.direct_api_key(), Some("synthetic-new"));
        assert!(fake.state.lock().unwrap().values.is_empty());
        store
            .save_credentials(
                &profile.id,
                &ProviderCredentials::api_key("synthetic-saved"),
            )
            .unwrap();
        let before = fake.state.lock().unwrap().values.clone();
        let merged = store
            .configuration_for_speech_draft_probe(
                &profile,
                &ProviderCredentials::api_key(""),
                false,
            )
            .unwrap();
        assert_eq!(merged.credentials.direct_api_key(), Some("synthetic-saved"));
        assert_eq!(fake.state.lock().unwrap().values, before);
    }

    #[test]
    fn custom_language_declaration_persists_normalizes_and_preserves_unrelated_patches() {
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        let profile = store
            .create_profile(ProviderKind::CustomDashScopeASR, "Synthetic")
            .unwrap();
        store.select_profile(&profile.id).unwrap();
        store
            .save_preferences_for_active_profile(|prefs| {
                prefs.target_language = TargetLanguage::Original;
                prefs.source_language = SourceLanguage::Japanese;
            })
            .unwrap();
        let saved = store
            .update_profile_options(
                &profile.id,
                None,
                None,
                None,
                None,
                None,
                Some(CustomSpeechLanguagesPatch {
                    languages: Some(vec![
                        SourceLanguage::German,
                        SourceLanguage::English,
                        SourceLanguage::German,
                    ]),
                }),
                None,
            )
            .unwrap();
        assert_eq!(
            saved.custom_speech_source_languages,
            Some(vec![SourceLanguage::English, SourceLanguage::German])
        );
        assert_eq!(
            store.preferences().source_language,
            SourceLanguage::Automatic
        );
        store.update_profile(&profile.id, "Renamed").unwrap();
        let restored = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        assert_eq!(
            restored
                .active_profile()
                .unwrap()
                .custom_speech_source_languages,
            saved.custom_speech_source_languages
        );
        assert_eq!(
            restored.preferences().source_language,
            SourceLanguage::Automatic
        );
        assert!(store
            .save_preferences_for_active_profile(
                |prefs| prefs.source_language = SourceLanguage::Japanese
            )
            .is_err());
        store
            .save_preferences_for_active_profile(|prefs| {
                prefs.source_language = SourceLanguage::German
            })
            .unwrap();
        store
            .update_profile_options(
                &profile.id,
                None,
                None,
                None,
                None,
                None,
                Some(CustomSpeechLanguagesPatch {
                    languages: Some(vec![]),
                }),
                None,
            )
            .unwrap();
        assert_eq!(
            store.preferences().source_language,
            SourceLanguage::Automatic
        );
        store
            .update_profile_options(
                &profile.id,
                None,
                None,
                None,
                None,
                None,
                Some(CustomSpeechLanguagesPatch { languages: None }),
                None,
            )
            .unwrap();
        store
            .save_preferences_for_active_profile(|prefs| {
                prefs.source_language = SourceLanguage::French
            })
            .unwrap();
        assert_eq!(
            store
                .active_profile()
                .unwrap()
                .custom_speech_source_languages,
            None
        );
        assert!(fake.state.lock().unwrap().loads.is_empty());
        assert!(fake.state.lock().unwrap().values.is_empty());
    }

    #[test]
    fn inactive_language_declaration_does_not_normalize_the_active_profile() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        store
            .save_preferences_for_active_profile(|prefs| {
                prefs.target_language = TargetLanguage::Original;
                prefs.source_language = SourceLanguage::Japanese;
            })
            .unwrap();
        let before = store.preferences();
        let custom = store
            .create_profile(ProviderKind::CustomOpenAIASR, "Other")
            .unwrap();
        store
            .update_profile_options(
                &custom.id,
                None,
                None,
                None,
                None,
                None,
                Some(CustomSpeechLanguagesPatch {
                    languages: Some(vec![SourceLanguage::English]),
                }),
                None,
            )
            .unwrap();
        assert_eq!(store.preferences(), before);
        let builtin = store.active_profile().unwrap();
        assert!(store
            .update_profile_options(
                &builtin.id,
                None,
                None,
                None,
                None,
                None,
                Some(CustomSpeechLanguagesPatch { languages: None }),
                None
            )
            .is_err());
        assert_eq!(store.preferences(), before);
        assert_eq!(store.active_profile().unwrap(), builtin);
        store.select_profile(&custom.id).unwrap();
        assert_eq!(
            store.preferences().source_language,
            SourceLanguage::Automatic
        );
    }

    #[test]
    fn failed_custom_language_write_rolls_back_preferences_and_profile() {
        for fail_catalog in [true, false] {
            let directory = tempfile::tempdir().unwrap();
            let fake = FakeSecretStore::default();
            let mut store = SettingsStore::at_path(directory.path().into(), Box::new(fake));
            let custom = store
                .create_profile(ProviderKind::CustomDashScopeASR, "Synthetic")
                .unwrap();
            store.select_profile(&custom.id).unwrap();
            store
                .save_preferences_for_active_profile(|prefs| {
                    prefs.target_language = TargetLanguage::Original;
                    prefs.source_language = SourceLanguage::Japanese;
                })
                .unwrap();
            let before = store.preferences();
            let blocked = directory.path().join("blocked-write");
            std::fs::create_dir(&blocked).unwrap();
            if fail_catalog {
                store.catalog_path = blocked;
            } else {
                store.prefs_path = blocked;
            }
            assert!(store
                .update_profile_options(
                    &custom.id,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(CustomSpeechLanguagesPatch {
                        languages: Some(vec![SourceLanguage::English])
                    }),
                    None
                )
                .is_err());
            assert_eq!(store.preferences(), before);
            assert_eq!(store.active_profile().unwrap(), custom);
            let restarted = SettingsStore::at_path(
                directory.path().into(),
                Box::new(FakeSecretStore::default()),
            );
            assert_eq!(restarted.preferences(), before);
            assert_eq!(restarted.active_profile().unwrap(), custom);
        }
    }

    #[test]
    fn speech_probes_and_listening_honor_declared_languages_without_widening_them() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let custom = store
            .create_profile(ProviderKind::CustomDashScopeASR, "Synthetic")
            .unwrap();
        let speech =
            custom_speech_request("wss://speech.example/asr", "synthetic-model", "synthetic");
        store.save_credentials(&custom.id, &speech).unwrap();
        let custom = store
            .update_profile_options(
                &custom.id,
                None,
                None,
                None,
                None,
                None,
                Some(CustomSpeechLanguagesPatch {
                    languages: Some(vec![SourceLanguage::French, SourceLanguage::German]),
                }),
                None,
            )
            .unwrap();
        // Current preferences belong to another profile; a probe may normalize
        // its temporary source, but cannot silently broaden this declaration.
        store
            .save_preferences(|prefs| {
                prefs.source_language = SourceLanguage::Japanese;
                prefs.target_language = TargetLanguage::Original;
            })
            .unwrap();
        assert_eq!(
            store
                .configuration_for_speech_probe(&custom)
                .unwrap()
                .source_language,
            SourceLanguage::Automatic
        );
        assert_eq!(
            store
                .configuration_for_speech_draft_probe(&custom, &speech, true)
                .unwrap()
                .source_language,
            SourceLanguage::Automatic
        );
        assert_eq!(
            store.preferences().source_language,
            SourceLanguage::Japanese
        );
        assert_eq!(store.configuration_for_profile(&custom).unwrap_err(), crate::core::configuration::LiveTranslationConfigurationError::UnsupportedSourceLanguage.to_string());
        store.select_profile(&custom.id).unwrap();
        store
            .save_preferences_for_active_profile(|prefs| {
                prefs.source_language = SourceLanguage::German
            })
            .unwrap();
        store
            .save_credentials(
                &custom.id,
                &openai_compatible_request(
                    "",
                    "https://translation.example/v1",
                    "",
                    "synthetic-model",
                ),
            )
            .unwrap();
        store
            .save_preferences_for_active_profile(|prefs| {
                prefs.target_language = TargetLanguage::French
            })
            .unwrap();
        let configuration = store.configuration().unwrap();
        assert_eq!(configuration.source_language, SourceLanguage::German);
        assert_eq!(configuration.target_language, TargetLanguage::French);
        assert_eq!(
            store
                .active_profile()
                .unwrap()
                .custom_speech_source_languages,
            custom.custom_speech_source_languages
        );
    }

    #[test]
    fn profile_metadata_patches_preserve_independent_names_and_proxies() {
        use crate::core::network_proxy::ProxyMode;
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        let profile = store.active_profile().unwrap();
        let configuration_name = "B 站 · @home / (测试) 😀";
        let translator_name = "翻译 & 字幕 + [本地] 🐱";
        store
            .update_profile(&profile.id, &format!("  {configuration_name}  "))
            .unwrap();
        let text_proxy = ProxyConfig {
            mode: ProxyMode::Direct,
            url: None,
        };
        store
            .update_profile_options(
                &profile.id,
                None,
                None,
                Some(text_proxy.clone()),
                Some(TextTranslationName {
                    route: TextTranslation::OpenAICompatible,
                    name: format!("  {translator_name}  "),
                }),
                None,
                None,
                None,
            )
            .unwrap();
        // An independently saved alias or proxy must not replay a name copied
        // from an older snapshot. Renaming in turn preserves those fields.
        assert_eq!(store.profile(&profile.id).unwrap().name, configuration_name);
        let renamed = format!("{configuration_name} 2");
        store.update_profile(&profile.id, &renamed).unwrap();
        store
            .update_profile_options(
                &profile.id,
                None,
                Some(ProxyConfig::default()),
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let reloaded = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        let saved = reloaded.profile(&profile.id).unwrap();
        assert_eq!(saved.name, renamed);
        assert_eq!(saved.text_network_proxy, Some(text_proxy));
        assert_eq!(saved.speech_network_proxy, Some(ProxyConfig::default()));
        assert_eq!(
            saved.text_translation_names[&TextTranslation::OpenAICompatible],
            translator_name
        );
        let secrets = fake.state.lock().unwrap();
        assert!(secrets.loads.is_empty());
        assert!(secrets.values.is_empty());
    }

    #[test]
    fn profile_name_patches_enforce_unicode_bounds_without_losing_saved_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let store = SettingsStore::at_path(
            directory.path().into(),
            Box::new(FakeSecretStore::default()),
        );
        let profile = store.active_profile().unwrap();
        let name = "😀".repeat(64);
        let saved = store
            .update_profile_options(
                &profile.id,
                Some(&name),
                None,
                None,
                Some(TextTranslationName {
                    route: TextTranslation::OpenAICompatible,
                    name: name.clone(),
                }),
                None,
                None,
                None,
            )
            .unwrap();
        assert_eq!(saved.name, name);
        for invalid in ["  ".to_string(), "😀".repeat(65)] {
            assert!(store.update_profile(&profile.id, &invalid).is_err());
            assert_eq!(store.profile(&profile.id).unwrap(), saved);
        }
        assert!(store
            .update_profile_options(
                &profile.id,
                None,
                None,
                None,
                Some(TextTranslationName {
                    route: TextTranslation::OpenAICompatible,
                    name: "😀".repeat(65),
                }),
                None,
                None,
                None,
            )
            .is_err());
        assert_eq!(store.profile(&profile.id).unwrap(), saved);
    }

    #[test]
    fn speech_name_patches_persist_independently_without_credential_access() {
        for provider in [
            ProviderKind::CustomDashScopeASR,
            ProviderKind::CustomOpenAIASR,
        ] {
            let directory = tempfile::tempdir().unwrap();
            let fake = FakeSecretStore::default();
            let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
            let profile = store
                .create_profile(provider, "Original configuration")
                .unwrap();
            store
                .update_profile_options(
                    &profile.id,
                    None,
                    Some(ProxyConfig::default()),
                    None,
                    Some(TextTranslationName {
                        route: TextTranslation::OpenAICompatible,
                        name: "Independent translator".into(),
                    }),
                    None,
                    None,
                    None,
                )
                .unwrap();
            store
                .update_profile(&profile.id, "Renamed configuration")
                .unwrap();
            let named = store
                .update_profile_options(
                    &profile.id,
                    None,
                    None,
                    None,
                    None,
                    Some("  Whisper · 本地  "),
                    None,
                    None,
                )
                .unwrap();
            assert_eq!(named.name, "Renamed configuration");
            assert_eq!(
                named.speech_recognition_name.as_deref(),
                Some("Whisper · 本地")
            );
            assert_eq!(named.speech_network_proxy, Some(ProxyConfig::default()));
            assert_eq!(
                named.text_translation_names[&TextTranslation::OpenAICompatible],
                "Independent translator"
            );
            let other = store.create_profile(provider, "Other").unwrap();
            assert!(other.speech_recognition_name.is_none());
            let reloaded = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
            assert_eq!(reloaded.profile(&profile.id).unwrap(), named);
            let cleared = reloaded
                .update_profile_options(&profile.id, None, None, None, None, Some("  "), None, None)
                .unwrap();
            assert!(cleared.speech_recognition_name.is_none());
            assert_eq!(cleared.name, named.name);
            assert_eq!(cleared.text_translation_names, named.text_translation_names);
            assert_eq!(cleared.speech_network_proxy, named.speech_network_proxy);
            let reopened = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
            assert_eq!(reopened.profile(&profile.id).unwrap(), cleared);
            let secrets = fake.state.lock().unwrap();
            assert!(secrets.loads.is_empty());
            assert!(secrets.values.is_empty());
        }
    }

    #[test]
    fn speech_name_updates_validate_and_roll_back_failed_writes() {
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let mut store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        let built_in = store.active_profile().unwrap();
        assert!(store
            .update_profile_options(
                &built_in.id,
                None,
                None,
                None,
                None,
                Some("Unsupported"),
                None,
                None
            )
            .is_err());
        assert_eq!(store.active_profile().unwrap(), built_in);
        let profile = store
            .create_profile(ProviderKind::CustomOpenAIASR, "Custom")
            .unwrap();
        for invalid in [
            "😀".repeat(65),
            "line\nbreak".into(),
            "control\u{0000}".into(),
        ] {
            assert!(store
                .update_profile_options(
                    &profile.id,
                    Some("Unpersisted configuration"),
                    None,
                    None,
                    None,
                    Some(&invalid),
                    None,
                    None,
                )
                .is_err());
            assert_eq!(store.profile(&profile.id).unwrap(), profile);
        }
        store.catalog_path = directory.path().join("blocked-catalog");
        std::fs::create_dir(&store.catalog_path).unwrap();
        assert!(store
            .update_profile_options(
                &profile.id,
                None,
                None,
                None,
                None,
                Some("Unpersisted recognition"),
                None,
                None,
            )
            .is_err());
        assert_eq!(store.profile(&profile.id).unwrap(), profile);
        let reopened = SettingsStore::at_path(directory.path().into(), Box::new(fake));
        assert_eq!(reopened.profile(&profile.id).unwrap(), profile);
    }

    #[test]
    fn translation_names_persist_independently_without_credential_access() {
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        let profile = store.active_profile().unwrap();
        for (route, name) in [
            (TextTranslation::OpenAICompatible, "  Work translator  "),
            (TextTranslation::DeepLX, "Local translator"),
        ] {
            store
                .update_profile_options(
                    &profile.id,
                    None,
                    None,
                    None,
                    Some(TextTranslationName {
                        route,
                        name: name.into(),
                    }),
                    None,
                    None,
                    None,
                )
                .unwrap();
        }
        store
            .update_profile(&profile.id, "Renamed profile")
            .unwrap();
        let other = store
            .create_profile(ProviderKind::AlibabaCloud, "Other")
            .unwrap();
        assert!(other.text_translation_names.is_empty());
        let reloaded = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        let restored = reloaded.profile(&profile.id).unwrap();
        assert_eq!(restored.name, "Renamed profile");
        assert_eq!(
            restored.text_translation_names[&TextTranslation::OpenAICompatible],
            "Work translator"
        );
        assert_eq!(
            restored.text_translation_names[&TextTranslation::DeepLX],
            "Local translator"
        );
        assert_eq!(restored.text_translation(), TextTranslation::FollowService);
        reloaded
            .update_profile_options(
                &profile.id,
                None,
                None,
                None,
                Some(TextTranslationName {
                    route: TextTranslation::OpenAICompatible,
                    name: "  ".into(),
                }),
                None,
                None,
                None,
            )
            .unwrap();
        let reopened = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        let names = reopened
            .profile(&profile.id)
            .unwrap()
            .text_translation_names;
        assert_eq!(names.len(), 1);
        assert_eq!(names[&TextTranslation::DeepLX], "Local translator");
        let secrets = fake.state.lock().unwrap();
        assert!(secrets.loads.is_empty());
        assert!(secrets.values.is_empty());
    }

    #[test]
    fn translation_name_updates_reject_invalid_values_and_roll_back_failed_writes() {
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let mut store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        let profile = store.active_profile().unwrap();
        for (route, name) in [
            (TextTranslation::OpenAICompatible, "x".repeat(65)),
            (TextTranslation::FollowService, "Unsupported".into()),
        ] {
            assert!(store
                .update_profile_options(
                    &profile.id,
                    Some("Unpersisted profile name"),
                    None,
                    None,
                    Some(TextTranslationName { route, name }),
                    None,
                    None,
                    None,
                )
                .is_err());
            assert_eq!(store.active_profile().unwrap(), profile);
        }
        store.catalog_path = directory.path().join("blocked-catalog");
        std::fs::create_dir(&store.catalog_path).unwrap();
        assert!(store
            .update_profile_options(
                &profile.id,
                None,
                None,
                None,
                Some(TextTranslationName {
                    route: TextTranslation::OpenAICompatible,
                    name: "Unpersisted translator".into(),
                }),
                None,
                None,
                None,
            )
            .is_err());
        assert_eq!(store.active_profile().unwrap(), profile);
        let reloaded = SettingsStore::at_path(directory.path().into(), Box::new(fake));
        assert_eq!(reloaded.active_profile().unwrap(), profile);
    }

    #[test]
    fn translation_names_survive_route_saves_without_changing_saved_secrets() {
        for provider in [ProviderKind::AlibabaCloud, ProviderKind::CustomOpenAIASR] {
            let fake = FakeSecretStore::default();
            let store = settings(&fake);
            let profile = store.create_profile(provider, "Named translator").unwrap();
            let speech = if provider.is_custom_speech() {
                custom_speech_request("wss://speech.example/realtime", "speech-model", "synthetic")
            } else {
                ProviderCredentials::api_key("synthetic")
            };
            store.save_credentials(&profile.id, &speech).unwrap();
            store
                .save_credentials(
                    &profile.id,
                    &openai_compatible_request(
                        "",
                        "https://translation.example/v1",
                        "synthetic-text-key",
                        "text-model",
                    ),
                )
                .unwrap();
            let before = fake.state.lock().unwrap().values.clone();
            fake.state.lock().unwrap().loads.clear();
            let named = store
                .update_profile_options(
                    &profile.id,
                    None,
                    None,
                    None,
                    Some(TextTranslationName {
                        route: TextTranslation::OpenAICompatible,
                        name: "Work translator".into(),
                    }),
                    provider.is_custom_speech().then_some("My recognition"),
                    None,
                    None,
                )
                .unwrap();
            assert_eq!(fake.state.lock().unwrap().values, before);
            assert!(fake.state.lock().unwrap().loads.is_empty());
            for route in [
                TextTranslation::FollowService,
                TextTranslation::OpenAICompatible,
            ] {
                store
                    .save_credentials(&profile.id, &translation_request(route, "", "", ""))
                    .unwrap();
                let switched = store.profile(&profile.id).unwrap();
                assert_eq!(switched.text_translation(), route);
                assert_eq!(
                    switched.text_translation_names,
                    named.text_translation_names
                );
                assert_eq!(
                    switched.speech_recognition_name,
                    named.speech_recognition_name
                );
            }
            assert_eq!(fake.state.lock().unwrap().values, before);
        }
    }

    #[test]
    fn profile_stage_proxies_persist_and_legacy_routes_remain_isolated() {
        use crate::core::network_proxy::ProxyMode;
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let mut store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        let profile = store.active_profile().unwrap();
        store.save_api_key(&profile.id, "synthetic-asr").unwrap();
        let inherited = ProxyConfig {
            mode: ProxyMode::Custom,
            url: Some("http://127.0.0.1:7890/".into()),
        };
        store
            .save_preferences_for_active_profile(|prefs| prefs.network_proxy = inherited.clone())
            .unwrap();
        let previous = store.configuration().unwrap();
        let text = ProxyConfig {
            mode: ProxyMode::Direct,
            url: None,
        };
        store
            .update_profile_options(
                &profile.id,
                None,
                None,
                Some(text.clone()),
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let updated = store.active_profile().unwrap();
        assert_eq!(updated.speech_network_proxy, None);
        assert_eq!(store.configuration().unwrap().network_proxy, inherited);
        assert_eq!(store.configuration().unwrap().text_network_proxy, text);
        assert_eq!(
            store
                .configuration_for_text_probe(&updated)
                .unwrap()
                .network_proxy,
            text
        );
        assert_eq!(
            store
                .configuration_for_speech_probe(&updated)
                .unwrap()
                .network_proxy,
            inherited
        );
        assert_eq!(previous.text_network_proxy, inherited);
        let other = store
            .create_profile(ProviderKind::AlibabaCloud, "Other")
            .unwrap();
        store.save_api_key(&other.id, "synthetic-other").unwrap();
        store.select_profile(&other.id).unwrap();
        assert_eq!(store.configuration().unwrap().text_network_proxy, inherited);
        let speech = ProxyConfig {
            mode: ProxyMode::Custom,
            url: Some("socks5h://127.0.0.1:1080".into()),
        };
        store
            .update_profile_options(
                &profile.id,
                Some("Renamed"),
                Some(speech.clone()),
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        store.select_profile(&profile.id).unwrap();
        let reloaded = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        assert_eq!(reloaded.configuration().unwrap().network_proxy, speech);
        assert_eq!(reloaded.configuration().unwrap().text_network_proxy, text);
        assert_eq!(
            reloaded
                .configuration_for_profile_probe(&reloaded.active_profile().unwrap())
                .unwrap()
                .text_network_proxy,
            text
        );
        assert_eq!(reloaded.preferences().network_proxy, inherited);
        let before = store.active_profile().unwrap();
        let bytes = std::fs::read(&store.catalog_path).unwrap();
        assert!(store
            .update_profile_options(
                &profile.id,
                Some("Rejected"),
                None,
                Some(ProxyConfig {
                    mode: ProxyMode::Custom,
                    url: Some("http://user:private-value@localhost:7890".into())
                }),
                None,
                None,
                None,
                None,
            )
            .is_err());
        assert_eq!(store.active_profile().unwrap(), before);
        assert_eq!(std::fs::read(&store.catalog_path).unwrap(), bytes);
        store.catalog_path = directory.path().join("blocked-catalog");
        std::fs::create_dir(&store.catalog_path).unwrap();
        assert!(store
            .update_profile_options(
                &profile.id,
                Some("Unpersisted"),
                Some(ProxyConfig::default()),
                None,
                None,
                None,
                None,
                None,
            )
            .is_err());
        assert_eq!(store.active_profile().unwrap(), before);
        assert_eq!(reloaded.active_profile().unwrap(), before);
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires an isolated desktop Secret Service and synthetic native prompt interaction"]
    fn linux_secret_service_first_save_and_restart() {
        use secret_service::{blocking::SecretService, EncryptionType};
        let mode = std::env::var("MIMI_TEST_SECRET_SERVICE_FIRST_SAVE").unwrap();
        let directory = std::env::var("MIMI_TEST_PRIVATE_KEYRING_DIRECTORY").unwrap();
        assert!(std::env::var("DBUS_SESSION_BUS_ADDRESS")
            .unwrap()
            .contains(&directory));
        let store = SettingsStore::in_memory_with_scope(
            Box::new(KeyringSecretStore),
            false,
            "app.yuxino.mimi.test.secret-service-first-save",
            false,
        );
        let profile = store.active_profile().unwrap();
        let synthetic_value = "synthetic-first-save-value";
        if mode == "read" {
            assert_eq!(store.credential_state(&profile), CredentialState::Present);
            assert!(store.load_api_key().unwrap().as_deref() == Some(synthetic_value));
            store.delete_api_key(&profile.id).unwrap();
            assert_eq!(store.credential_state(&profile), CredentialState::Missing);
            return;
        }
        assert_eq!(mode, "write");
        let service = SecretService::connect(EncryptionType::Dh).unwrap();
        let assert_no_default = || {
            assert!(matches!(
                service.get_default_collection(),
                Err(secret_service::Error::NoResult)
            ));
        };
        assert_no_default();
        assert_eq!(store.credential_diagnostic(&profile), "missing");
        assert_no_default();
        // The background-migration write path must also leave the collection
        // absent instead of raising a first-run prompt during a settings read.
        assert_eq!(
            KeyringSecretStore.save(
                store.profile_keychain_service,
                &credential_account(&profile),
                synthetic_value,
            ),
            Err(SecretStoreError::AccessDenied)
        );
        assert_no_default();
        // The script dismisses the first native prompt, then enters a password
        // for an encrypted synthetic keyring on the explicit second save.
        let progress = Path::new(&directory).join("first-save-phase");
        std::fs::write(&progress, "cancel").unwrap();
        assert_eq!(
            store.save_api_key(&profile.id, synthetic_value),
            Err(CREDENTIAL_STORE_ACCESS_DENIED.into())
        );
        assert_no_default();
        std::fs::write(&progress, "accept").unwrap();
        store.save_api_key(&profile.id, synthetic_value).unwrap();
        assert_eq!(store.credential_state(&profile), CredentialState::Present);
        assert!(service.get_default_collection().is_ok());

        // A new test process has no Mimi credential cache. This proves a
        // durable backend read across app-process lifetimes, not Stop/Start.
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "settings_store::tests::linux_secret_service_first_save_and_restart",
                "--exact",
                "--ignored",
            ])
            .env("MIMI_TEST_SECRET_SERVICE_FIRST_SAVE", "read")
            .status()
            .unwrap();
        assert!(result.success());
    }

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
    fn dock_visibility_defaults_on_and_preserves_saved_choices_without_credentials() {
        let legacy: Preferences = serde_json::from_str(r#"{"ui_language":"ja"}"#).unwrap();
        assert!(legacy.show_in_dock);
        let hidden: Preferences =
            serde_json::from_str(r#"{"ui_language":"ja","show_in_dock":false}"#).unwrap();
        assert!(!hidden.show_in_dock);
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        assert!(store.preferences().show_in_dock);
        store
            .save_preferences_for_active_profile(|prefs| prefs.show_in_dock = false)
            .unwrap();
        let profile = store
            .create_profile(ProviderKind::OpenAIRealtime, "Synthetic")
            .unwrap();
        store.select_profile(&profile.id).unwrap();
        let reloaded = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        assert!(!reloaded.preferences().show_in_dock);
        reloaded
            .save_preferences(|prefs| prefs.show_in_dock = true)
            .unwrap();
        let final_store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        assert!(final_store.preferences().show_in_dock);
        assert!(fake.state.lock().unwrap().loads.is_empty());
    }

    #[test]
    fn application_target_defaults_and_requires_fresh_recording_opt_in() {
        use crate::core::system_audio_target::SystemAudioTarget;
        let mut preferences: Preferences = serde_json::from_str("{}").unwrap();
        assert_eq!(preferences.system_audio_target, SystemAudioTarget::System);
        preferences.record_session_audio = true;
        let target = SystemAudioTarget::Application {
            id: "com.example.player".into(),
            name: "Player".into(),
        };
        preferences.apply_system_audio_target(target.clone());
        assert!(!preferences.record_session_audio);
        preferences.record_session_audio = true;
        preferences.apply_system_audio_target(target.clone());
        assert!(preferences.record_session_audio);
        let restored: Preferences =
            serde_json::from_value(serde_json::to_value(&preferences).unwrap()).unwrap();
        assert_eq!(restored.system_audio_target, target);
        preferences.apply_system_audio_target(SystemAudioTarget::System);
        assert!(!preferences.record_session_audio);
    }

    #[test]
    fn audio_input_legacy_default_and_explicit_selection_round_trip() {
        let legacy: super::Preferences = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy.audio_input, AudioInput::System);
        assert!(!legacy.record_session_audio);
        for input in [AudioInput::System, AudioInput::Microphone, AudioInput::Both] {
            let mut preferences = legacy.clone();
            preferences.apply_audio_preferences(Some(input), None);
            let restored: super::Preferences =
                serde_json::from_str(&serde_json::to_string(&preferences).unwrap()).unwrap();
            assert_eq!(restored.audio_input, input);
            assert!(!restored.record_session_audio);
        }
    }

    #[test]
    fn changing_audio_input_requires_a_new_recording_opt_in() {
        for previous in [AudioInput::System, AudioInput::Microphone, AudioInput::Both] {
            for next in [AudioInput::System, AudioInput::Microphone, AudioInput::Both] {
                if previous == next {
                    continue;
                }
                for request in [None, Some(false), Some(true)] {
                    let mut preferences = super::Preferences {
                        audio_input: previous,
                        record_session_audio: true,
                        ..Default::default()
                    };
                    preferences.apply_audio_preferences(Some(next), request);
                    assert_eq!(preferences.audio_input, next);
                    assert!(!preferences.record_session_audio);
                    preferences.apply_audio_preferences(None, Some(true));
                    assert!(preferences.record_session_audio);
                    preferences.apply_audio_preferences(Some(next), None);
                    assert!(preferences.record_session_audio);
                }
            }
        }
    }

    #[test]
    fn loading_preferences_preserves_explicit_inputs_and_their_recording_opt_in() {
        for blocked_catalog in [false, true] {
            for input in [AudioInput::System, AudioInput::Microphone, AudioInput::Both] {
                let directory = tempfile::tempdir().unwrap();
                let previous = Preferences {
                    audio_input: input,
                    record_session_audio: true,
                    font_size: 19.0,
                    overlay_frame: Some(OverlayFrame {
                        x: 20.0,
                        y: 30.0,
                        width: 700.0,
                        height: 350.0,
                    }),
                    ..Preferences::default()
                };
                std::fs::write(
                    directory.path().join("preferences.json"),
                    serde_json::to_vec(&previous).unwrap(),
                )
                .unwrap();
                if blocked_catalog {
                    std::fs::write(directory.path().join(PROFILE_CATALOG_FILE), b"invalid")
                        .unwrap();
                }
                let store = SettingsStore::at_path(
                    directory.path().into(),
                    Box::new(FakeSecretStore::default()),
                );
                let loaded = store.preferences();
                assert_eq!(loaded.audio_input, input);
                assert!(loaded.record_session_audio);
                assert_eq!(loaded.font_size, previous.font_size);
                assert_eq!(loaded.overlay_frame, previous.overlay_frame);
                let persisted: Preferences = serde_json::from_slice(
                    &std::fs::read(directory.path().join("preferences.json")).unwrap(),
                )
                .unwrap();
                assert_eq!(persisted.audio_input, input);
                assert!(persisted.record_session_audio);
            }
        }
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
                |prefs| prefs.target_language = TargetLanguage::TraditionalChinese
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
                prefs.target_language = TargetLanguage::Khmer;
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
        assert_eq!(probe.source_language, SourceLanguage::French);
        assert_eq!(probe.target_language, TargetLanguage::Original);
        assert_eq!(store.preferences(), expanded);
        store
            .save_credentials(
                &active.id,
                &translation_request(TextTranslation::DeepL, "", "", "synthetic:fx"),
            )
            .unwrap();
        assert_eq!(store.preferences().source_language, SourceLanguage::French);
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
                |prefs| prefs.target_language = TargetLanguage::Khmer
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
                prefs.target_language = TargetLanguage::Khmer;
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
    fn windows_credential_manager_imports_enterprise_into_file_then_retires_native_item() {
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
        let profile = ServiceProfile::new(
            format!("credential-file-migration-{unique}"),
            "Credential migration test",
            ProviderKind::OpenAIRealtime,
        )
        .unwrap();
        let account = credential_account(&profile);
        let directory = tempfile::tempdir().unwrap();
        let catalog = ProfileCatalog {
            schema_version: PROFILE_CATALOG_SCHEMA_VERSION,
            active_profile_id: profile.id.clone(),
            profiles: vec![profile],
        };
        std::fs::write(
            directory.path().join(PROFILE_CATALOG_FILE),
            serde_json::to_vec(&catalog).unwrap(),
        )
        .unwrap();
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

        // Legacy reads must not rewrite the native item or its persistence.
        let observed = keyring_core::Entry::new(&service, &account).unwrap();
        assert_eq!(observed.get_password().unwrap(), "migration-secret");
        assert_eq!(
            observed.get_attributes().unwrap()["persistence"],
            "Enterprise"
        );

        let store =
            file_credentials::FileCredentialStore::for_app(directory.path(), &service, false);
        assert_eq!(store.pending_imports().unwrap(), 1);
        store.migrate_legacy().unwrap();
        assert_eq!(store.pending_imports().unwrap(), 0);
        let saved: serde_json::Value = serde_json::from_slice(
            &std::fs::read(directory.path().join("credentials/credentials.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(saved["entries"][&service][&account], "migration-secret");
        assert!(matches!(
            observed.get_password(),
            Err(keyring_core::Error::NoEntry)
        ));
        drop(store);

        let restarted =
            file_credentials::FileCredentialStore::for_app(directory.path(), &service, false);
        restarted.migrate_legacy().unwrap();
        assert_eq!(
            restarted.load(&service, &account).unwrap().as_deref(),
            Some("migration-secret")
        );
        restarted
            .save(&service, &account, "updated-secret")
            .unwrap();
        assert_eq!(
            restarted.load(&service, &account).unwrap().as_deref(),
            Some("updated-secret")
        );
        assert!(matches!(
            observed.get_password(),
            Err(keyring_core::Error::NoEntry)
        ));
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
        fn contains(&self, service: &str, account: &str) -> Result<bool, SecretStoreError> {
            let state = self.state.lock().unwrap();
            let key = cache_key(service, account);
            if let Some(error) = state.failures.get(&key) {
                return Err(*error);
            }
            if state.unavailable.contains(&key) {
                return Err(SecretStoreError::Unavailable);
            }
            Ok(state.values.contains_key(&key))
        }

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

    fn custom_speech_request(endpoint: &str, model: &str, key: &str) -> ProviderCredentials {
        ProviderCredentials::CustomSpeech {
            endpoint: endpoint.into(),
            model: model.into(),
            api_key: key.into(),
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn settings_snapshots_and_profile_switches_never_authorize_keys() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let first = ServiceProfile::alibaba_default();
        let second = openai_profile(&store, "Second provider");
        let third = store
            .create_profile(ProviderKind::GoogleGeminiLive, "Third provider")
            .unwrap();
        for profile in [&first, &second, &third] {
            fake.put(
                PROFILE_KEYCHAIN_SERVICE,
                &credential_account(profile),
                "synthetic-profile-key",
            );
        }
        for profile in [&first, &second, &third, &first, &third, &second] {
            store.select_profile(&profile.id).unwrap();
            for _ in 0..2 {
                let snapshot =
                    crate::commands::SettingsSnapshotPayload::try_from_store(&store).unwrap();
                assert_eq!(snapshot.active_profile_id, profile.id);
                assert!(snapshot
                    .profiles
                    .iter()
                    .all(|p| p.credential_state == CredentialState::Present));
            }
            assert!(fake.state.lock().unwrap().loads.is_empty());
        }
        // Actual listening configuration still validates each selected key once.
        for profile in [&first, &second, &third] {
            store.select_profile(&profile.id).unwrap();
            for _ in 0..2 {
                assert!(store.configuration().is_ok());
            }
            assert_eq!(
                fake.load_count(PROFILE_KEYCHAIN_SERVICE, &credential_account(profile)),
                1
            );
        }
        assert_eq!(
            fake.load_count(PROFILE_KEYCHAIN_SERVICE, LEGACY_MIGRATION_TOMBSTONE_ACCOUNT),
            0
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn selecting_custom_pipeline_does_not_authorize_either_credential_slot() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = store
            .create_profile(ProviderKind::CustomDashScopeASR, "Separate pipeline")
            .unwrap();
        store
            .save_credentials(
                &profile.id,
                &custom_speech_request(
                    "wss://speech.example/realtime",
                    "synthetic-model",
                    "synthetic-speech",
                ),
            )
            .unwrap();
        store
            .save_credentials(
                &profile.id,
                &openai_compatible_request(
                    "",
                    "https://text.example/v1",
                    "synthetic-text",
                    "synthetic-model",
                ),
            )
            .unwrap();
        store.secret_cache.lock().unwrap().clear();
        fake.state.lock().unwrap().loads.clear();
        let snapshot = crate::commands::SettingsSnapshotPayload::try_from_store(&store).unwrap();
        let inactive = snapshot
            .profiles
            .iter()
            .find(|p| p.id == profile.id)
            .unwrap();
        assert_eq!(
            inactive.speech_credential_state,
            Some(CredentialState::Present)
        );
        assert_eq!(
            inactive.text_credential_state,
            Some(CredentialState::Present)
        );
        assert_eq!(
            fake.load_count(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile)),
            0
        );
        assert_eq!(
            fake.load_count(
                PROFILE_KEYCHAIN_SERVICE,
                &SettingsStore::destination_account(&profile)
            ),
            0
        );
        store.select_profile(&profile.id).unwrap();
        crate::commands::SettingsSnapshotPayload::try_from_store(&store).unwrap();
        assert!(fake.state.lock().unwrap().loads.is_empty());
        let configured = store.profile(&profile.id).unwrap();
        for _ in 0..2 {
            assert!(store.configuration_for_profile_probe(&configured).is_ok());
        }
        assert_eq!(
            fake.load_count(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile)),
            1
        );
        assert_eq!(
            fake.load_count(
                PROFILE_KEYCHAIN_SERVICE,
                &SettingsStore::destination_account(&profile)
            ),
            1
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn snapshot_presence_respects_legacy_tombstone_and_cached_access_failure() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = ServiceProfile::alibaba_default();
        fake.put(
            LEGACY_KEYCHAIN_SERVICE_V3,
            LEGACY_KEYCHAIN_ACCOUNT,
            "synthetic-legacy",
        );
        assert_eq!(
            store.credential_state_for_snapshot(&profile),
            CredentialState::Present
        );
        assert!(fake.state.lock().unwrap().loads.is_empty());
        fake.put(
            PROFILE_KEYCHAIN_SERVICE,
            LEGACY_MIGRATION_TOMBSTONE_ACCOUNT,
            LEGACY_MIGRATION_TOMBSTONE_VALUE,
        );
        assert_eq!(
            store.credential_state_for_snapshot(&profile),
            CredentialState::Missing
        );
        assert!(fake.state.lock().unwrap().loads.is_empty());
        let other = openai_profile(&store, "Access failure");
        let account = credential_account(&other);
        fake.put(PROFILE_KEYCHAIN_SERVICE, &account, "synthetic-key");
        fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &account);
        assert_eq!(store.credential_state(&other), CredentialState::Unavailable);
        fake.state
            .lock()
            .unwrap()
            .unavailable
            .remove(&cache_key(PROFILE_KEYCHAIN_SERVICE, &account));
        assert_eq!(
            store.credential_state_for_snapshot(&other),
            CredentialState::Unavailable
        );
        assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, &account), 1);
    }

    #[test]
    fn stage_probes_read_only_their_saved_credentials_without_switching_or_persisting() {
        use crate::core::configuration::TextTranslationProbeCredentials;
        for provider in [
            ProviderKind::AlibabaCloud,
            ProviderKind::CustomDashScopeASR,
            ProviderKind::CustomOpenAIASR,
        ] {
            let fake = FakeSecretStore::default();
            let store = settings(&fake);
            let profile = store.create_profile(provider, "Stage probe").unwrap();
            if provider.is_custom_speech() {
                store
                    .save_credentials(
                        &profile.id,
                        &custom_speech_request(
                            "wss://speech.example/realtime",
                            "synthetic-model",
                            "synthetic-speech",
                        ),
                    )
                    .unwrap();
            } else {
                store
                    .save_credentials(
                        &profile.id,
                        &ProviderCredentials::api_key("synthetic-speech"),
                    )
                    .unwrap();
            }
            store
                .save_credentials(
                    &profile.id,
                    &openai_compatible_request(
                        "",
                        "https://text.example/v1",
                        "synthetic-text",
                        "synthetic-text-model",
                    ),
                )
                .unwrap();
            let profile = store.profile(&profile.id).unwrap();
            let speech_account = credential_account(&profile);
            let text_account = SettingsStore::destination_account(&profile);
            let before = store.preferences();
            let before_active_id = store.active_profile().unwrap().id;
            store.secret_cache.lock().unwrap().clear();
            fake.state.lock().unwrap().loads.clear();
            fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &text_account);
            let speech = store.configuration_for_speech_probe(&profile).unwrap();
            assert_eq!(speech.target_language, TargetLanguage::Original);
            assert_eq!(speech.text_credentials, None);
            assert_eq!(
                fake.load_count(PROFILE_KEYCHAIN_SERVICE, &speech_account),
                1
            );
            assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, &text_account), 0);
            {
                let mut state = fake.state.lock().unwrap();
                state
                    .unavailable
                    .remove(&cache_key(PROFILE_KEYCHAIN_SERVICE, &text_account));
                state
                    .unavailable
                    .insert(cache_key(PROFILE_KEYCHAIN_SERVICE, &speech_account));
                state.loads.clear();
            }
            store.secret_cache.lock().unwrap().clear();
            let text = store.configuration_for_text_probe(&profile).unwrap();
            assert!(
                matches!(text.credentials, TextTranslationProbeCredentials::Independent(TextTranslationCredentials::OpenAICompatible { api_key, .. }) if api_key == "synthetic-text")
            );
            assert_eq!(
                fake.load_count(PROFILE_KEYCHAIN_SERVICE, &speech_account),
                0
            );
            assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, &text_account), 1);
            assert_eq!(store.preferences(), before);
            assert_eq!(store.active_profile().unwrap().id, before_active_id);
        }
    }

    #[test]
    fn text_probe_can_precede_custom_speech_setup_and_retry_only_its_locked_slot() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = store
            .create_profile(ProviderKind::CustomOpenAIASR, "Text first")
            .unwrap();
        assert_eq!(
            store.configuration_for_text_probe(&profile).unwrap_err(),
            "text_translation_not_configured"
        );
        store
            .save_credentials(
                &profile.id,
                &translation_request(TextTranslation::DeepL, "", "", "synthetic-text:fx"),
            )
            .unwrap();
        let profile = store.profile(&profile.id).unwrap();
        let speech_account = credential_account(&profile);
        let text_account = SettingsStore::destination_account(&profile);
        store.secret_cache.lock().unwrap().clear();
        fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &text_account);
        assert!(store.configuration_for_text_probe(&profile).is_err());
        fake.state
            .lock()
            .unwrap()
            .unavailable
            .remove(&cache_key(PROFILE_KEYCHAIN_SERVICE, &text_account));
        fake.state.lock().unwrap().loads.clear();
        assert!(store.configuration_for_text_probe(&profile).is_ok());
        assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, &text_account), 1);
        assert_eq!(
            fake.load_count(PROFILE_KEYCHAIN_SERVICE, &speech_account),
            0
        );
    }

    #[test]
    fn legacy_combined_deeplx_text_probe_retries_its_item_after_unlocking() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = store
            .create_profile(ProviderKind::DeepLX, "Legacy combined")
            .unwrap();
        let account = credential_account(&profile);
        let value = ProviderCredentials::DeepLX {
            asr_api_key: "synthetic-asr".into(),
            endpoint: "https://translation.example/translate".into(),
            token: "synthetic-text".into(),
        }
        .encode_for_keychain(ProviderKind::DeepLX)
        .unwrap();
        fake.put(PROFILE_KEYCHAIN_SERVICE, &account, &value);
        fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &account);
        assert_eq!(
            store.credential_state(&profile),
            CredentialState::Unavailable
        );
        assert!(store.configuration_for_text_probe(&profile).is_err());
        fake.state
            .lock()
            .unwrap()
            .unavailable
            .remove(&cache_key(PROFILE_KEYCHAIN_SERVICE, &account));
        fake.state.lock().unwrap().loads.clear();
        let configuration = store.configuration_for_text_probe(&profile).unwrap();
        assert!(matches!(configuration.credentials,
            crate::core::configuration::TextTranslationProbeCredentials::Independent(
                TextTranslationCredentials::DeepLX { token, .. }
            ) if token == "synthetic-text"));
        assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, &account), 1);
    }

    #[test]
    fn custom_speech_retains_only_its_unchanged_endpoint_key_and_never_borrows_another_profile() {
        for provider in [
            ProviderKind::CustomDashScopeASR,
            ProviderKind::CustomOpenAIASR,
        ] {
            let fake = FakeSecretStore::default();
            let store = settings(&fake);
            let profile = store.create_profile(provider, "Custom speech").unwrap();
            let other = store.create_profile(provider, "Other speech").unwrap();
            let first = custom_speech_request(
                "wss://first.example/recognition",
                "first-model",
                "synthetic-first",
            );
            store.save_credentials(&profile.id, &first).unwrap();
            let account = credential_account(&profile);
            let before = fake.value(PROFILE_KEYCHAIN_SERVICE, &account).unwrap();
            fake.make_delete_unavailable(PROFILE_KEYCHAIN_SERVICE, &account);
            assert!(store
                .save_credentials(&other.id, &custom_speech_request("", "", ""))
                .is_err());
            assert!(store
                .save_credentials(
                    &profile.id,
                    &custom_speech_request("wss://second.example/recognition", "second-model", "")
                )
                .is_err());
            assert!(store
                .save_credentials(
                    &profile.id,
                    &custom_speech_request(
                        "wss://second.example/recognition",
                        "",
                        "synthetic-new-service"
                    )
                )
                .is_err());
            assert_eq!(
                fake.value(PROFILE_KEYCHAIN_SERVICE, &account).as_deref(),
                Some(before.as_str())
            );
            store
                .save_credentials(
                    &profile.id,
                    &custom_speech_request(
                        " wss://first.example/recognition ",
                        "updated-model",
                        "",
                    ),
                )
                .unwrap();
            let credentials = store.credentials_for_profile(&profile).unwrap().unwrap();
            assert!(
                matches!(&credentials, ProviderCredentials::CustomSpeech { endpoint, model, api_key }
                if endpoint == "wss://first.example/recognition" && model == "updated-model" && api_key == "synthetic-first")
            );
            store
                .save_credentials(
                    &profile.id,
                    &custom_speech_request(
                        "wss://second.example/recognition",
                        "second-model",
                        "synthetic-second",
                    ),
                )
                .unwrap();
            assert!(
                matches!(store.credentials_for_profile(&profile).unwrap().unwrap(), ProviderCredentials::CustomSpeech { endpoint, model, api_key }
                if endpoint == "wss://second.example/recognition" && model == "second-model" && api_key == "synthetic-second")
            );
        }
    }

    #[test]
    fn custom_speech_and_each_text_route_survive_restart_in_separate_os_items() {
        for provider in [
            ProviderKind::CustomDashScopeASR,
            ProviderKind::CustomOpenAIASR,
        ] {
            let directory = tempfile::tempdir().unwrap();
            let fake = FakeSecretStore::default();
            let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
            let profile = store.create_profile(provider, "Custom speech").unwrap();
            store
                .save_credentials(
                    &profile.id,
                    &custom_speech_request(
                        "wss://speech.example/recognition",
                        "speech-model",
                        "synthetic-speech-key",
                    ),
                )
                .unwrap();
            store.select_profile(&profile.id).unwrap();
            let account = credential_account(&profile);
            let source_before = fake.value(PROFILE_KEYCHAIN_SERVICE, &account).unwrap();
            fake.make_delete_unavailable(PROFILE_KEYCHAIN_SERVICE, &account);
            for request in [
                translation_request(TextTranslation::DeepL, "", "", "synthetic-deepl:fx"),
                translation_request(
                    TextTranslation::DeepLX,
                    "",
                    "https://text.example/translate",
                    "synthetic-deeplx",
                ),
                openai_compatible_request(
                    "",
                    "https://text.example/v1",
                    "synthetic-chat-key",
                    "chat-model",
                ),
            ] {
                store.save_credentials(&profile.id, &request).unwrap();
                assert_eq!(
                    fake.value(PROFILE_KEYCHAIN_SERVICE, &account).as_deref(),
                    Some(source_before.as_str())
                );
            }
            let destination_before = fake
                .value(
                    PROFILE_KEYCHAIN_SERVICE,
                    &SettingsStore::destination_account(&profile),
                )
                .unwrap();
            for private in [
                "speech.example",
                "speech-model",
                "synthetic-speech-key",
                "text.example",
                "synthetic-deepl",
                "synthetic-chat-key",
                "chat-model",
            ] {
                assert!(!destination_before.contains("synthetic-speech-key"));
                for path in [&store.prefs_path, &store.catalog_path] {
                    assert!(!std::fs::read_to_string(path).unwrap().contains(private));
                }
            }
            drop(store);
            fake.state.lock().unwrap().loads.clear();
            let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
            for route in [
                TextTranslation::DeepL,
                TextTranslation::DeepLX,
                TextTranslation::OpenAICompatible,
                TextTranslation::FollowService,
            ] {
                store
                    .save_credentials(&profile.id, &translation_request(route, "", "", ""))
                    .unwrap();
                if route != TextTranslation::FollowService {
                    store
                        .save_preferences_for_active_profile(|prefs| {
                            prefs.target_language = TargetLanguage::English
                        })
                        .unwrap();
                }
                let config = store.configuration().unwrap();
                assert_eq!(
                    config.credentials.direct_api_key(),
                    Some("synthetic-speech-key")
                );
                assert_eq!(config.provider, provider);
                assert_eq!(
                    config.capabilities().input_sample_rate_hz,
                    provider.capabilities().input_sample_rate_hz
                );
                match route {
                    TextTranslation::DeepL => assert!(
                        matches!(config.text_credentials, Some(TextTranslationCredentials::DeepL { api_key }) if api_key == "synthetic-deepl:fx")
                    ),
                    TextTranslation::DeepLX => assert!(
                        matches!(config.text_credentials, Some(TextTranslationCredentials::DeepLX { token, .. }) if token == "synthetic-deeplx")
                    ),
                    TextTranslation::OpenAICompatible => assert!(
                        matches!(config.text_credentials, Some(TextTranslationCredentials::OpenAICompatible { api_key, model, .. }) if api_key == "synthetic-chat-key" && model == "chat-model")
                    ),
                    TextTranslation::ChatMock | TextTranslation::Apple => {
                        unreachable!("covered by independent local and ChatMock persistence tests")
                    }
                    TextTranslation::FollowService => {
                        assert_eq!(config.target_language, TargetLanguage::Original);
                        assert_eq!(config.text_credentials, None);
                    }
                }
                assert_eq!(
                    fake.value(PROFILE_KEYCHAIN_SERVICE, &account).as_deref(),
                    Some(source_before.as_str())
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
    fn custom_text_updates_preserve_speech_and_snapshots_do_not_authorize_saved_keys() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = store
            .create_profile(ProviderKind::CustomDashScopeASR, "Custom speech")
            .unwrap();
        store
            .save_credentials(
                &profile.id,
                &custom_speech_request(
                    "wss://speech.example/recognition",
                    "speech-model",
                    "synthetic-speech",
                ),
            )
            .unwrap();
        let account = credential_account(&profile);
        let before = fake.value(PROFILE_KEYCHAIN_SERVICE, &account);
        store.secret_cache.lock().unwrap().clear();
        fake.state.lock().unwrap().loads.clear();
        store
            .save_credentials(
                &profile.id,
                &openai_compatible_request(
                    "",
                    "https://text.example/v1",
                    "synthetic-text",
                    "text-model",
                ),
            )
            .unwrap();
        assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, &account), 0);
        assert_eq!(fake.value(PROFILE_KEYCHAIN_SERVICE, &account), before);
        let request = translation_request(
            TextTranslation::DeepL,
            "synthetic-forbidden-speech-update",
            "",
            "synthetic-text",
        );
        assert_eq!(
            store.save_credentials(&profile.id, &request),
            Err("custom_speech_text_update_contains_speech_key".into())
        );
        assert_eq!(fake.value(PROFILE_KEYCHAIN_SERVICE, &account), before);
        store.secret_cache.lock().unwrap().clear();
        fake.state.lock().unwrap().loads.clear();
        store.select_profile(&profile.id).unwrap();
        let snapshot = crate::commands::SettingsSnapshotPayload::from_store(&store);
        let payload = snapshot
            .profiles
            .iter()
            .find(|entry| entry.id == profile.id)
            .unwrap();
        assert_eq!(
            payload.speech_credential_state,
            Some(CredentialState::Present)
        );
        assert_eq!(
            payload.text_credential_state,
            Some(CredentialState::Present)
        );
        assert_eq!(
            fake.load_count(PROFILE_KEYCHAIN_SERVICE, &account),
            usize::from(!cfg!(target_os = "macos"))
        );
        assert_eq!(
            fake.load_count(
                PROFILE_KEYCHAIN_SERVICE,
                &SettingsStore::destination_account(&profile)
            ),
            usize::from(!cfg!(target_os = "macos"))
        );
        let configured = store.profile(&profile.id).unwrap();
        for _ in 0..2 {
            assert!(store.configuration_for_profile_probe(&configured).is_ok());
        }
        assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, &account), 1);
        assert_eq!(
            fake.load_count(
                PROFILE_KEYCHAIN_SERVICE,
                &SettingsStore::destination_account(&profile)
            ),
            1
        );
        let json = serde_json::to_string(&snapshot).unwrap();
        for private in [
            "speech.example",
            "text.example",
            "speech-model",
            "text-model",
            "synthetic-speech",
            "synthetic-text",
        ] {
            assert!(!json.contains(private));
        }
        let alibaba = snapshot
            .profiles
            .iter()
            .find(|entry| entry.provider == ProviderKind::AlibabaCloud)
            .unwrap();
        assert_eq!(alibaba.speech_credential_state, None);
        assert_eq!(alibaba.text_credential_state, None);
    }

    #[test]
    fn custom_original_recognition_does_not_read_a_missing_text_destination() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = store
            .create_profile(ProviderKind::CustomOpenAIASR, "Custom speech")
            .unwrap();
        store
            .save_credentials(
                &profile.id,
                &custom_speech_request(
                    "wss://speech.example/realtime",
                    "speech-model",
                    "synthetic-speech",
                ),
            )
            .unwrap();
        store.select_profile(&profile.id).unwrap();
        store
            .save_credentials(
                &profile.id,
                &translation_request(TextTranslation::DeepL, "", "", "synthetic-deepl:fx"),
            )
            .unwrap();
        let before_probe = store.preferences();
        let configured_profile = store.profile(&profile.id).unwrap();
        let probe = store
            .configuration_for_profile_probe(&configured_profile)
            .unwrap();
        assert_eq!(probe.target_language, TargetLanguage::English);
        assert!(
            matches!(probe.text_credentials, Some(TextTranslationCredentials::DeepL { api_key }) if api_key == "synthetic-deepl:fx")
        );
        assert_eq!(store.preferences(), before_probe);
        assert_eq!(store.active_profile().unwrap().id, profile.id);
        let destination = SettingsStore::destination_account(&profile);
        fake.state
            .lock()
            .unwrap()
            .values
            .remove(&cache_key(PROFILE_KEYCHAIN_SERVICE, &destination));
        store.secret_cache.lock().unwrap().clear();
        fake.state.lock().unwrap().loads.clear();
        assert_eq!(store.configuration().unwrap().text_credentials, None);
        assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, &destination), 0);
        let profile = store.profile(&profile.id).unwrap();
        assert_eq!(
            store.custom_credential_states(&profile),
            Some((CredentialState::Present, CredentialState::Missing))
        );
        store
            .save_preferences_for_active_profile(|prefs| {
                prefs.target_language = TargetLanguage::English
            })
            .unwrap();
        assert_eq!(
            store.configuration().unwrap_err(),
            crate::core::credentials::ProviderCredentialsError::MissingTextTranslation.to_string()
        );
    }

    #[test]
    fn custom_text_save_failures_roll_back_route_and_destination_without_touching_speech() {
        for fail_destination in [false, true] {
            let fake = FakeSecretStore::default();
            let mut store = settings(&fake);
            let profile = store
                .create_profile(ProviderKind::CustomDashScopeASR, "Custom speech")
                .unwrap();
            store
                .save_credentials(
                    &profile.id,
                    &custom_speech_request(
                        "wss://speech.example/recognition",
                        "speech-model",
                        "synthetic-speech",
                    ),
                )
                .unwrap();
            store.select_profile(&profile.id).unwrap();
            store
                .save_credentials(
                    &profile.id,
                    &translation_request(TextTranslation::DeepL, "", "", "synthetic-old:fx"),
                )
                .unwrap();
            store
                .save_preferences_for_active_profile(|prefs| {
                    prefs.target_language = TargetLanguage::English
                })
                .unwrap();
            let account = credential_account(&profile);
            let destination = SettingsStore::destination_account(&profile);
            let before_speech = fake.value(PROFILE_KEYCHAIN_SERVICE, &account);
            let before_text = fake.value(PROFILE_KEYCHAIN_SERVICE, &destination);
            let before_preferences = store.preferences();
            let blocked = tempfile::tempdir().unwrap();
            if fail_destination {
                fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &destination);
            } else {
                store.catalog_path = blocked.path().into();
            }
            fake.make_delete_unavailable(PROFILE_KEYCHAIN_SERVICE, &account);
            assert!(store
                .save_credentials(
                    &profile.id,
                    &openai_compatible_request(
                        "",
                        "https://new-text.example/v1",
                        "synthetic-new",
                        "new-text-model"
                    )
                )
                .is_err());
            assert_eq!(
                store.profile(&profile.id).unwrap().text_translation(),
                TextTranslation::DeepL
            );
            assert_eq!(store.preferences(), before_preferences);
            assert_eq!(
                fake.value(PROFILE_KEYCHAIN_SERVICE, &account),
                before_speech
            );
            assert_eq!(
                fake.value(PROFILE_KEYCHAIN_SERVICE, &destination),
                before_text
            );
        }
    }

    #[test]
    fn custom_credential_deletion_rolls_back_text_when_speech_delete_fails_then_removes_both() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = store
            .create_profile(ProviderKind::CustomOpenAIASR, "Custom speech")
            .unwrap();
        store
            .save_credentials(
                &profile.id,
                &custom_speech_request(
                    "wss://speech.example/realtime",
                    "speech-model",
                    "synthetic-speech",
                ),
            )
            .unwrap();
        store
            .save_credentials(
                &profile.id,
                &translation_request(TextTranslation::DeepL, "", "", "synthetic-text:fx"),
            )
            .unwrap();
        let account = credential_account(&profile);
        let destination = SettingsStore::destination_account(&profile);
        let before_text = fake.value(PROFILE_KEYCHAIN_SERVICE, &destination);
        fake.make_delete_unavailable(PROFILE_KEYCHAIN_SERVICE, &account);
        assert!(store.delete_api_key(&profile.id).is_err());
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, &destination),
            before_text
        );
        assert!(fake.value(PROFILE_KEYCHAIN_SERVICE, &account).is_some());
        fake.state.lock().unwrap().unavailable_deletes.clear();
        store.delete_api_key(&profile.id).unwrap();
        assert_eq!(fake.value(PROFILE_KEYCHAIN_SERVICE, &account), None);
        assert_eq!(fake.value(PROFILE_KEYCHAIN_SERVICE, &destination), None);
    }

    #[test]
    fn custom_speech_failed_readback_restores_the_existing_item_in_place() {
        #[derive(Clone)]
        struct FailOnceReadback {
            inner: FakeSecretStore,
            rejected_value: Arc<Mutex<Option<String>>>,
        }
        impl SecretStore for FailOnceReadback {
            fn load(
                &self,
                service: &str,
                account: &str,
            ) -> Result<Option<String>, SecretStoreError> {
                let value = self.inner.load(service, account)?;
                let mut rejected = self.rejected_value.lock().unwrap();
                if rejected.is_some() && rejected.as_deref() == value.as_deref() {
                    *rejected = None;
                    return Err(SecretStoreError::Unavailable);
                }
                Ok(value)
            }
            fn save(
                &self,
                service: &str,
                account: &str,
                value: &str,
            ) -> Result<(), SecretStoreError> {
                self.inner.save(service, account, value)
            }
            fn delete(&self, service: &str, account: &str) -> Result<(), SecretStoreError> {
                self.inner.delete(service, account)
            }
        }
        let fake = FakeSecretStore::default();
        let rejected_value = Arc::new(Mutex::new(None));
        let store = SettingsStore::in_memory(
            Box::new(FailOnceReadback {
                inner: fake.clone(),
                rejected_value: Arc::clone(&rejected_value),
            }),
            false,
        );
        let profile = store
            .create_profile(ProviderKind::CustomDashScopeASR, "Custom speech")
            .unwrap();
        store
            .save_credentials(
                &profile.id,
                &custom_speech_request(
                    "wss://speech.example/recognition",
                    "old-model",
                    "synthetic-old-key",
                ),
            )
            .unwrap();
        let account = credential_account(&profile);
        let previous = fake.value(PROFILE_KEYCHAIN_SERVICE, &account);
        fake.make_delete_unavailable(PROFILE_KEYCHAIN_SERVICE, &account);
        let replacement = custom_speech_request(
            "wss://speech.example/recognition",
            "new-model",
            "synthetic-new-key",
        );
        *rejected_value.lock().unwrap() =
            Some(replacement.encode_for_keychain(profile.provider).unwrap());
        assert_eq!(
            store.save_credentials(&profile.id, &replacement),
            Err(CREDENTIAL_STORE_UNAVAILABLE.into())
        );
        assert_eq!(fake.value(PROFILE_KEYCHAIN_SERVICE, &account), previous);
        assert_eq!(
            store
                .credentials_for_profile(&profile)
                .unwrap()
                .unwrap()
                .direct_api_key(),
            Some("synthetic-old-key")
        );
    }

    #[test]
    fn local_dev_preset_keeps_other_profiles_and_text_destinations_in_the_os_store() {
        let fake = FakeSecretStore::default();
        let directory = tempfile::tempdir().unwrap();
        let store = SettingsStore::load_with_secret(
            directory.path().into(),
            false,
            local_dev_credentials::test_store(
                Ok(Some("synthetic-file-key".into())),
                Box::new(fake.clone()),
            ),
            DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE,
            false,
        );
        let preset = store.active_profile().unwrap();
        assert_eq!(preset.id, LOCAL_DEV_ALIBABA_PROFILE_ID);
        assert_eq!(store.credential_state(&preset), CredentialState::Present);
        assert_eq!(store.profile_credential_storage(&preset.id), "localDevFile");
        let draft = openai_compatible_request("", "https://draft.example/v1", "synthetic", "model");
        assert_eq!(
            store
                .configuration_for_text_draft_probe(&preset, &draft)
                .unwrap_err(),
            "local_dev_credentials_read_only"
        );
        assert_eq!(
            store
                .configuration_for_speech_draft_probe(
                    &preset,
                    &ProviderCredentials::api_key("synthetic"),
                    true
                )
                .unwrap_err(),
            "local_dev_credentials_read_only"
        );
        assert_eq!(
            store.update_profile_options(
                &preset.id,
                None,
                None,
                None,
                Some(TextTranslationName {
                    route: TextTranslation::OpenAICompatible,
                    name: "Read-only alias".into(),
                }),
                None,
                None,
                None,
            ),
            Err("local_dev_credentials_read_only".into())
        );
        assert!(store
            .profile(&preset.id)
            .unwrap()
            .text_translation_names
            .is_empty());
        assert_eq!(
            store.update_profile_options(
                &preset.id,
                None,
                None,
                None,
                None,
                Some("Read-only recognition"),
                None,
                None,
            ),
            Err("local_dev_credentials_read_only".into())
        );
        assert!(store
            .profile(&preset.id)
            .unwrap()
            .speech_recognition_name
            .is_none());
        assert_eq!(
            store.save_credentials(
                &preset.id,
                &ProviderCredentials::api_key("synthetic-replacement")
            ),
            Err("local_dev_credentials_read_only".into())
        );
        assert_eq!(
            store.delete_profile(&preset.id),
            Err("local_dev_credentials_read_only".into())
        );
        assert_eq!(
            store.reveal_credential(&preset.id, CredentialRevealField::ApiKey, None),
            Err("local_dev_credentials_read_only".into())
        );

        let regular = store
            .create_profile(ProviderKind::AlibabaCloud, "Regular Alibaba")
            .unwrap();
        assert_eq!(store.credential_state(&regular), CredentialState::Missing);
        store
            .save_credentials(
                &regular.id,
                &ProviderCredentials::api_key("synthetic-regular-asr"),
            )
            .unwrap();
        store.select_profile(&regular.id).unwrap();
        for request in [
            translation_request(TextTranslation::DeepL, "", "", "synthetic-deepl:fx"),
            translation_request(
                TextTranslation::DeepLX,
                "",
                "http://127.0.0.1:8001/translate",
                "",
            ),
            openai_compatible_request(
                "",
                "https://text.example/v1",
                "synthetic-text-key",
                "text-model",
            ),
            ProviderCredentials::AlibabaTranslation {
                api_key: String::new(),
                text_translation: TextTranslation::ChatMock,
                endpoint: "http://127.0.0.1:8000/v1".into(),
                token: String::new(),
                model: "chat-model".into(),
                clear_token: false,
            },
            translation_request(TextTranslation::FollowService, "", "", ""),
        ] {
            store.save_credentials(&regular.id, &request).unwrap();
            assert!(store.configuration().is_ok());
        }
        assert_eq!(
            store
                .reveal_credential(&regular.id, CredentialRevealField::ApiKey, None)
                .unwrap()
                .as_deref(),
            Some("synthetic-regular-asr")
        );
        let account = credential_account(&regular);
        assert_eq!(
            fake.value(DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE, &account)
                .as_deref(),
            Some("synthetic-regular-asr")
        );
        assert!(fake.value(PROFILE_KEYCHAIN_SERVICE, &account).is_none());
        assert!(fake
            .value(
                DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE,
                &credential_account(&preset)
            )
            .is_none());
        for path in [&store.prefs_path, &store.catalog_path] {
            let metadata = std::fs::read_to_string(path).unwrap();
            assert!(!metadata.contains("synthetic-file-key"));
            assert!(!metadata.contains("synthetic-regular-asr"));
            assert!(!metadata.contains("synthetic-text-key"));
        }
        drop(store);
        let reopened = SettingsStore::load_with_secret(
            directory.path().into(),
            false,
            local_dev_credentials::test_store(
                Ok(Some("synthetic-file-key".into())),
                Box::new(fake.clone()),
            ),
            DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE,
            false,
        );
        assert_eq!(reopened.active_profile().unwrap().id, regular.id);
        assert!(reopened.configuration().is_ok());
        reopened.select_profile(&preset.id).unwrap();
        assert!(reopened.configuration().is_ok());
        drop(reopened);
        let without_file = SettingsStore::load_with_secret(
            directory.path().into(),
            false,
            Box::new(fake.clone()),
            DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE,
            false,
        );
        assert!(without_file
            .profile_catalog()
            .unwrap()
            .1
            .iter()
            .all(|p| p.id != LOCAL_DEV_ALIBABA_PROFILE_ID));
        assert_eq!(
            fake.value(DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE, &account)
                .as_deref(),
            Some("synthetic-regular-asr")
        );
    }

    #[test]
    fn an_invalid_local_dev_file_does_not_lock_or_supply_other_provider_profiles() {
        let fake = FakeSecretStore::default();
        let store = SettingsStore::in_memory_with_scope(
            local_dev_credentials::test_store(
                Err(SecretStoreError::LocalDevFileUnavailable),
                Box::new(fake.clone()),
            ),
            false,
            DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE,
            false,
        );
        let preset = store.active_profile().unwrap();
        fake.put(
            DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE,
            &credential_account(&preset),
            "synthetic-must-not-fallback",
        );
        assert_eq!(
            store.configuration().unwrap_err(),
            "local_dev_credentials_unavailable"
        );
        let regular = store
            .create_profile(ProviderKind::OpenAIRealtime, "Regular OpenAI")
            .unwrap();
        store
            .save_credentials(
                &regular.id,
                &ProviderCredentials::api_key("synthetic-openai"),
            )
            .unwrap();
        store.select_profile(&regular.id).unwrap();
        assert!(store.configuration().is_ok());
        assert_eq!(store.profile_credential_storage(&regular.id), "keychain");
        store.delete_api_key(&regular.id).unwrap();
        assert_eq!(store.credential_state(&regular), CredentialState::Missing);
        assert_eq!(
            store.credential_state(&preset),
            CredentialState::Unavailable
        );
    }

    #[test]
    fn the_local_dev_preset_does_not_consume_a_user_profile_slot() {
        let fake = FakeSecretStore::default();
        let store = SettingsStore::in_memory_with_scope(
            local_dev_credentials::test_store(Ok(None), Box::new(fake)),
            false,
            DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE,
            false,
        );
        for _ in 1..MAXIMUM_PROFILE_COUNT {
            store
                .create_profile(ProviderKind::AlibabaCloud, "Regular")
                .unwrap();
        }
        assert_eq!(
            store.profile_catalog().unwrap().1.len(),
            MAXIMUM_PROFILE_COUNT + 1
        );
        assert!(store
            .create_profile(ProviderKind::AlibabaCloud, "Too many")
            .is_err());
    }

    #[test]
    fn deleting_the_last_regular_dev_profile_does_not_restore_it_on_restart() {
        let directory = tempfile::tempdir().unwrap();
        atomic_write(
            &directory.path().join(PROFILE_CATALOG_FILE),
            &serde_json::to_vec(&ProfileCatalog::legacy_alibaba()).unwrap(),
        )
        .unwrap();
        let fake = FakeSecretStore::default();
        let load = || {
            SettingsStore::load_with_secret(
                directory.path().into(),
                false,
                local_dev_credentials::test_store(
                    Ok(Some("synthetic-file-key".into())),
                    Box::new(fake.clone()),
                ),
                DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE,
                false,
            )
        };
        let store = load();
        store.delete_profile(DEFAULT_ALIBABA_PROFILE_ID).unwrap();
        assert_eq!(store.profile_catalog().unwrap().1.len(), 1);
        drop(store);
        let reopened = load();
        assert_eq!(reopened.profile_catalog().unwrap().1.len(), 1);
        assert_eq!(
            reopened.active_profile().unwrap().id,
            LOCAL_DEV_ALIBABA_PROFILE_ID
        );
    }

    #[test]
    fn custom_speech_never_reads_the_local_development_file_or_falls_back_to_os_credentials() {
        struct ReadOnlyCredentials;
        impl SecretStore for ReadOnlyCredentials {
            fn load(&self, _: &str, _: &str) -> Result<Option<String>, SecretStoreError> {
                panic!("custom speech must not read the Alibaba-only dev file or fall back")
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
        for provider in [
            ProviderKind::CustomDashScopeASR,
            ProviderKind::CustomOpenAIASR,
        ] {
            let mut profile = store.create_profile(provider, "Custom speech").unwrap();
            assert_eq!(
                store.custom_credential_states(&profile),
                Some((CredentialState::Missing, CredentialState::Present))
            );
            profile.text_translation = Some(TextTranslation::DeepL);
            assert_eq!(
                store.custom_credential_states(&profile),
                Some((CredentialState::Missing, CredentialState::Missing))
            );
            assert!(store.credentials_for_profile(&profile).unwrap().is_none());
            assert_eq!(
                store.save_credentials(
                    &profile.id,
                    &custom_speech_request("wss://speech.example", "speech-model", "synthetic")
                ),
                Err("local_dev_credentials_read_only".into())
            );
        }
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
    fn credential_editor_state_distinguishes_anonymous_text_from_saved_speech_key() {
        for route in [TextTranslation::OpenAICompatible, TextTranslation::ChatMock] {
            let fake = FakeSecretStore::default();
            let store = settings(&fake);
            let profile = ServiceProfile::alibaba_default();
            store
                .save_api_key(&profile.id, "synthetic-speech-secret")
                .unwrap();
            let request = ProviderCredentials::AlibabaTranslation {
                api_key: String::new(),
                text_translation: route,
                endpoint: "http://127.0.0.1:8000/v1".into(),
                token: String::new(),
                model: "local-model".into(),
                clear_token: false,
            };
            store.save_credentials(&profile.id, &request).unwrap();
            let state = store
                .credential_editor_state(&profile.id, Some(route))
                .unwrap();
            assert!(state.saved_fields.is_empty());
            assert_eq!(
                state.endpoint.as_deref(),
                Some("http://127.0.0.1:8000/v1/chat/completions")
            );
            assert_eq!(state.model.as_deref(), Some("local-model"));
            let speech = store.credential_editor_state(&profile.id, None).unwrap();
            assert_eq!(
                speech.saved_fields,
                vec![
                    CredentialRevealField::ApiKey,
                    CredentialRevealField::AsrApiKey
                ]
            );
            assert!(speech.endpoint.is_none());
            assert!(speech.model.is_none());
            assert!(!serde_json::to_string(&speech)
                .unwrap()
                .contains("synthetic-speech-secret"));

            let mut authenticated = request.clone();
            let ProviderCredentials::AlibabaTranslation { token, .. } = &mut authenticated else {
                unreachable!()
            };
            *token = "synthetic-text-secret".into();
            store.save_credentials(&profile.id, &authenticated).unwrap();
            let state = store
                .credential_editor_state(&profile.id, Some(route))
                .unwrap();
            assert_eq!(state.saved_fields, vec![CredentialRevealField::Token]);
            let json = serde_json::to_string(&state).unwrap();
            assert!(!json.contains("synthetic-speech-secret"));
            assert!(!json.contains("synthetic-text-secret"));

            let mut anonymous = request;
            let ProviderCredentials::AlibabaTranslation { clear_token, .. } = &mut anonymous else {
                unreachable!()
            };
            *clear_token = true;
            store.save_credentials(&profile.id, &anonymous).unwrap();
            assert!(store
                .credential_editor_state(&profile.id, Some(route))
                .unwrap()
                .saved_fields
                .is_empty());
            let snapshot = serde_json::to_string(
                &crate::commands::SettingsSnapshotPayload::from_store(&store),
            )
            .unwrap();
            for private in [
                "synthetic-speech-secret",
                "synthetic-text-secret",
                "127.0.0.1:8000",
                "local-model",
            ] {
                assert!(!snapshot.contains(private));
            }
        }
    }

    #[test]
    fn credential_editor_state_reads_only_requested_slot_and_rejects_stale_route() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = ServiceProfile::alibaba_default();
        store
            .save_credentials(
                &profile.id,
                &openai_compatible_request(
                    "synthetic-speech-secret",
                    "http://127.0.0.1:8000/v1",
                    "",
                    "local-model",
                ),
            )
            .unwrap();
        store.secret_cache.lock().unwrap().clear();
        fake.state.lock().unwrap().loads.clear();
        assert_eq!(
            store
                .credential_editor_state(&profile.id, Some(TextTranslation::DeepLX))
                .err()
                .as_deref(),
            Some("credential_reveal_field_mismatch")
        );
        assert!(fake.state.lock().unwrap().loads.is_empty());
        let speech_account = credential_account(&profile);
        let text_account = SettingsStore::destination_account(&profile);
        fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &speech_account);
        assert!(store
            .credential_editor_state(&profile.id, Some(TextTranslation::OpenAICompatible))
            .is_ok());
        assert_eq!(
            fake.load_count(PROFILE_KEYCHAIN_SERVICE, &speech_account),
            0
        );
        assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, &text_account), 1);
        assert_eq!(
            store
                .credential_editor_state(&profile.id, None)
                .err()
                .as_deref(),
            Some(CREDENTIAL_STORE_UNAVAILABLE)
        );
    }

    #[test]
    fn credential_editor_state_ui_fixtures_never_access_real_credentials() {
        let fake = FakeSecretStore::default();
        let store = SettingsStore::in_memory(Box::new(fake.clone()), true);
        let profile = ServiceProfile::alibaba_default();
        let speech = store.credential_editor_state(&profile.id, None).unwrap();
        assert!(speech.saved_fields.contains(&CredentialRevealField::ApiKey));
        assert_eq!(
            store.reveal_credential(&profile.id, CredentialRevealField::ApiKey, None),
            Ok(Some("sk-demo-not-a-real-key".into()))
        );
        store
            .save_credentials(
                &profile.id,
                &openai_compatible_request("", "http://127.0.0.1:8000/v1", "", "local-model"),
            )
            .unwrap();
        let text = store
            .credential_editor_state(&profile.id, Some(TextTranslation::OpenAICompatible))
            .unwrap();
        assert!(text.saved_fields.is_empty());
        assert_eq!(
            text.endpoint.as_deref(),
            Some("http://127.0.0.1:8000/v1/chat/completions")
        );
        assert_eq!(text.model.as_deref(), Some("local-model"));
        let state = fake.state.lock().unwrap();
        assert!(state.loads.is_empty());
        assert!(state.values.is_empty());
    }

    #[test]
    fn credential_editor_state_hydrates_provider_fields_without_secret_values() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        for (provider, credentials, expected) in [
            (
                ProviderKind::CustomOpenAIASR,
                custom_speech_request(
                    "wss://speech.example/realtime",
                    "speech-model",
                    "synthetic-speech-secret",
                ),
                serde_json::json!({"savedFields": ["apiKey", "asrApiKey"], "endpoint": "wss://speech.example/realtime", "model": "speech-model"}),
            ),
            (
                ProviderKind::AzureOpenAIRealtime,
                ProviderCredentials::AzureOpenAI {
                    endpoint: "https://sample.openai.azure.com".into(),
                    deployment: "realtime".into(),
                    transcription_deployment: "transcribe".into(),
                    api_key: "synthetic-azure-secret".into(),
                },
                serde_json::json!({"savedFields": ["apiKey"], "endpoint": "https://sample.openai.azure.com", "deployment": "realtime", "transcriptionDeployment": "transcribe"}),
            ),
            (
                ProviderKind::TencentCloud,
                ProviderCredentials::TencentCloud {
                    app_id: "12345".into(),
                    secret_id: "synthetic-tencent-id".into(),
                    secret_key: "synthetic-tencent-secret".into(),
                },
                serde_json::json!({"savedFields": ["secretId", "secretKey"], "appId": "12345"}),
            ),
            (
                ProviderKind::BaiduTranslate,
                ProviderCredentials::BaiduTranslate {
                    app_id: "67890".into(),
                    app_key: "synthetic-baidu-secret".into(),
                },
                serde_json::json!({"savedFields": ["appKey"], "appId": "67890"}),
            ),
        ] {
            let profile = store
                .create_profile(provider, "Saved editor fields")
                .unwrap();
            store.save_credentials(&profile.id, &credentials).unwrap();
            let state = store.credential_editor_state(&profile.id, None).unwrap();
            assert_eq!(serde_json::to_value(state).unwrap(), expected);
        }
    }

    #[test]
    fn credential_editor_state_supports_legacy_deeplx_and_keeps_private_paths_out_of_snapshots() {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = store
            .create_profile(ProviderKind::DeepLX, "Legacy text destination")
            .unwrap();
        store
            .save_credentials(
                &profile.id,
                &ProviderCredentials::DeepLX {
                    asr_api_key: "synthetic-speech-secret".into(),
                    endpoint: "https://example.com/private-path/translate".into(),
                    token: String::new(),
                },
            )
            .unwrap();
        let state = store
            .credential_editor_state(&profile.id, Some(TextTranslation::DeepLX))
            .unwrap();
        assert!(state.saved_fields.is_empty());
        assert_eq!(
            state.endpoint.as_deref(),
            Some("https://example.com/private-path/translate")
        );
        let snapshot = serde_json::to_string(
            &crate::commands::SettingsSnapshotPayload::from_store(&store),
        )
        .unwrap();
        assert!(!snapshot.contains("private-path"));
        assert!(!snapshot.contains("synthetic-speech-secret"));
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
            let profile = store
                .create_profile(ProviderKind::AlibabaCloud, "Recovery")
                .unwrap();
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
            clear_token: false,
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
            clear_token: false,
        }
    }

    fn chatmock_request(endpoint: &str, key: &str, model: &str) -> ProviderCredentials {
        ProviderCredentials::AlibabaTranslation {
            api_key: String::new(),
            text_translation: TextTranslation::ChatMock,
            endpoint: endpoint.into(),
            token: key.into(),
            model: model.into(),
            clear_token: false,
        }
    }

    #[test]
    fn chatmock_and_generic_destinations_stay_independent_across_save_reveal_switch_and_restart() {
        use crate::core::configuration::TextTranslationProbeCredentials;
        for provider in [
            ProviderKind::AlibabaCloud,
            ProviderKind::CustomDashScopeASR,
            ProviderKind::CustomOpenAIASR,
        ] {
            let directory = tempfile::tempdir().unwrap();
            let fake = FakeSecretStore::default();
            let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
            let profile = store
                .create_profile(provider, "Independent destinations")
                .unwrap();
            let speech = if provider.is_custom_speech() {
                custom_speech_request(
                    "wss://speech.example/realtime",
                    "speech-model",
                    "synthetic-speech",
                )
            } else {
                ProviderCredentials::api_key("synthetic-speech")
            };
            store.save_credentials(&profile.id, &speech).unwrap();
            let speech_before = fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile));
            let destination_account = SettingsStore::destination_account(&profile);
            // Identical endpoint makes route isolation independent of the address-change guard.
            store
                .save_credentials(
                    &profile.id,
                    &openai_compatible_request(
                        "",
                        "http://127.0.0.1:8000/v1",
                        "synthetic-generic",
                        "generic-model",
                    ),
                )
                .unwrap();
            let old_destination = fake
                .value(PROFILE_KEYCHAIN_SERVICE, &destination_account)
                .unwrap();
            assert!(!old_destination.contains("chat_mock"));
            // Old generic settings cannot provide ChatMock's address, model, or key.
            assert!(store
                .save_credentials(&profile.id, &chatmock_request("", "", ""))
                .is_err());
            assert_eq!(
                fake.value(PROFILE_KEYCHAIN_SERVICE, &destination_account)
                    .as_deref(),
                Some(old_destination.as_str())
            );
            assert_eq!(
                store.profile(&profile.id).unwrap().text_translation(),
                TextTranslation::OpenAICompatible
            );
            store
                .save_credentials(
                    &profile.id,
                    &chatmock_request("http://127.0.0.1:8000/v1", "", "chat-model"),
                )
                .unwrap();
            let current = store.profile(&profile.id).unwrap();
            let probe = store.configuration_for_text_probe(&current).unwrap();
            assert!(
                matches!(probe.credentials, TextTranslationProbeCredentials::Independent(TextTranslationCredentials::ChatMock { api_key, model, .. }) if api_key.is_empty() && model == "chat-model")
            );
            assert_eq!(
                store.reveal_credential(
                    &profile.id,
                    CredentialRevealField::Token,
                    Some(TextTranslation::ChatMock)
                ),
                Ok(None)
            );
            assert!(store
                .reveal_credential(
                    &profile.id,
                    CredentialRevealField::Token,
                    Some(TextTranslation::OpenAICompatible)
                )
                .is_err());
            store
                .save_credentials(&profile.id, &chatmock_request("", "synthetic-chatmock", ""))
                .unwrap();
            let saved = fake
                .value(PROFILE_KEYCHAIN_SERVICE, &destination_account)
                .unwrap();
            assert!(saved.contains("open_ai_compatible") && saved.contains("chat_mock"));
            for path in [&store.prefs_path, &store.catalog_path] {
                let public = std::fs::read_to_string(path).unwrap_or_default();
                for private in [
                    "synthetic-generic",
                    "synthetic-chatmock",
                    "synthetic-speech",
                    "127.0.0.1",
                    "chat-model",
                    "generic-model",
                ] {
                    assert!(!public.contains(private));
                }
            }
            drop(store);
            let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
            for route in [
                TextTranslation::OpenAICompatible,
                TextTranslation::ChatMock,
                TextTranslation::OpenAICompatible,
                TextTranslation::ChatMock,
            ] {
                store
                    .save_credentials(&profile.id, &translation_request(route, "", "", ""))
                    .unwrap();
                let current = store.profile(&profile.id).unwrap();
                let expected = if route == TextTranslation::ChatMock {
                    "synthetic-chatmock"
                } else {
                    "synthetic-generic"
                };
                assert_eq!(
                    store
                        .reveal_credential(&profile.id, CredentialRevealField::Token, Some(route))
                        .unwrap()
                        .as_deref(),
                    Some(expected)
                );
                let text = store
                    .text_credentials_for_profile(&current)
                    .unwrap()
                    .unwrap();
                assert_eq!(text.translation(), route);
                let model = match text {
                    TextTranslationCredentials::ChatMock { model, .. }
                    | TextTranslationCredentials::OpenAICompatible { model, .. } => model,
                    _ => unreachable!(),
                };
                assert_eq!(
                    model,
                    if route == TextTranslation::ChatMock {
                        "chat-model"
                    } else {
                        "generic-model"
                    }
                );
                assert_eq!(
                    fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile)),
                    speech_before
                );
            }
            let clear = ProviderCredentials::AlibabaTranslation {
                api_key: String::new(),
                text_translation: TextTranslation::ChatMock,
                endpoint: String::new(),
                token: String::new(),
                model: String::new(),
                clear_token: true,
            };
            store.save_credentials(&profile.id, &clear).unwrap();
            assert_eq!(
                store.reveal_credential(
                    &profile.id,
                    CredentialRevealField::Token,
                    Some(TextTranslation::ChatMock)
                ),
                Ok(None)
            );
            store
                .save_credentials(&profile.id, &openai_compatible_request("", "", "", ""))
                .unwrap();
            assert_eq!(
                store.reveal_credential(
                    &profile.id,
                    CredentialRevealField::Token,
                    Some(TextTranslation::OpenAICompatible)
                ),
                Ok(Some("synthetic-generic".into()))
            );
            assert_eq!(
                fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile)),
                speech_before
            );
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
                    TextTranslation::ChatMock | TextTranslation::Apple => {
                        unreachable!("covered by independent local and ChatMock persistence tests")
                    }
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
    fn anonymous_openai_compatible_destination_survives_restart_and_probes_without_speech_keys() {
        use crate::core::configuration::TextTranslationProbeCredentials;
        for provider in [
            ProviderKind::AlibabaCloud,
            ProviderKind::CustomDashScopeASR,
            ProviderKind::CustomOpenAIASR,
        ] {
            let directory = tempfile::tempdir().unwrap();
            let fake = FakeSecretStore::default();
            let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
            let profile = store.create_profile(provider, "Anonymous text").unwrap();
            let speech = if provider.is_custom_speech() {
                custom_speech_request(
                    "wss://speech.example/realtime",
                    "speech-model",
                    "synthetic-speech",
                )
            } else {
                ProviderCredentials::api_key("synthetic-speech")
            };
            store.save_credentials(&profile.id, &speech).unwrap();
            store
                .save_credentials(
                    &profile.id,
                    &openai_compatible_request("", "http://127.0.0.1:8000/v1", "", "local-model"),
                )
                .unwrap();
            let speech_account = credential_account(&profile);
            let destination_account = SettingsStore::destination_account(&profile);
            let speech_before = fake.value(PROFILE_KEYCHAIN_SERVICE, &speech_account);
            let destination = fake
                .value(PROFILE_KEYCHAIN_SERVICE, &destination_account)
                .unwrap();
            assert!(!destination.contains("synthetic-speech"));
            drop(store);
            fake.state.lock().unwrap().loads.clear();
            let reloaded = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
            let profile = reloaded.profile(&profile.id).unwrap();
            assert_eq!(
                profile.text_translation(),
                TextTranslation::OpenAICompatible
            );
            fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &speech_account);
            let probe = reloaded.configuration_for_text_probe(&profile).unwrap();
            assert!(
                matches!(probe.credentials, TextTranslationProbeCredentials::Independent(
                TextTranslationCredentials::OpenAICompatible { endpoint, api_key, model }
            ) if endpoint == "http://127.0.0.1:8000/v1/chat/completions" && api_key.is_empty() && model == "local-model")
            );
            assert_eq!(
                fake.load_count(PROFILE_KEYCHAIN_SERVICE, &speech_account),
                0
            );
            assert_eq!(
                fake.load_count(PROFILE_KEYCHAIN_SERVICE, &destination_account),
                1
            );
            assert_eq!(
                fake.value(PROFILE_KEYCHAIN_SERVICE, &speech_account),
                speech_before
            );
        }
    }

    #[test]
    fn openai_compatible_explicit_key_removal_preserves_endpoint_model_and_other_secrets() {
        for provider in [
            ProviderKind::AlibabaCloud,
            ProviderKind::CustomDashScopeASR,
            ProviderKind::CustomOpenAIASR,
        ] {
            let fake = FakeSecretStore::default();
            let store = settings(&fake);
            let profile = store.create_profile(provider, "Remove text key").unwrap();
            let speech = if provider.is_custom_speech() {
                custom_speech_request(
                    "wss://speech.example/realtime",
                    "speech-model",
                    "synthetic-speech",
                )
            } else {
                ProviderCredentials::api_key("synthetic-speech")
            };
            store.save_credentials(&profile.id, &speech).unwrap();
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
                        "http://127.0.0.1:8000/v1",
                        "synthetic-reverse-proxy",
                        "local-model",
                    ),
                )
                .unwrap();
            let speech_before = fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile));
            let destination_account = SettingsStore::destination_account(&profile);
            let before = fake.value(PROFILE_KEYCHAIN_SERVICE, &destination_account);
            // Contradictory or unrelated removal requests fail before any mutation.
            for (route, token) in [
                (TextTranslation::OpenAICompatible, "synthetic-new-key"),
                (TextTranslation::DeepL, ""),
                (TextTranslation::DeepLX, ""),
                (TextTranslation::FollowService, ""),
            ] {
                let request = ProviderCredentials::AlibabaTranslation {
                    api_key: String::new(),
                    text_translation: route,
                    endpoint: String::new(),
                    token: token.into(),
                    model: String::new(),
                    clear_token: true,
                };
                assert!(store.save_credentials(&profile.id, &request).is_err());
                assert_eq!(
                    fake.value(PROFILE_KEYCHAIN_SERVICE, &destination_account),
                    before
                );
            }
            let request = ProviderCredentials::AlibabaTranslation {
                api_key: String::new(),
                text_translation: TextTranslation::OpenAICompatible,
                endpoint: String::new(),
                token: String::new(),
                model: String::new(),
                clear_token: true,
            };
            store.save_credentials(&profile.id, &request).unwrap();
            let profile = store.profile(&profile.id).unwrap();
            let text = store
                .text_credentials_for_profile(&profile)
                .unwrap()
                .unwrap();
            assert!(
                matches!(text, TextTranslationCredentials::OpenAICompatible { endpoint, api_key, model }
                if endpoint == "http://127.0.0.1:8000/v1/chat/completions" && api_key.is_empty() && model == "local-model")
            );
            let destination = fake
                .value(PROFILE_KEYCHAIN_SERVICE, &destination_account)
                .unwrap();
            assert!(!destination.contains("synthetic-reverse-proxy"));
            assert!(destination.contains("synthetic-deepl:fx"));
            assert_eq!(
                fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile)),
                speech_before
            );
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
    fn openai_compatible_model_updates_retain_keys_but_new_destinations_never_inherit_them() {
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
        store
            .save_credentials(
                &profile.id,
                &openai_compatible_request("", "", "", "updated-model"),
            )
            .unwrap();
        let config = store.configuration().unwrap();
        assert!(
            matches!(&config.credentials, ProviderCredentials::OpenAICompatible {
            endpoint, api_key, model, ..
        } if endpoint.starts_with("https://first.example/") && api_key == "synthetic-first" && model == "updated-model")
        );
        store
            .save_credentials(
                &profile.id,
                &openai_compatible_request("", "http://127.0.0.1:8000/v1", "", "local-model"),
            )
            .unwrap();
        let config = store.configuration().unwrap();
        assert!(
            matches!(&config.credentials, ProviderCredentials::OpenAICompatible {
            asr_api_key, endpoint, api_key, model,
        } if asr_api_key == "synthetic-asr" && endpoint == "http://127.0.0.1:8000/v1/chat/completions" && api_key.is_empty() && model == "local-model")
        );
        assert!(!fake
            .value(
                PROFILE_KEYCHAIN_SERVICE,
                &SettingsStore::destination_account(&profile)
            )
            .unwrap()
            .contains("synthetic-first"));
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
                TextTranslation::OpenAICompatible
                | TextTranslation::ChatMock
                | TextTranslation::Apple => unreachable!(),
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
        assert_eq!(store.profile_catalog().unwrap(), (String::new(), vec![]));
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
                preferences.subtitle_font_family = "Noto Sans CJK JP".into();
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
    fn explicit_alibaba_save_retries_failed_reads_without_a_diagnostic() {
        for route in [TextTranslation::FollowService, TextTranslation::Apple] {
            let fake = FakeSecretStore::default();
            let store = settings(&fake);
            let profile = store.active_profile().unwrap();
            let account = credential_account(&profile);
            fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &account);
            let request = translation_request(route, "synthetic-key", "", "");
            assert_eq!(
                store.save_credentials(&profile.id, &request),
                Err(CREDENTIAL_STORE_UNAVAILABLE.into())
            );
            fake.state.lock().unwrap().unavailable.clear();
            store.save_credentials(&profile.id, &request).unwrap();
            assert_eq!(store.credential_state(&profile), CredentialState::Present);
            if route == TextTranslation::Apple {
                assert_eq!(
                    fake.load_count(
                        PROFILE_KEYCHAIN_SERVICE,
                        &SettingsStore::destination_account(&profile),
                    ),
                    0
                );
            }
        }
    }

    #[test]
    fn explicit_custom_save_retries_only_its_failed_stage() {
        for speech_save in [false, true] {
            let fake = FakeSecretStore::default();
            let store = settings(&fake);
            let profile = store
                .create_profile(ProviderKind::CustomOpenAIASR, "Synthetic custom")
                .unwrap();
            let speech = credential_account(&profile);
            let text = SettingsStore::destination_account(&profile);
            for account in [&speech, &text] {
                fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, account);
                assert!(store
                    .load_secret(PROFILE_KEYCHAIN_SERVICE, account)
                    .is_err());
            }
            let request = if speech_save {
                ProviderCredentials::CustomSpeech {
                    endpoint: "wss://speech.example/v1".into(),
                    model: "synthetic-model".into(),
                    api_key: "synthetic-speech".into(),
                }
            } else {
                translation_request(TextTranslation::DeepL, "", "", "synthetic-text:fx")
            };
            assert!(store.save_credentials(&profile.id, &request).is_err());
            fake.state.lock().unwrap().unavailable.clear();
            fake.state.lock().unwrap().loads.clear();
            store.save_credentials(&profile.id, &request).unwrap();
            let (saved, untouched) = if speech_save {
                (&speech, &text)
            } else {
                (&text, &speech)
            };
            assert!(store
                .load_secret(PROFILE_KEYCHAIN_SERVICE, saved)
                .unwrap()
                .is_some());
            assert!(store
                .load_secret(PROFILE_KEYCHAIN_SERVICE, untouched)
                .is_err());
            assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, untouched), 0);
        }
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
                chat_mock: None,
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

        assert!(preferences.subtitle_font_family.is_empty());
        assert_eq!(preferences.subtitle_background_opacity, 80);
        assert_eq!(preferences.subtitle_color, SubtitleColor::White);
        assert_eq!(preferences.subtitle_alignment, SubtitleAlignment::Center);
        assert_eq!(
            preferences.subtitle_display_mode,
            SubtitleDisplayMode::Translation
        );
        assert!(!preferences.subtitle_blends_with_background);
        assert!(preferences.show_intermediate_subtitles);
        assert!(!preferences.show_subtitle_dividers);
        assert!(!preferences.show_subtitle_timestamps);
        assert_eq!(preferences.pulse_style, PulseStyle::Ribbon);
    }

    #[test]
    fn intermediate_subtitle_preference_round_trips_both_choices() {
        for enabled in [false, true] {
            let prefs = Preferences {
                show_intermediate_subtitles: enabled,
                ..Preferences::default()
            };
            let restored: Preferences =
                serde_json::from_str(&serde_json::to_string(&prefs).unwrap()).unwrap();
            assert_eq!(restored.show_intermediate_subtitles, enabled);
        }
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
                    "show_in_dock": false,
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
            assert!(!prefs.show_in_dock);
            store
                .save_preferences(|prefs| prefs.translation_mode = TranslationMode::HighQuality)
                .unwrap();
            let persisted: Preferences = serde_json::from_slice(
                &std::fs::read(directory.path().join("preferences.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(persisted.translation_mode, TranslationMode::Turbo);
            assert_eq!(persisted.pulse_style, PulseStyle::Ribbon);
            assert!(!persisted.show_in_dock);
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
    fn subtitle_font_family_persists_and_resets_without_secret_access() {
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        let original_catalog = store.profile_catalog().unwrap();
        for (input, expected) in [
            ("  思源黑体  ", "思源黑体"),
            (
                "A Font Removed From This Computer",
                "A Font Removed From This Computer",
            ),
            ("", ""),
        ] {
            store
                .save_preferences_for_active_profile(|prefs| {
                    prefs.subtitle_font_family = input.into();
                })
                .unwrap();
            store
                .save_preferences_for_active_profile(|prefs| prefs.font_size = 19.0)
                .unwrap();
            let reloaded = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
            assert_eq!(reloaded.preferences().subtitle_font_family, expected);
            assert_eq!(reloaded.preferences().font_size, 19.0);
            assert_eq!(reloaded.profile_catalog().unwrap(), original_catalog);
        }
        let state = fake.state.lock().unwrap();
        assert!(state.loads.is_empty());
        assert!(state.values.is_empty());
    }

    #[test]
    fn subtitle_font_invalid_save_rolls_back_all_preferences_and_disk() {
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        store
            .save_preferences(|prefs| prefs.subtitle_font_family = "Noto Sans".into())
            .unwrap();
        let before = store.preferences();
        let bytes = std::fs::read(directory.path().join("preferences.json")).unwrap();
        for invalid in ["Font\nName".to_owned(), "字".repeat(257)] {
            assert_eq!(
                store.save_preferences_for_active_profile(|prefs| {
                    prefs.font_size = 20.0;
                    prefs.subtitle_font_family = invalid;
                }),
                Err(crate::core::subtitle_font::INVALID_FAMILY_NAME.into())
            );
            assert_eq!(store.preferences(), before);
            assert_eq!(
                std::fs::read(directory.path().join("preferences.json")).unwrap(),
                bytes
            );
        }
        assert!(fake.state.lock().unwrap().loads.is_empty());
    }

    #[test]
    fn subtitle_font_load_sanitizes_invalid_names_without_resetting_other_preferences() {
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        for (input, expected) in [
            ("  Noto Sans  ".to_owned(), "Noto Sans"),
            ("Font\u{7f}".to_owned(), ""),
            ("字".repeat(257), ""),
        ] {
            std::fs::write(
                directory.path().join("preferences.json"),
                serde_json::to_vec(&serde_json::json!({
                    "subtitle_font_family": input,
                    "font_size": 19,
                    "show_in_dock": false
                }))
                .unwrap(),
            )
            .unwrap();
            let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
            assert_eq!(store.preferences().subtitle_font_family, expected);
            assert_eq!(store.preferences().font_size, 19.0);
            assert!(!store.preferences().show_in_dock);
        }
        assert!(fake.state.lock().unwrap().loads.is_empty());
    }

    #[test]
    fn subtitle_background_opacity_persists_and_is_bounded_on_save_and_load() {
        let directory = std::env::temp_dir().join(format!(
            "mimi-background-opacity-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let fake = FakeSecretStore::default();
        let store = SettingsStore::at_path(directory.clone(), Box::new(fake.clone()));
        for (input, expected) in [(0, 0), (35, 35), (100, 100), (255, 100)] {
            store
                .save_preferences(|prefs| prefs.subtitle_background_opacity = input)
                .unwrap();
            let reloaded = SettingsStore::at_path(directory.clone(), Box::new(fake.clone()));
            assert_eq!(reloaded.preferences().subtitle_background_opacity, expected);
        }
        std::fs::write(
            directory.join("preferences.json"),
            br#"{"subtitle_background_opacity":255}"#,
        )
        .unwrap();
        let reloaded = SettingsStore::at_path(directory.clone(), Box::new(fake));
        assert_eq!(reloaded.preferences().subtitle_background_opacity, 100);
        std::fs::remove_dir_all(directory).unwrap();
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
        let legacy: Preferences = serde_json::from_str(
            r#"{"keep_subtitle_text_opaque":true,"font_size":22,"subtitle_background_opacity":35}"#,
        )
        .unwrap();
        assert_eq!(legacy.font_size, 22.0);
        assert_eq!(legacy.subtitle_background_opacity, 35);
        assert_eq!(legacy.microphone_subtitle_color, SubtitleColor::Yellow);
        for enabled in [true, false] {
            store
                .save_preferences_for_active_profile(|prefs| {
                    prefs.show_subtitle_dividers = enabled;
                    prefs.microphone_subtitle_color = SubtitleColor::Custom([0x12, 0x34, 0x56]);
                })
                .unwrap();
            assert_eq!(store.configuration().unwrap(), original_configuration);
            let reloaded = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
            assert_eq!(reloaded.preferences().show_subtitle_dividers, enabled);
            assert_eq!(
                reloaded.preferences().microphone_subtitle_color,
                SubtitleColor::Custom([0x12, 0x34, 0x56])
            );
            assert_eq!(reloaded.preferences().subtitle_color, SubtitleColor::White);
        }
    }

    #[test]
    fn subtitle_timestamps_persist_without_changing_provider_configuration() {
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        store
            .save_api_key(DEFAULT_ALIBABA_PROFILE_ID, "synthetic-asr")
            .unwrap();
        let original_configuration = store.configuration().unwrap();
        assert!(!store.preferences().show_subtitle_timestamps);
        for enabled in [true, false] {
            store
                .save_preferences_for_active_profile(|prefs| {
                    prefs.show_subtitle_timestamps = enabled;
                })
                .unwrap();
            let reloaded = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
            assert_eq!(reloaded.preferences().show_subtitle_timestamps, enabled);
            assert_eq!(reloaded.configuration().unwrap(), original_configuration);
            assert_eq!(
                crate::commands::SettingsSnapshotPayload::from_store(&reloaded)
                    .show_subtitle_timestamps,
                enabled
            );
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
        assert_eq!(
            preferences.target_language,
            TargetLanguage::SimplifiedChinese
        );
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
