//! Provider credentials and credential-free local recognition configuration.

use crate::core::services::provider::{
    ProviderKind, ServiceProfile, SpeechTranslationMode, TextTranslation,
};
use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

const MAXIMUM_CREDENTIAL_FIELD_LENGTH: usize = 1_024;
const MAXIMUM_DEPLOYMENT_LENGTH: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProviderCredentialsError {
    #[error("Add the connection credentials for {0} in Settings.")]
    Missing(ProviderKind),
    #[error("The saved credentials do not match the selected service.")]
    ProviderMismatch,
    #[error("The Azure OpenAI endpoint must be an official HTTPS resource endpoint.")]
    InvalidAzureEndpoint,
    #[error("Use an HTTPS DeepLX endpoint (or HTTP on localhost), without URL credentials, query or fragment.")]
    InvalidDeepLXEndpoint,
    #[error("Use an HTTPS OpenAI-compatible endpoint (or HTTP on localhost), without URL credentials, query or fragment.")]
    InvalidOpenAICompatibleEndpoint,
    #[error("The OpenAI-compatible model name is invalid.")]
    InvalidOpenAICompatibleModel,
    #[error("custom_speech_endpoint_invalid")]
    InvalidCustomSpeechEndpoint,
    #[error("custom_speech_model_invalid")]
    InvalidCustomSpeechModel,
    #[error("text_translation_credentials_missing")]
    MissingTextTranslation,
    #[error("The Azure OpenAI deployment name is invalid.")]
    InvalidAzureDeployment,
    #[error("One or more credential fields are invalid.")]
    InvalidField,
    #[error("The saved credentials could not be read.")]
    InvalidStoredValue,
    #[error("credential_reveal_field_mismatch")]
    InvalidRevealField,
}

/// Explicit single-field reveal requests, never part of normal snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CredentialRevealField {
    ApiKey,
    AsrApiKey,
    Token,
    SecretId,
    SecretKey,
    AppKey,
}

impl CredentialRevealField {
    pub fn allowed_for(
        self,
        profile: &ServiceProfile,
        expected_text_translation: Option<TextTranslation>,
    ) -> bool {
        match self {
            Self::ApiKey => {
                profile.provider.uses_api_key_only()
                    || profile.provider == ProviderKind::AzureOpenAIRealtime
                    || profile.provider.is_custom_speech()
            }
            Self::AsrApiKey => {
                matches!(
                    profile.provider,
                    ProviderKind::AlibabaCloud | ProviderKind::DeepLX
                ) || profile.provider.is_custom_speech()
            }
            Self::Token => {
                profile.supports_text_translation_configuration()
                    && !matches!(
                        profile.text_translation(),
                        TextTranslation::FollowService | TextTranslation::Apple
                    )
                    && expected_text_translation == Some(profile.text_translation())
            }
            Self::SecretId | Self::SecretKey => profile.provider == ProviderKind::TencentCloud,
            Self::AppKey => profile.provider == ProviderKind::BaiduTranslate,
        }
    }
}

/// Write-only IPC payload and keychain representation. The tagged JSON form is
/// used only for providers with multiple fields; historical one-key values
/// remain raw strings in Keychain for rollback compatibility.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ProviderCredentials {
    /// Local recognition has no persisted credential slot.
    AppleSpeech,
    /// Write-only request. Empty api_key reuses the profile's existing key
    /// natively; this request variant is never stored or returned over IPC.
    AlibabaTranslation {
        api_key: String,
        text_translation: TextTranslation,
        endpoint: String,
        token: String,
        #[serde(default)]
        model: String,
        #[serde(default)]
        clear_token: bool,
    },
    DeepLX {
        asr_api_key: String,
        endpoint: String,
        token: String,
    },
    DeepL {
        asr_api_key: String,
        api_key: String,
    },
    OpenAICompatible {
        asr_api_key: String,
        endpoint: String,
        api_key: String,
        model: String,
    },
    ChatMock {
        asr_api_key: String,
        endpoint: String,
        api_key: String,
        model: String,
    },
    ApiKey {
        api_key: String,
    },
    CustomSpeech {
        endpoint: String,
        model: String,
        api_key: String,
    },
    AzureOpenAI {
        endpoint: String,
        deployment: String,
        transcription_deployment: String,
        api_key: String,
    },
    TencentCloud {
        app_id: String,
        secret_id: String,
        secret_key: String,
    },
    BaiduTranslate {
        app_id: String,
        app_key: String,
    },
}

/// Independent text-destination credentials. They never contain a speech key.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum TextTranslationCredentials {
    /// Local translation stores no private destination or credential.
    Apple,
    #[serde(rename = "xAI")]
    XAI {
        api_key: String,
        model: String,
    },
    DeepL {
        api_key: String,
    },
    DeepLX {
        endpoint: String,
        token: String,
    },
    OpenAICompatible {
        endpoint: String,
        model: String,
        api_key: String,
    },
    ChatMock {
        endpoint: String,
        model: String,
        api_key: String,
    },
}

impl fmt::Debug for TextTranslationCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TextTranslationCredentials")
            .field("route", &self.translation())
            .field("values", &"[REDACTED]")
            .finish()
    }
}

impl TextTranslationCredentials {
    /// Resolve a write-only draft without requiring or exposing a speech key.
    /// Saves and draft checks share endpoint-bound optional authentication.
    pub fn resolve_update(
        route: TextTranslation,
        saved: Option<&Self>,
        endpoint: &str,
        token_update: Option<&str>,
        model: &str,
    ) -> Result<Self, ProviderCredentialsError> {
        if route == TextTranslation::XAI && !endpoint.trim().is_empty() {
            return Err(ProviderCredentialsError::InvalidField);
        }
        let saved = saved.filter(|value| value.translation() == route);
        let (old_endpoint, old_model, old_key) = match saved {
            Some(Self::DeepL { api_key }) => ("", "", api_key.as_str()),
            Some(Self::XAI { api_key, model }) => ("", model.as_str(), api_key.as_str()),
            Some(Self::DeepLX { endpoint, token }) => (endpoint.as_str(), "", token.as_str()),
            Some(
                Self::OpenAICompatible {
                    endpoint,
                    model,
                    api_key,
                }
                | Self::ChatMock {
                    endpoint,
                    model,
                    api_key,
                },
            ) => (endpoint.as_str(), model.as_str(), api_key.as_str()),
            Some(Self::Apple) | None => ("", "", ""),
        };
        let endpoint = if endpoint.trim().is_empty() {
            old_endpoint.to_string()
        } else if route.uses_chat_completions() {
            crate::core::protocols::translation::openai_compatible::endpoint(endpoint)
                .map_err(|_| ProviderCredentialsError::InvalidOpenAICompatibleEndpoint)?
                .to_string()
        } else if route == TextTranslation::DeepLX {
            crate::core::protocols::translation::deeplx::endpoint(endpoint)
                .map_err(|_| ProviderCredentialsError::InvalidDeepLXEndpoint)?
                .to_string()
        } else {
            String::new()
        };
        let key = token_update.unwrap_or(if endpoint == old_endpoint {
            old_key
        } else {
            ""
        });
        let model = if model.trim().is_empty() {
            old_model
        } else {
            model
        };
        match route {
            TextTranslation::XAI => Self::XAI {
                api_key: key.into(),
                model: model.into(),
            },
            TextTranslation::Apple => Self::Apple,
            TextTranslation::DeepL => Self::DeepL {
                api_key: key.into(),
            },
            TextTranslation::DeepLX => Self::DeepLX {
                endpoint,
                token: key.into(),
            },
            TextTranslation::OpenAICompatible => Self::OpenAICompatible {
                endpoint,
                model: model.into(),
                api_key: key.into(),
            },
            TextTranslation::ChatMock => Self::ChatMock {
                endpoint,
                model: model.into(),
                api_key: key.into(),
            },
            TextTranslation::FollowService => {
                return Err(ProviderCredentialsError::ProviderMismatch)
            }
        }
        .validated()
    }

    pub const fn translation(&self) -> TextTranslation {
        match self {
            Self::XAI { .. } => TextTranslation::XAI,
            Self::Apple => TextTranslation::Apple,
            Self::DeepL { .. } => TextTranslation::DeepL,
            Self::DeepLX { .. } => TextTranslation::DeepLX,
            Self::OpenAICompatible { .. } => TextTranslation::OpenAICompatible,
            Self::ChatMock { .. } => TextTranslation::ChatMock,
        }
    }

    pub fn validated(&self) -> Result<Self, ProviderCredentialsError> {
        let required = |value: &str| {
            let value = value.trim();
            if value.is_empty() {
                return Err(ProviderCredentialsError::MissingTextTranslation);
            }
            if value.chars().count() > MAXIMUM_CREDENTIAL_FIELD_LENGTH
                || value.chars().any(char::is_control)
            {
                return Err(ProviderCredentialsError::InvalidField);
            }
            Ok(value.to_owned())
        };
        match self {
            Self::XAI { api_key, model } => Ok(Self::XAI {
                api_key: required(api_key)?,
                model: crate::core::protocols::translation::openai_compatible::validate_model(
                    model,
                )
                .map_err(|_| ProviderCredentialsError::InvalidOpenAICompatibleModel)?,
            }),
            Self::Apple => Ok(Self::Apple),
            Self::DeepL { api_key } => Ok(Self::DeepL {
                api_key: required(api_key)?,
            }),
            Self::DeepLX { endpoint, token } => Ok(Self::DeepLX {
                endpoint: crate::core::protocols::translation::deeplx::endpoint(endpoint)
                    .map_err(|_| ProviderCredentialsError::InvalidDeepLXEndpoint)?
                    .to_string(),
                token: if token.trim().is_empty() {
                    String::new()
                } else {
                    required(token)?
                },
            }),
            Self::OpenAICompatible {
                endpoint,
                model,
                api_key,
            } => Ok(Self::OpenAICompatible {
                endpoint: crate::core::protocols::translation::openai_compatible::endpoint(
                    endpoint,
                )
                .map_err(|_| ProviderCredentialsError::InvalidOpenAICompatibleEndpoint)?
                .to_string(),
                model: crate::core::protocols::translation::openai_compatible::validate_model(
                    model,
                )
                .map_err(|_| ProviderCredentialsError::InvalidOpenAICompatibleModel)?,
                api_key: optional_openai_compatible_key(api_key)?,
            }),
            Self::ChatMock {
                endpoint,
                model,
                api_key,
            } => Ok(Self::ChatMock {
                endpoint: crate::core::protocols::translation::openai_compatible::endpoint(
                    endpoint,
                )
                .map_err(|_| ProviderCredentialsError::InvalidOpenAICompatibleEndpoint)?
                .to_string(),
                model: crate::core::protocols::translation::openai_compatible::validate_model(
                    model,
                )
                .map_err(|_| ProviderCredentialsError::InvalidOpenAICompatibleModel)?,
                api_key: optional_openai_compatible_key(api_key)?,
            }),
        }
    }
}

impl fmt::Debug for ProviderCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderCredentials")
            .field("kind", &self.kind_label())
            .field("values", &"[REDACTED]")
            .finish()
    }
}

impl ProviderCredentials {
    /// Saved values are used only for blank draft fields; changed service
    /// addresses never inherit authentication for a different destination.
    pub fn resolve_draft(
        &self,
        provider: ProviderKind,
        saved: Option<&Self>,
    ) -> Result<Self, ProviderCredentialsError> {
        self.resolve_draft_with_mode(provider, SpeechTranslationMode::Simultaneous, saved)
    }

    pub fn resolve_draft_for_profile(
        &self,
        profile: &ServiceProfile,
        saved: Option<&Self>,
    ) -> Result<Self, ProviderCredentialsError> {
        self.resolve_draft_with_mode(profile.provider, profile.speech_translation_mode, saved)
    }

    fn resolve_draft_with_mode(
        &self,
        provider: ProviderKind,
        mode: SpeechTranslationMode,
        saved: Option<&Self>,
    ) -> Result<Self, ProviderCredentialsError> {
        let retain = |draft: &str, previous: &str| {
            if draft.trim().is_empty() {
                previous.to_string()
            } else {
                draft.to_string()
            }
        };
        let merged = match (self, saved) {
            (Self::ApiKey { api_key }, previous) => Self::api_key(retain(
                api_key,
                previous
                    .and_then(Self::alibaba_key)
                    .or_else(|| previous.and_then(Self::direct_api_key))
                    .unwrap_or_default(),
            )),
            (
                Self::CustomSpeech {
                    endpoint,
                    model,
                    api_key,
                },
                previous,
            ) => {
                let (old_endpoint, old_model, old_key) = match previous {
                    Some(Self::CustomSpeech {
                        endpoint,
                        model,
                        api_key,
                    }) => (endpoint.as_str(), model.as_str(), api_key.as_str()),
                    None => ("", "", ""),
                    _ => return Err(ProviderCredentialsError::ProviderMismatch),
                };
                let endpoint = if endpoint.trim().is_empty() {
                    old_endpoint.to_string()
                } else {
                    crate::core::protocols::recognition::custom_speech::endpoint(endpoint, provider)
                        .map_err(|_| ProviderCredentialsError::InvalidCustomSpeechEndpoint)?
                        .to_string()
                };
                let same = endpoint == old_endpoint;
                Self::CustomSpeech {
                    endpoint,
                    model: retain(model, if same { old_model } else { "" }),
                    api_key: retain(api_key, if same { old_key } else { "" }),
                }
            }
            (
                Self::AzureOpenAI {
                    endpoint,
                    deployment,
                    transcription_deployment,
                    api_key,
                },
                Some(Self::AzureOpenAI {
                    endpoint: old_endpoint,
                    deployment: old_deployment,
                    transcription_deployment: old_transcription,
                    api_key: old_key,
                }),
            ) => {
                let endpoint = validated_azure_endpoint(&retain(endpoint, old_endpoint))?;
                // Older native adapters saved valid addresses before canonicalization.
                let same_resource = validated_azure_endpoint(old_endpoint)
                    .is_ok_and(|previous| endpoint == previous);
                let api_key = retain(api_key, if same_resource { old_key } else { "" });
                Self::AzureOpenAI {
                    endpoint,
                    api_key,
                    deployment: retain(deployment, old_deployment),
                    transcription_deployment: retain(transcription_deployment, old_transcription),
                }
            }
            (
                Self::TencentCloud {
                    app_id,
                    secret_id,
                    secret_key,
                },
                Some(Self::TencentCloud {
                    app_id: old_app_id,
                    secret_id: old_id,
                    secret_key: old_key,
                }),
            ) => {
                let app_id = retain(app_id, old_app_id);
                let same_app = app_id.trim() == old_app_id;
                let secret_id = retain(secret_id, if same_app { old_id } else { "" });
                let secret_key = retain(
                    secret_key,
                    if same_app && secret_id.trim() == old_id {
                        old_key
                    } else {
                        ""
                    },
                );
                Self::TencentCloud {
                    app_id,
                    secret_id,
                    secret_key,
                }
            }
            (
                Self::BaiduTranslate { app_id, app_key },
                Some(Self::BaiduTranslate {
                    app_id: old_id,
                    app_key: old_key,
                }),
            ) => {
                let app_id = retain(app_id, old_id);
                let app_key = retain(app_key, if app_id.trim() == old_id { old_key } else { "" });
                Self::BaiduTranslate { app_id, app_key }
            }
            _ => self.clone(),
        };
        merged.validated_for_speech_mode(provider, mode)
    }

    /// Returns only the requested known-provider field. An independent text
    /// destination must match the saved route, so an unsaved route switch
    /// cannot reveal another service's token or official API key.
    pub fn revealed_field(
        &self,
        profile: &ServiceProfile,
        field: CredentialRevealField,
        expected_text_translation: Option<TextTranslation>,
    ) -> Result<Option<&str>, ProviderCredentialsError> {
        use CredentialRevealField as Field;
        if !field.allowed_for(profile, expected_text_translation) {
            return Err(ProviderCredentialsError::InvalidRevealField);
        }
        let value = match (field, self) {
            (Field::ApiKey | Field::AsrApiKey, Self::CustomSpeech { api_key, .. })
                if profile.provider.is_custom_speech() =>
            {
                api_key
            }
            (Field::ApiKey, Self::ApiKey { api_key }) if profile.provider.uses_api_key_only() => {
                api_key
            }
            (Field::ApiKey, Self::AzureOpenAI { api_key, .. })
                if profile.provider == ProviderKind::AzureOpenAIRealtime =>
            {
                api_key
            }
            (
                Field::ApiKey,
                Self::DeepL { asr_api_key, .. }
                | Self::DeepLX { asr_api_key, .. }
                | Self::OpenAICompatible { asr_api_key, .. }
                | Self::ChatMock { asr_api_key, .. },
            ) if profile.provider == ProviderKind::AlibabaCloud => asr_api_key,
            (
                Field::AsrApiKey,
                Self::DeepL { asr_api_key, .. }
                | Self::DeepLX { asr_api_key, .. }
                | Self::OpenAICompatible { asr_api_key, .. }
                | Self::ChatMock { asr_api_key, .. },
            ) if matches!(
                profile.provider,
                ProviderKind::AlibabaCloud | ProviderKind::DeepLX
            ) =>
            {
                asr_api_key
            }
            (Field::AsrApiKey, Self::ApiKey { api_key })
                if matches!(
                    profile.provider,
                    ProviderKind::AlibabaCloud | ProviderKind::DeepLX
                ) =>
            {
                api_key
            }
            (Field::SecretId, Self::TencentCloud { secret_id, .. })
                if profile.provider == ProviderKind::TencentCloud =>
            {
                secret_id
            }
            (Field::SecretKey, Self::TencentCloud { secret_key, .. })
                if profile.provider == ProviderKind::TencentCloud =>
            {
                secret_key
            }
            (Field::AppKey, Self::BaiduTranslate { app_key, .. })
                if profile.provider == ProviderKind::BaiduTranslate =>
            {
                app_key
            }
            (Field::Token, Self::DeepL { api_key, .. })
                if profile.supports_text_translation_configuration()
                    && profile.text_translation() == TextTranslation::DeepL
                    && expected_text_translation == Some(TextTranslation::DeepL) =>
            {
                api_key
            }
            (Field::Token, Self::DeepLX { token, .. })
                if profile.supports_text_translation_configuration()
                    && profile.text_translation() == TextTranslation::DeepLX
                    && expected_text_translation == Some(TextTranslation::DeepLX) =>
            {
                token
            }
            (Field::Token, Self::OpenAICompatible { api_key, .. })
                if profile.supports_text_translation_configuration()
                    && profile.text_translation() == TextTranslation::OpenAICompatible
                    && expected_text_translation == Some(TextTranslation::OpenAICompatible) =>
            {
                api_key
            }
            (Field::Token, Self::ChatMock { api_key, .. })
                if profile.supports_text_translation_configuration()
                    && profile.text_translation() == TextTranslation::ChatMock
                    && expected_text_translation == Some(TextTranslation::ChatMock) =>
            {
                api_key
            }
            _ => return Err(ProviderCredentialsError::InvalidRevealField),
        };
        if value.chars().count() > MAXIMUM_CREDENTIAL_FIELD_LENGTH
            || value.chars().any(char::is_control)
        {
            return Err(ProviderCredentialsError::InvalidStoredValue);
        }
        Ok((!value.is_empty()).then_some(value.as_str()))
    }

    pub fn api_key(value: impl Into<String>) -> Self {
        Self::ApiKey {
            api_key: value.into(),
        }
    }

    pub const fn kind_label(&self) -> &'static str {
        match self {
            Self::AppleSpeech => "apple_speech",
            Self::AlibabaTranslation { .. } => "alibaba_translation_update",
            Self::DeepLX { .. } => "deeplx",
            Self::DeepL { .. } => "deepl",
            Self::OpenAICompatible { .. } => "openai_compatible",
            Self::ChatMock { .. } => "chat_mock",
            Self::ApiKey { .. } => "api_key",
            Self::CustomSpeech { .. } => "custom_speech",
            Self::AzureOpenAI { .. } => "azure_openai",
            Self::TencentCloud { .. } => "tencent_cloud",
            Self::BaiduTranslate { .. } => "baidu_translate",
        }
    }

    pub fn validated_for_speech_mode(
        &self,
        provider: ProviderKind,
        mode: SpeechTranslationMode,
    ) -> Result<Self, ProviderCredentialsError> {
        if provider == ProviderKind::AzureOpenAIRealtime && mode == SpeechTranslationMode::AsrMt {
            if let Self::AzureOpenAI {
                endpoint,
                deployment,
                transcription_deployment,
                api_key,
            } = self
            {
                return Ok(Self::AzureOpenAI {
                    endpoint: validated_azure_endpoint(endpoint)?,
                    deployment: if deployment.trim().is_empty() {
                        String::new()
                    } else {
                        validated_deployment(deployment)?
                    },
                    transcription_deployment: validated_deployment(transcription_deployment)?,
                    api_key: required_field(api_key, provider)?,
                });
            }
        }
        self.validated_for(provider)
    }

    pub fn encode_for_profile(
        &self,
        profile: &ServiceProfile,
    ) -> Result<String, ProviderCredentialsError> {
        let credentials =
            self.validated_for_speech_mode(profile.provider, profile.speech_translation_mode)?;
        match credentials {
            Self::AppleSpeech => Err(ProviderCredentialsError::ProviderMismatch),
            Self::ApiKey { api_key } => Ok(api_key),
            other => serde_json::to_string(&other)
                .map_err(|_| ProviderCredentialsError::InvalidStoredValue),
        }
    }

    pub fn validated_for(&self, provider: ProviderKind) -> Result<Self, ProviderCredentialsError> {
        match (provider, self) {
            (ProviderKind::AppleSpeech, Self::AppleSpeech) => Ok(Self::AppleSpeech),
            (
                provider,
                Self::CustomSpeech {
                    endpoint,
                    model,
                    api_key,
                },
            ) if provider.is_custom_speech() => Ok(Self::CustomSpeech {
                endpoint: crate::core::protocols::recognition::custom_speech::endpoint(
                    endpoint, provider,
                )
                .map_err(|_| ProviderCredentialsError::InvalidCustomSpeechEndpoint)?
                .to_string(),
                model: crate::core::protocols::recognition::custom_speech::validate_model(model)
                    .map_err(|_| ProviderCredentialsError::InvalidCustomSpeechModel)?,
                api_key: required_field(api_key, provider)?,
            }),
            (
                ProviderKind::AlibabaCloud,
                Self::DeepL {
                    asr_api_key,
                    api_key,
                },
            ) => Ok(Self::DeepL {
                asr_api_key: required_field(asr_api_key, provider)?,
                api_key: required_field(api_key, provider)?,
            }),
            (
                ProviderKind::AlibabaCloud,
                Self::OpenAICompatible {
                    asr_api_key,
                    endpoint,
                    api_key,
                    model,
                },
            ) => Ok(Self::OpenAICompatible {
                asr_api_key: required_field(asr_api_key, provider)?,
                endpoint: crate::core::protocols::translation::openai_compatible::endpoint(
                    endpoint,
                )
                .map_err(|_| ProviderCredentialsError::InvalidOpenAICompatibleEndpoint)?
                .to_string(),
                api_key: optional_openai_compatible_key(api_key)?,
                model: crate::core::protocols::translation::openai_compatible::validate_model(
                    model,
                )
                .map_err(|_| ProviderCredentialsError::InvalidOpenAICompatibleModel)?,
            }),
            (
                ProviderKind::AlibabaCloud,
                Self::ChatMock {
                    asr_api_key,
                    endpoint,
                    api_key,
                    model,
                },
            ) => Ok(Self::ChatMock {
                asr_api_key: required_field(asr_api_key, provider)?,
                endpoint: crate::core::protocols::translation::openai_compatible::endpoint(
                    endpoint,
                )
                .map_err(|_| ProviderCredentialsError::InvalidOpenAICompatibleEndpoint)?
                .to_string(),
                api_key: optional_openai_compatible_key(api_key)?,
                model: crate::core::protocols::translation::openai_compatible::validate_model(
                    model,
                )
                .map_err(|_| ProviderCredentialsError::InvalidOpenAICompatibleModel)?,
            }),
            (
                ProviderKind::DeepLX,
                Self::DeepLX {
                    asr_api_key,
                    endpoint,
                    token,
                },
            ) => Ok(Self::DeepLX {
                asr_api_key: required_field(asr_api_key, provider)?,
                endpoint: crate::core::protocols::translation::deeplx::endpoint(endpoint)
                    .map_err(|_| ProviderCredentialsError::InvalidDeepLXEndpoint)?
                    .to_string(),
                token: if token.trim().is_empty() {
                    String::new()
                } else {
                    required_field(token, provider)?
                },
            }),
            (provider, Self::ApiKey { api_key }) if provider.uses_api_key_only() => {
                Ok(Self::ApiKey {
                    api_key: required_field(api_key, provider)?,
                })
            }
            (
                ProviderKind::AzureOpenAIRealtime,
                Self::AzureOpenAI {
                    endpoint,
                    deployment,
                    transcription_deployment,
                    api_key,
                },
            ) => Ok(Self::AzureOpenAI {
                endpoint: validated_azure_endpoint(endpoint)?,
                deployment: validated_deployment(deployment)?,
                transcription_deployment: validated_deployment(transcription_deployment)?,
                api_key: required_field(api_key, provider)?,
            }),
            (
                ProviderKind::TencentCloud,
                Self::TencentCloud {
                    app_id,
                    secret_id,
                    secret_key,
                },
            ) => {
                let app_id = required_field(app_id, provider)?;
                if !app_id.bytes().all(|byte| byte.is_ascii_digit()) || app_id.len() > 32 {
                    return Err(ProviderCredentialsError::InvalidField);
                }
                Ok(Self::TencentCloud {
                    app_id,
                    secret_id: required_field(secret_id, provider)?,
                    secret_key: required_field(secret_key, provider)?,
                })
            }
            (ProviderKind::BaiduTranslate, Self::BaiduTranslate { app_id, app_key }) => {
                Ok(Self::BaiduTranslate {
                    app_id: identifier_field(app_id, provider)?,
                    app_key: required_field(app_key, provider)?,
                })
            }
            _ => Err(ProviderCredentialsError::ProviderMismatch),
        }
    }

    pub fn encode_for_keychain(
        &self,
        provider: ProviderKind,
    ) -> Result<String, ProviderCredentialsError> {
        let credentials = self.validated_for(provider)?;
        match credentials {
            Self::AppleSpeech => Err(ProviderCredentialsError::ProviderMismatch),
            Self::ApiKey { api_key } => Ok(api_key),
            other => serde_json::to_string(&other)
                .map_err(|_| ProviderCredentialsError::InvalidStoredValue),
        }
    }

    pub fn decode_from_keychain(
        provider: ProviderKind,
        value: &str,
    ) -> Result<Self, ProviderCredentialsError> {
        if provider == ProviderKind::AppleSpeech {
            return Err(ProviderCredentialsError::ProviderMismatch);
        }
        if value.trim().is_empty() {
            return Err(ProviderCredentialsError::Missing(provider));
        }
        if provider.uses_api_key_only() {
            return Self::api_key(value).validated_for(provider);
        }
        serde_json::from_str::<Self>(value)
            .map_err(|_| ProviderCredentialsError::InvalidStoredValue)?
            .validated_for(provider)
    }

    /// Recognizes only the two historical Alibaba credential representations.
    /// The profile account/provider never changes when its text route changes.
    pub fn decode_for_profile(
        profile: &ServiceProfile,
        value: &str,
    ) -> Result<Self, ProviderCredentialsError> {
        if profile.provider == ProviderKind::AzureOpenAIRealtime && profile.uses_realtime_asr() {
            return serde_json::from_str::<Self>(value)
                .map_err(|_| ProviderCredentialsError::InvalidStoredValue)?
                .validated_for_speech_mode(profile.provider, profile.speech_translation_mode);
        }
        if !matches!(
            profile.provider,
            ProviderKind::AlibabaCloud | ProviderKind::DeepLX
        ) {
            return Self::decode_from_keychain(profile.provider, value);
        }
        if profile.provider == ProviderKind::AlibabaCloud {
            return Self::decode_from_keychain(ProviderKind::AlibabaCloud, value);
        }
        let credentials = if !value.trim_start().starts_with('{') {
            Self::api_key(value).validated_for(ProviderKind::AlibabaCloud)?
        } else {
            let stored = serde_json::from_str::<Self>(value)
                .map_err(|_| ProviderCredentialsError::InvalidStoredValue)?;
            stored.validated_for(if matches!(stored, Self::DeepL { .. }) {
                ProviderKind::AlibabaCloud
            } else {
                ProviderKind::DeepLX
            })?
        };
        Ok(credentials)
    }

    pub fn alibaba_key(&self) -> Option<&str> {
        match self {
            Self::ApiKey { api_key } => Some(api_key),
            Self::DeepLX { asr_api_key, .. }
            | Self::DeepL { asr_api_key, .. }
            | Self::OpenAICompatible { asr_api_key, .. }
            | Self::ChatMock { asr_api_key, .. } => Some(asr_api_key),
            _ => None,
        }
    }

    pub fn direct_api_key(&self) -> Option<&str> {
        match self {
            Self::ApiKey { api_key } => Some(api_key),
            Self::AzureOpenAI { api_key, .. } => Some(api_key),
            Self::CustomSpeech { api_key, .. } => Some(api_key),
            Self::AppleSpeech
            | Self::AlibabaTranslation { .. }
            | Self::DeepLX { .. }
            | Self::DeepL { .. }
            | Self::OpenAICompatible { .. }
            | Self::ChatMock { .. }
            | Self::TencentCloud { .. }
            | Self::BaiduTranslate { .. } => None,
        }
    }

    pub fn azure_openai(&self) -> Option<(&str, &str, &str, &str)> {
        match self {
            Self::AzureOpenAI {
                endpoint,
                deployment,
                transcription_deployment,
                api_key,
            } => Some((endpoint, deployment, transcription_deployment, api_key)),
            _ => None,
        }
    }

    pub fn tencent_cloud(&self) -> Option<(&str, &str, &str)> {
        match self {
            Self::TencentCloud {
                app_id,
                secret_id,
                secret_key,
            } => Some((app_id, secret_id, secret_key)),
            _ => None,
        }
    }

    pub fn baidu_translate(&self) -> Option<(&str, &str)> {
        match self {
            Self::BaiduTranslate { app_id, app_key } => Some((app_id, app_key)),
            _ => None,
        }
    }
}

/// No-auth OpenAI-compatible destinations still have a validated endpoint and model.
/// Never substitute a recognition key for an empty text-service key.
fn optional_openai_compatible_key(value: &str) -> Result<String, ProviderCredentialsError> {
    if value.chars().any(char::is_control)
        || value.trim().chars().count() > MAXIMUM_CREDENTIAL_FIELD_LENGTH
    {
        return Err(ProviderCredentialsError::InvalidField);
    }
    Ok(value.trim().to_owned())
}

fn required_field(value: &str, provider: ProviderKind) -> Result<String, ProviderCredentialsError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ProviderCredentialsError::Missing(provider));
    }
    if value.chars().count() > MAXIMUM_CREDENTIAL_FIELD_LENGTH
        || value.chars().any(char::is_control)
    {
        return Err(ProviderCredentialsError::InvalidField);
    }
    Ok(value.to_string())
}

fn identifier_field(
    value: &str,
    provider: ProviderKind,
) -> Result<String, ProviderCredentialsError> {
    let value = required_field(value, provider)?;
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(ProviderCredentialsError::InvalidField);
    }
    Ok(value)
}

fn validated_deployment(value: &str) -> Result<String, ProviderCredentialsError> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > MAXIMUM_DEPLOYMENT_LENGTH
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(ProviderCredentialsError::InvalidAzureDeployment);
    }
    Ok(value.to_string())
}

fn validated_azure_endpoint(value: &str) -> Result<String, ProviderCredentialsError> {
    let value = value.trim().trim_end_matches('/');
    let endpoint =
        url::Url::parse(value).map_err(|_| ProviderCredentialsError::InvalidAzureEndpoint)?;
    let host = endpoint
        .host_str()
        .ok_or(ProviderCredentialsError::InvalidAzureEndpoint)?
        .to_ascii_lowercase();
    let official_host = [".openai.azure.com", ".openai.azure.cn", ".openai.azure.us"]
        .iter()
        .any(|suffix| host.ends_with(suffix) && host.len() > suffix.len());
    if endpoint.scheme() != "https"
        || !official_host
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.port().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
        || !matches!(endpoint.path(), "" | "/")
    {
        return Err(ProviderCredentialsError::InvalidAzureEndpoint);
    }
    Ok(format!("https://{host}"))
}

#[cfg(test)]
mod tests;
