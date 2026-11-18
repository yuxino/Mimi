//! Credential editing, per-profile destinations and bounded lookup cache.

use super::preferences::normalize_preferences_value;
use super::{
    CredentialState, SecretCacheKey, SecretStoreError, SettingsStore, CREDENTIAL_STORE_UNAVAILABLE,
    PROFILE_NOT_FOUND,
};
use super::{
    CREDENTIAL_SERVICE_UNAVAILABLE, CREDENTIAL_STORE_ACCESS_DENIED, PROFILE_CATALOG_UNAVAILABLE,
};
use crate::core::credentials::{
    CredentialRevealField, ProviderCredentials, TextTranslationCredentials,
};
use crate::core::provider::{
    ProviderKind, ServiceProfile, TextTranslation, DEFAULT_ALIBABA_PROFILE_ID,
};
use serde::{Deserialize, Serialize};

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
    pub(super) fn from_text_credentials(credentials: TextTranslationCredentials) -> Self {
        let mut state = Self::default();
        let token = match credentials {
            TextTranslationCredentials::Apple => return state,
            TextTranslationCredentials::XAI { api_key, model } => {
                state.model = Some(model);
                api_key
            }
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
pub(super) struct TextTranslationDestination {
    pub(super) endpoint: String,
    pub(super) token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) deep_l_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) open_ai_compatible: Option<OpenAICompatibleDestination>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) chat_mock: Option<OpenAICompatibleDestination>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) xai: Option<XAITextDestination>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct XAITextDestination {
    pub(super) api_key: String,
    pub(super) model: String,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OpenAICompatibleDestination {
    pub(super) endpoint: String,
    pub(super) api_key: String,
    pub(super) model: String,
}

impl TextTranslationDestination {
    pub(super) fn credentials(&self, route: TextTranslation) -> Option<TextTranslationCredentials> {
        match route {
            TextTranslation::XAI => {
                self.xai
                    .as_ref()
                    .map(|value| TextTranslationCredentials::XAI {
                        api_key: value.api_key.clone(),
                        model: value.model.clone(),
                    })
            }
            TextTranslation::FollowService | TextTranslation::Apple => None,
            TextTranslation::DeepL => {
                self.deep_l_api_key
                    .as_ref()
                    .map(|api_key| TextTranslationCredentials::DeepL {
                        api_key: api_key.clone(),
                    })
            }
            TextTranslation::DeepLX => {
                if self.endpoint.is_empty() && self.token.is_empty() {
                    return None;
                }
                Some(TextTranslationCredentials::DeepLX {
                    endpoint: self.endpoint.clone(),
                    token: self.token.clone(),
                })
            }
            TextTranslation::OpenAICompatible => {
                let value = self.open_ai_compatible.clone()?;
                Some(TextTranslationCredentials::OpenAICompatible {
                    endpoint: value.endpoint,
                    model: value.model,
                    api_key: value.api_key,
                })
            }
            TextTranslation::ChatMock => {
                let value = self.chat_mock.clone()?;
                Some(TextTranslationCredentials::ChatMock {
                    endpoint: value.endpoint,
                    model: value.model,
                    api_key: value.api_key,
                })
            }
        }
    }

    pub(super) fn set_credentials(&mut self, credentials: TextTranslationCredentials) {
        match credentials {
            TextTranslationCredentials::XAI { api_key, model } => {
                self.xai = Some(XAITextDestination { api_key, model });
            }
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

pub(super) fn credential_account(profile: &ServiceProfile) -> String {
    format!(
        "provider-profile:{}:{}:api-key",
        profile.id,
        profile.provider.wire_value()
    )
}

pub(super) fn is_default_alibaba(profile: &ServiceProfile) -> bool {
    profile.id == DEFAULT_ALIBABA_PROFILE_ID && profile.provider == ProviderKind::AlibabaCloud
}
pub(super) fn cache_key(service: &str, account: &str) -> SecretCacheKey {
    (service.to_string(), account.to_string())
}

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

    pub(super) fn retry_profile_credential_errors(
        &self,
        profile: &ServiceProfile,
        speech: bool,
        text: bool,
    ) {
        let speech = speech && profile.provider != ProviderKind::AppleSpeech;
        let account = credential_account(profile);
        self.secret_cache
            .lock()
            .unwrap()
            .retain(|(service, slot), result| {
                let selected = service == self.profile_keychain_service
                    && ((speech && slot == &account)
                        || (text
                            && profile.supports_text_translation_configuration()
                            && slot == &Self::destination_account(profile)));
                result.is_ok() || !selected
            });
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

    pub(super) fn azure_translation_deployment_is_missing(&self, profile: &ServiceProfile) -> bool {
        if profile.provider != ProviderKind::AzureOpenAIRealtime
            || profile.speech_translation_mode
                != crate::core::provider::SpeechTranslationMode::Simultaneous
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

    pub(super) fn speech_presence_for_snapshot(&self, profile: &ServiceProfile) -> CredentialState {
        if profile.provider == ProviderKind::AppleSpeech {
            return CredentialState::Present;
        }
        Self::presence_state(self.secret_present(&credential_account(profile)))
    }

    pub(super) fn secret_present(&self, account: &str) -> Result<bool, SecretStoreError> {
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

    pub(super) fn presence_state(presence: Result<bool, SecretStoreError>) -> CredentialState {
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

    pub fn profile_credential_storage(&self, _profile_id: &str) -> &'static str {
        self.credential_storage()
    }

    #[cfg(test)]
    pub(super) fn migrate_credentials(&self) -> Result<(), String> {
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

    pub(super) fn decode_speech_editor_credentials(
        profile: &ServiceProfile,
        value: &str,
    ) -> Result<ProviderCredentials, crate::core::credentials::ProviderCredentialsError> {
        // An Azure ASR setup may omit the translation deployment. Keep its
        // saved fields editable after switching back to simultaneous mode;
        // session/probe validation still requires the complete active route.
        let mut editable = profile.clone();
        if editable.provider == ProviderKind::AzureOpenAIRealtime {
            editable.speech_translation_mode = crate::core::provider::SpeechTranslationMode::AsrMt;
        }
        ProviderCredentials::decode_for_profile(&editable, value)
    }

    /// Hydrates only the selected editor's configuration and saved-field flags.
    /// API keys remain native until a separate explicit reveal request.
    pub fn credential_editor_state(
        &self,
        profile_id: &str,
        text_translation: Option<TextTranslation>,
    ) -> Result<CredentialEditorState, String> {
        let profile = self.profile(profile_id)?;
        if text_translation == Some(TextTranslation::Apple) {
            return Ok(CredentialEditorState::default());
        }
        if let Some(route) = text_translation {
            // Explicitly selecting a destination may hydrate its model/address
            // and saved-field flags before committing the route. Revealing a
            // secret remains restricted to the currently saved route below.
            if !profile.supports_text_translation_configuration()
                || route == TextTranslation::FollowService
            {
                return Err("credential_reveal_field_mismatch".into());
            }
            if let Some(value) = self.destination_value(&profile)? {
                let destination: TextTranslationDestination = serde_json::from_str(&value)
                    .map_err(|_| CREDENTIAL_STORE_UNAVAILABLE.to_string())?;
                let Some(credentials) = destination.credentials(route) else {
                    return Ok(CredentialEditorState::default());
                };
                let credentials = match credentials {
                    TextTranslationCredentials::XAI { api_key, model } if api_key.is_empty() => {
                        let model =
                            crate::core::protocols::openai_compatible::validate_model(&model)
                                .map_err(|_| CREDENTIAL_STORE_UNAVAILABLE.to_string())?;
                        TextTranslationCredentials::XAI { api_key, model }
                    }
                    credentials => credentials
                        .validated()
                        .map_err(|_| CREDENTIAL_STORE_UNAVAILABLE.to_string())?,
                };
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
        let credentials = Self::decode_speech_editor_credentials(&profile, &value)
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
        let profile = self.profile(profile_id)?;
        if !field.allowed_for(&profile, text_translation) {
            return Err("credential_reveal_field_mismatch".into());
        }
        // Read only the selected field's existing local-file slot. In particular,
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
                    TextTranslation::XAI => {
                        return Ok(destination
                            .xai
                            .map(|value| value.api_key)
                            .filter(|key| !key.is_empty()))
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
        let credentials = Self::decode_speech_editor_credentials(&profile, &value)
            .map_err(|_| "credential_store_unavailable".to_string())?;
        credentials
            .revealed_field(&profile, field, text_translation)
            .map(|value| value.map(str::to_owned))
            .map_err(|_| "credential_reveal_field_mismatch".into())
    }

    #[cfg(test)]
    pub(super) fn load_api_key(&self) -> Result<Option<String>, String> {
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
        if profile.provider == ProviderKind::AzureOpenAIRealtime
            && matches!(credentials, ProviderCredentials::AzureOpenAI { api_key, .. } if api_key.trim().is_empty())
        {
            self.retry_profile_credential_errors(&profile, true, false);
            let saved = self
                .load_api_key_for_profile(&profile)
                .map_err(SecretStoreError::public_error)?
                .map(|value| Self::decode_speech_editor_credentials(&profile, &value))
                .transpose()
                .map_err(|error| error.to_string())?;
            // The same native merge used by checks keeps the key bound to its
            // resource, including when ASR metadata needs a new deployment.
            let value = credentials
                .resolve_draft_for_profile(&profile, saved.as_ref())
                .and_then(|resolved| resolved.encode_for_profile(&profile))
                .map_err(|error| error.to_string())?;
            return self.save_api_key_for_profile(&profile, &value, true);
        }
        let value = credentials
            .encode_for_profile(&profile)
            .map_err(|error| error.to_string())?;
        self.save_api_key_for_profile(&profile, &value, true)
    }

    pub(super) fn destination_account(profile: &ServiceProfile) -> String {
        format!(
            "provider-profile:{}:{}:text-translation",
            profile.id,
            profile.provider.wire_value()
        )
    }

    pub(super) fn destination_value(
        &self,
        profile: &ServiceProfile,
    ) -> Result<Option<String>, String> {
        if !profile.supports_text_translation_configuration() {
            return Ok(None);
        }
        self.load_secret(
            self.profile_keychain_service,
            &Self::destination_account(profile),
        )
        .map_err(SecretStoreError::public_error)
    }

    pub(super) fn write_destination_value(
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

    pub(super) fn credentials_for_profile(
        &self,
        profile: &ServiceProfile,
    ) -> Result<Option<ProviderCredentials>, String> {
        if profile.provider == ProviderKind::AppleSpeech {
            return Ok(Some(ProviderCredentials::AppleSpeech));
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
        ) && profile.text_translation() != TextTranslation::XAI
            && !matches!(
                profile.text_translation(),
                TextTranslation::FollowService | TextTranslation::Apple
            )
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
                TextTranslation::ChatMock => {
                    let destination = destination.chat_mock.unwrap_or_default();
                    ProviderCredentials::ChatMock {
                        asr_api_key,
                        endpoint: destination.endpoint,
                        api_key: destination.api_key,
                        model: destination.model,
                    }
                }
                TextTranslation::XAI => return Ok(Some(credentials)),
                TextTranslation::FollowService | TextTranslation::Apple => unreachable!(),
            };
            return credentials
                .validated_for(profile.effective_provider())
                .map(Some)
                .map_err(|error| error.to_string());
        }
        Ok(Some(credentials))
    }

    pub(super) fn text_credentials_for_profile(
        &self,
        profile: &ServiceProfile,
    ) -> Result<Option<TextTranslationCredentials>, String> {
        if profile.text_translation() == TextTranslation::Apple {
            return Ok(Some(TextTranslationCredentials::Apple));
        }
        if profile.text_translation() == TextTranslation::FollowService {
            return Ok(None);
        }
        let Some(value) = self.destination_value(profile)? else {
            return Ok(None);
        };
        let destination: TextTranslationDestination =
            serde_json::from_str(&value).map_err(|_| {
                crate::core::credentials::ProviderCredentialsError::InvalidStoredValue.to_string()
            })?;
        if profile.text_translation() == TextTranslation::XAI
            && destination
                .xai
                .as_ref()
                .is_some_and(|value| value.api_key.trim().is_empty())
        {
            return Ok(None);
        }
        destination
            .credentials(profile.text_translation())
            .map(|credentials| credentials.validated())
            .transpose()
            .map_err(|error| error.to_string())
    }

    pub(super) fn save_custom_speech(
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
        let previous = match self.load_api_key_for_profile(profile) {
            Ok(previous) => previous,
            Err(SecretStoreError::MigrationRequired) => {
                // A complete explicit value can retire its unread pending import.
                // The unknown old value must never become a missing-key rollback.
                let (Some(endpoint), Some(model)) = (&entered_endpoint, &entered_model) else {
                    return Err(SecretStoreError::MigrationRequired.public_error());
                };
                if api_key.trim().is_empty() {
                    return Err(SecretStoreError::MigrationRequired.public_error());
                }
                let value = ProviderCredentials::CustomSpeech {
                    endpoint: endpoint.clone(),
                    model: model.clone(),
                    api_key: api_key.into(),
                }
                .encode_for_keychain(profile.provider)
                .map_err(|error| error.to_string())?;
                return self.save_api_key_for_profile(profile, &value, true);
            }
            Err(error) => return Err(error.public_error()),
        };
        let draft = ProviderCredentials::CustomSpeech {
            endpoint: entered_endpoint.unwrap_or_default(),
            model: entered_model.unwrap_or_default(),
            api_key: api_key.into(),
        };
        // A complete explicit replacement can repair a malformed saved slot.
        // Partial edits still require a valid old value, and the raw previous
        // value remains available for rollback if persistence fails.
        let value = match draft.encode_for_keychain(profile.provider) {
            Ok(value) => value,
            Err(_) => {
                let previous_credentials = previous
                    .as_deref()
                    .map(|value| ProviderCredentials::decode_for_profile(profile, value))
                    .transpose()
                    .map_err(|error| error.to_string())?;
                draft
                    .resolve_draft(profile.provider, previous_credentials.as_ref())
                    .and_then(|credentials| credentials.encode_for_keychain(profile.provider))
                    .map_err(|error| error.to_string())?
            }
        };
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

    pub(super) fn save_custom_text_translation(
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
    pub(super) fn save_apple_text_translation(
        &self,
        profile: &ServiceProfile,
        api_key: &str,
    ) -> Result<(), String> {
        use crate::core::credentials::ProviderCredentialsError as Error;
        if !(profile.provider.supports_text_translation() || profile.uses_independent_asr())
            || (profile.is_standalone_asr() && !api_key.trim().is_empty())
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

    pub(super) fn save_text_translation(
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
        if profile.is_standalone_asr() {
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
                            xai: None,
                        }),
                        ProviderCredentials::DeepL { api_key, .. } => {
                            Some(TextTranslationDestination {
                                endpoint: String::new(),
                                token: String::new(),
                                deep_l_api_key: Some(api_key.clone()),
                                open_ai_compatible: None,
                                chat_mock: None,
                                xai: None,
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

    pub(super) fn delete_profile_credentials(
        &self,
        profile: &ServiceProfile,
    ) -> Result<(), String> {
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

    pub(super) fn load_api_key_for_profile(
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
            return Ok(Some(value));
        }
        Ok(None)
    }

    pub(super) fn delete_api_key_for_profile(
        &self,
        profile: &ServiceProfile,
    ) -> Result<(), String> {
        if profile.provider == ProviderKind::AppleSpeech {
            return Ok(());
        }
        let account = credential_account(profile);
        self.delete_secret(self.profile_keychain_service, &account)
            .map_err(SecretStoreError::public_error)
    }

    pub(super) fn save_api_key_for_profile(
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

        Ok(())
    }

    pub(super) fn load_secret(
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

    pub(super) fn load_secret_uncached(
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

    pub(super) fn save_secret(
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

    pub(super) fn delete_secret(
        &self,
        service: &str,
        account: &str,
    ) -> Result<(), SecretStoreError> {
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

    pub(super) fn cache_secret(&self, service: &str, account: &str, value: Option<String>) {
        self.secret_cache
            .lock()
            .unwrap()
            .insert(cache_key(service, account), Ok(value));
    }
}
