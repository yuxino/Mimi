//! Credential state queries, snapshot presence checks, and diagnostics.

use super::{cache_key, credential_account};
use crate::core::services::credentials::ProviderCredentials;
use crate::core::services::provider::{ProviderKind, ServiceProfile, TextTranslation};
use crate::settings_store::{
    CredentialState, SecretStoreError, SettingsStore, CREDENTIAL_SERVICE_UNAVAILABLE,
    CREDENTIAL_STORE_ACCESS_DENIED, CREDENTIAL_STORE_UNAVAILABLE,
};

impl SettingsStore {
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
            Err(error) if error == CREDENTIAL_STORE_UNAVAILABLE => "unavailable",
            Err(error) if error == CREDENTIAL_SERVICE_UNAVAILABLE => "serviceUnavailable",
            Err(error) if error == CREDENTIAL_STORE_ACCESS_DENIED => "accessDenied",
            Err(_) => "invalid",
            Ok(None) => "missing",
            Ok(Some(_)) => "present",
        }
    }

    pub fn credential_state(&self, profile: &ServiceProfile) -> CredentialState {
        if let Some((speech, text)) = self.custom_credential_states(profile) {
            return speech.combined(text);
        }
        match self.credentials_for_profile(profile) {
            Ok(Some(_)) => CredentialState::Present,
            Ok(None) => CredentialState::Missing,
            Err(_) if self.azure_translation_deployment_is_missing(profile) => {
                CredentialState::Missing
            }
            Err(_) => CredentialState::Unavailable,
        }
    }

    fn azure_translation_deployment_is_missing(&self, profile: &ServiceProfile) -> bool {
        if profile.provider != ProviderKind::AzureOpenAIRealtime
            || profile.speech_translation_mode
                != crate::core::services::provider::SpeechTranslationMode::Simultaneous
        {
            return false;
        }
        // A valid saved ASR setup needs one more field for simultaneous mode.
        // Other validation and storage errors remain unavailable.
        matches!(
            self.load_api_key_for_profile(profile)
                .ok()
                .flatten()
                .and_then(|value| Self::decode_speech_editor_credentials(profile, &value).ok()),
            Some(ProviderCredentials::AzureOpenAI { deployment, .. }) if deployment.is_empty()
        )
    }

    /// Settings show saved-item presence; selection does not authorize secrets.
    pub fn credential_state_for_snapshot(&self, profile: &ServiceProfile) -> CredentialState {
        if self.is_ui_test
            || (!self.secret.uses_local_file() && !cfg!(target_os = "macos"))
            || (self.secret.uses_local_file()
                && profile.provider == ProviderKind::AzureOpenAIRealtime)
        {
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
        if !profile.is_standalone_asr() {
            return None;
        }
        if self.is_ui_test || (!self.secret.uses_local_file() && !cfg!(target_os = "macos")) {
            return self.custom_credential_states(profile);
        }
        let speech = self.speech_presence_for_snapshot(profile);
        let text = if profile.text_translation() == TextTranslation::XAI
            && self.secret.uses_local_file()
        {
            match self.text_credentials_for_profile(profile) {
                Ok(Some(_)) => CredentialState::Present,
                Ok(None) => CredentialState::Missing,
                Err(_) => CredentialState::Unavailable,
            }
        } else if matches!(
            profile.text_translation(),
            TextTranslation::FollowService | TextTranslation::Apple
        ) {
            CredentialState::Present
        } else {
            Self::presence_state(self.secret_present(&Self::destination_account(profile)))
        };
        Some((speech, text))
    }

    fn speech_presence_for_snapshot(&self, profile: &ServiceProfile) -> CredentialState {
        if profile.provider == ProviderKind::AppleSpeech {
            return CredentialState::Present;
        }
        Self::presence_state(self.secret_present(&credential_account(profile)))
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
        if !profile.is_standalone_asr() {
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
        if self.secret.uses_local_file() {
            "localFile"
        } else {
            "keychain"
        }
    }
}
