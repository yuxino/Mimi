//! Credential editor hydration and explicit single-field secret reveal.

use super::TextTranslationDestination;
use crate::core::services::credentials::{
    CredentialRevealField, ProviderCredentials, TextTranslationCredentials,
};
use crate::core::services::provider::{ProviderKind, ServiceProfile, TextTranslation};
use crate::settings_store::{SecretStoreError, SettingsStore, CREDENTIAL_STORE_UNAVAILABLE};
use serde::Serialize;

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

impl SettingsStore {
    pub(in crate::settings_store) fn decode_speech_editor_credentials(
        profile: &ServiceProfile,
        value: &str,
    ) -> Result<ProviderCredentials, crate::core::services::credentials::ProviderCredentialsError>
    {
        // An Azure ASR setup may omit the translation deployment. Keep its
        // saved fields editable after switching back to simultaneous mode;
        // session/probe validation still requires the complete active route.
        let mut editable = profile.clone();
        if editable.provider == ProviderKind::AzureOpenAIRealtime {
            editable.speech_translation_mode =
                crate::core::services::provider::SpeechTranslationMode::AsrMt;
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
                            crate::core::protocols::translation::openai_compatible::validate_model(
                                &model,
                            )
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
}
