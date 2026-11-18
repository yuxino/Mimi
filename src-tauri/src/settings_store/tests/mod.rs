//! Synthetic shared settings fixtures and behavior regressions by responsibility.

use super::credentials::cache_key;
use super::*;
use crate::core::audio::audio_input::AudioInput;
use crate::core::models::{SourceLanguage, TargetLanguage, TranslationMode};
use crate::core::models::{SubtitleColor, SubtitleDisplayMode};
use crate::core::services::configuration::LiveTranslationConfiguration;
use crate::core::services::credentials::{
    CredentialRevealField, ProviderCredentials, TextTranslationCredentials,
};
use crate::core::services::network_proxy::ProxyConfig;
use crate::core::services::provider::{
    CustomSpeechLanguagesPatch, ProviderKind, TextTranslation, TextTranslationName,
};
use crate::core::services::provider::{ProviderPreferences, DEFAULT_ALIBABA_PROFILE_ID};
use std::cell::Cell;
use std::collections::HashSet;
use std::fs::OpenOptions;
use std::path::Path;
use std::sync::Arc;

mod apple;
mod configuration;
mod credential_editor;
mod credential_storage;
mod destinations;
mod migration;
mod native_credentials;
mod preferences;
mod profiles;
mod realtime;

fn save_realtime_speech_mode(
    store: &SettingsStore,
    id: &str,
    mode: crate::core::services::provider::SpeechTranslationMode,
) -> Result<ServiceProfile, String> {
    store.update_profile_options_with_speech_mode(
        id,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(mode),
    )
}

fn realtime_speech_credentials(provider: ProviderKind) -> ProviderCredentials {
    if provider == ProviderKind::AzureOpenAIRealtime {
        ProviderCredentials::AzureOpenAI {
            endpoint: "https://synthetic.openai.azure.com".into(),
            deployment: "translation-deployment".into(),
            transcription_deployment: "transcription-deployment".into(),
            api_key: "synthetic-recognition-key".into(),
        }
    } else {
        ProviderCredentials::api_key("synthetic-recognition-key")
    }
}

fn save_xai_options(
    store: &SettingsStore,
    id: &str,
    mode: Option<crate::core::services::provider::XAISpeechMode>,
    model: Option<&str>,
) -> Result<ServiceProfile, String> {
    store.update_profile_options_with_xai(
        id, None, None, None, None, None, None, None, None, None, None, mode, model,
    )
}

fn xai_text_request(key: &str, model: &str) -> ProviderCredentials {
    ProviderCredentials::AlibabaTranslation {
        api_key: String::new(),
        text_translation: TextTranslation::XAI,
        endpoint: String::new(),
        token: key.into(),
        model: model.into(),
        clear_token: false,
    }
}

fn mark_legacy_xai_profile(store: &SettingsStore, id: &str) {
    {
        let mut catalog = store.catalog.lock().unwrap();
        let profile = catalog
            .profiles
            .iter_mut()
            .find(|profile| profile.id == id)
            .unwrap();
        profile.xai_independent_translation_migrated = false;
        profile.text_translation = None;
        profile.xai_text_model = "legacy-custom-grok".into();
    }
    store.persist_catalog().unwrap();
}

fn fresh_store(directory: &Path, fake: &FakeSecretStore) -> SettingsStore {
    SettingsStore::load_with_secret(
        directory.into(),
        false,
        Box::new(fake.clone()),
        PROFILE_KEYCHAIN_SERVICE,
    )
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
            Some(
                crate::core::services::provider::ProfileLanguagePresetPatch {
                    preset: Some(crate::core::services::provider::ProfileLanguagePreset {
                        source_language: source,
                        target_language: target,
                    }),
                },
            ),
        )
        .unwrap();
}

fn persistence_attributes(value: &str) -> HashMap<String, String> {
    HashMap::from([("persistence".to_string(), value.to_string())])
}

#[derive(Default)]
struct FakeState {
    values: HashMap<SecretCacheKey, String>,
    failures: HashMap<SecretCacheKey, SecretStoreError>,
    unavailable: HashSet<SecretCacheKey>,
    unavailable_deletes: HashSet<SecretCacheKey>,
    loads: Vec<SecretCacheKey>,
    saves_before_failure: Option<usize>,
}

#[derive(Clone, Default)]
struct FakeSecretStore {
    state: Arc<Mutex<FakeState>>,
    local_file: bool,
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
    fn uses_local_file(&self) -> bool {
        self.local_file
    }

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
        if let Some(remaining) = &mut state.saves_before_failure {
            if *remaining == 0 {
                return Err(SecretStoreError::Unavailable);
            }
            *remaining -= 1;
        }
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

fn openai_profile(store: &SettingsStore, name: &str) -> ServiceProfile {
    store
        .create_profile(ProviderKind::OpenAIRealtime, name)
        .unwrap()
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
