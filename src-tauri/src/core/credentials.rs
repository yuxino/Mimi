//! Provider credentials and credential-free local recognition configuration.

use crate::core::provider::{ProviderKind, ServiceProfile, TextTranslation};
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
                profile.provider.supports_text_translation() && !profile.provider.is_local_speech()
            }
            Self::Token => {
                profile.provider.supports_text_translation()
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
    /// Windows captions are local and have no persisted recognition credential.
    WindowsLiveCaptions,
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
#[derive(Clone, PartialEq, Eq)]
pub enum TextTranslationCredentials {
    /// Local translation stores no private destination or credential.
    Apple,
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
        let saved = saved.filter(|value| value.translation() == route);
        let (old_endpoint, old_model, old_key) = match saved {
            Some(Self::DeepL { api_key }) => ("", "", api_key.as_str()),
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
            crate::core::protocols::openai_compatible::endpoint(endpoint)
                .map_err(|_| ProviderCredentialsError::InvalidOpenAICompatibleEndpoint)?
                .to_string()
        } else if route == TextTranslation::DeepLX {
            crate::core::protocols::deeplx::endpoint(endpoint)
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
            Self::Apple => Ok(Self::Apple),
            Self::DeepL { api_key } => Ok(Self::DeepL {
                api_key: required(api_key)?,
            }),
            Self::DeepLX { endpoint, token } => Ok(Self::DeepLX {
                endpoint: crate::core::protocols::deeplx::endpoint(endpoint)
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
                endpoint: crate::core::protocols::openai_compatible::endpoint(endpoint)
                    .map_err(|_| ProviderCredentialsError::InvalidOpenAICompatibleEndpoint)?
                    .to_string(),
                model: crate::core::protocols::openai_compatible::validate_model(model)
                    .map_err(|_| ProviderCredentialsError::InvalidOpenAICompatibleModel)?,
                api_key: optional_openai_compatible_key(api_key)?,
            }),
            Self::ChatMock {
                endpoint,
                model,
                api_key,
            } => Ok(Self::ChatMock {
                endpoint: crate::core::protocols::openai_compatible::endpoint(endpoint)
                    .map_err(|_| ProviderCredentialsError::InvalidOpenAICompatibleEndpoint)?
                    .to_string(),
                model: crate::core::protocols::openai_compatible::validate_model(model)
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
                    crate::core::protocols::custom_speech::endpoint(endpoint, provider)
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
                let api_key = retain(
                    api_key,
                    if endpoint == *old_endpoint {
                        old_key
                    } else {
                        ""
                    },
                );
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
        merged.validated_for(provider)
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
                if profile.provider.supports_text_translation()
                    && profile.text_translation() == TextTranslation::DeepL
                    && expected_text_translation == Some(TextTranslation::DeepL) =>
            {
                api_key
            }
            (Field::Token, Self::DeepLX { token, .. })
                if profile.provider.supports_text_translation()
                    && profile.text_translation() == TextTranslation::DeepLX
                    && expected_text_translation == Some(TextTranslation::DeepLX) =>
            {
                token
            }
            (Field::Token, Self::OpenAICompatible { api_key, .. })
                if profile.provider.supports_text_translation()
                    && profile.text_translation() == TextTranslation::OpenAICompatible
                    && expected_text_translation == Some(TextTranslation::OpenAICompatible) =>
            {
                api_key
            }
            (Field::Token, Self::ChatMock { api_key, .. })
                if profile.provider.supports_text_translation()
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
            Self::WindowsLiveCaptions => "windows_live_captions",
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

    pub fn validated_for(&self, provider: ProviderKind) -> Result<Self, ProviderCredentialsError> {
        match (provider, self) {
            (ProviderKind::AppleSpeech, Self::AppleSpeech) => Ok(Self::AppleSpeech),
            (ProviderKind::WindowsLiveCaptions, Self::WindowsLiveCaptions) => {
                Ok(Self::WindowsLiveCaptions)
            }
            (
                provider,
                Self::CustomSpeech {
                    endpoint,
                    model,
                    api_key,
                },
            ) if provider.is_custom_speech() => Ok(Self::CustomSpeech {
                endpoint: crate::core::protocols::custom_speech::endpoint(endpoint, provider)
                    .map_err(|_| ProviderCredentialsError::InvalidCustomSpeechEndpoint)?
                    .to_string(),
                model: crate::core::protocols::custom_speech::validate_model(model)
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
                endpoint: crate::core::protocols::openai_compatible::endpoint(endpoint)
                    .map_err(|_| ProviderCredentialsError::InvalidOpenAICompatibleEndpoint)?
                    .to_string(),
                api_key: optional_openai_compatible_key(api_key)?,
                model: crate::core::protocols::openai_compatible::validate_model(model)
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
                endpoint: crate::core::protocols::openai_compatible::endpoint(endpoint)
                    .map_err(|_| ProviderCredentialsError::InvalidOpenAICompatibleEndpoint)?
                    .to_string(),
                api_key: optional_openai_compatible_key(api_key)?,
                model: crate::core::protocols::openai_compatible::validate_model(model)
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
                endpoint: crate::core::protocols::deeplx::endpoint(endpoint)
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
            Self::AppleSpeech | Self::WindowsLiveCaptions => {
                Err(ProviderCredentialsError::ProviderMismatch)
            }
            Self::ApiKey { api_key } => Ok(api_key),
            other => serde_json::to_string(&other)
                .map_err(|_| ProviderCredentialsError::InvalidStoredValue),
        }
    }

    pub fn decode_from_keychain(
        provider: ProviderKind,
        value: &str,
    ) -> Result<Self, ProviderCredentialsError> {
        if provider.is_local_speech() {
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
            | Self::WindowsLiveCaptions
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
mod tests {
    #[test]
    fn apple_credentials_cannot_be_saved_decoded_or_revealed_as_a_secret() {
        let profile = ServiceProfile::new("apple", "Apple", ProviderKind::AppleSpeech).unwrap();
        assert_eq!(
            ProviderCredentials::AppleSpeech.validated_for(profile.provider),
            Ok(ProviderCredentials::AppleSpeech)
        );
        assert!(ProviderCredentials::AppleSpeech
            .encode_for_keychain(profile.provider)
            .is_err());
        assert!(ProviderCredentials::decode_from_keychain(
            profile.provider,
            "{\"kind\":\"appleSpeech\"}"
        )
        .is_err());
        assert!(!CredentialRevealField::AsrApiKey.allowed_for(&profile, None));
        assert!(!CredentialRevealField::ApiKey.allowed_for(&profile, None));
        assert_eq!(ProviderCredentials::AppleSpeech.direct_api_key(), None);
    }

    use super::*;

    #[test]
    fn text_draft_merging_binds_optional_auth_to_route_and_endpoint() {
        for route in [
            TextTranslation::OpenAICompatible,
            TextTranslation::ChatMock,
            TextTranslation::DeepLX,
        ] {
            let saved = TextTranslationCredentials::resolve_update(
                route,
                None,
                "https://text.example/v1",
                Some("synthetic-saved"),
                "saved-model",
            )
            .unwrap();
            let same = TextTranslationCredentials::resolve_update(
                route,
                Some(&saved),
                "https://text.example/v1",
                None,
                "draft-model",
            )
            .unwrap();
            let changed = TextTranslationCredentials::resolve_update(
                route,
                Some(&saved),
                "https://other.example/v1",
                None,
                "draft-model",
            )
            .unwrap();
            let key = |value: &TextTranslationCredentials| match value {
                TextTranslationCredentials::OpenAICompatible { api_key, .. }
                | TextTranslationCredentials::ChatMock { api_key, .. } => api_key.clone(),
                TextTranslationCredentials::DeepLX { token, .. } => token.clone(),
                _ => unreachable!(),
            };
            assert_eq!(key(&same), "synthetic-saved");
            assert!(key(&changed).is_empty());
            let cleared =
                TextTranslationCredentials::resolve_update(route, Some(&saved), "", Some(""), "")
                    .unwrap();
            assert!(key(&cleared).is_empty());
            if route.uses_chat_completions() {
                let other_route = if route == TextTranslation::ChatMock {
                    TextTranslation::OpenAICompatible
                } else {
                    TextTranslation::ChatMock
                };
                let other = TextTranslationCredentials::resolve_update(
                    other_route,
                    Some(&saved),
                    "https://text.example/v1",
                    None,
                    "other-model",
                )
                .unwrap();
                assert!(key(&other).is_empty());
            }
        }
    }

    #[test]
    fn general_drafts_reuse_keys_only_for_unchanged_service_identity() {
        let azure = ProviderCredentials::AzureOpenAI {
            endpoint: "https://old.openai.azure.com".into(),
            deployment: "translate".into(),
            transcription_deployment: "transcribe".into(),
            api_key: "synthetic-azure".into(),
        };
        let tencent = ProviderCredentials::TencentCloud {
            app_id: "123".into(),
            secret_id: "synthetic-id".into(),
            secret_key: "synthetic-key".into(),
        };
        let baidu = ProviderCredentials::BaiduTranslate {
            app_id: "123".into(),
            app_key: "synthetic-baidu".into(),
        };
        for (provider, saved, blank, changed) in [
            (
                ProviderKind::AzureOpenAIRealtime,
                azure,
                ProviderCredentials::AzureOpenAI {
                    endpoint: String::new(),
                    deployment: "new-deployment".into(),
                    transcription_deployment: String::new(),
                    api_key: String::new(),
                },
                ProviderCredentials::AzureOpenAI {
                    endpoint: "https://new.openai.azure.com".into(),
                    deployment: String::new(),
                    transcription_deployment: String::new(),
                    api_key: String::new(),
                },
            ),
            (
                ProviderKind::TencentCloud,
                tencent,
                ProviderCredentials::TencentCloud {
                    app_id: "123".into(),
                    secret_id: String::new(),
                    secret_key: String::new(),
                },
                ProviderCredentials::TencentCloud {
                    app_id: "456".into(),
                    secret_id: String::new(),
                    secret_key: String::new(),
                },
            ),
            (
                ProviderKind::BaiduTranslate,
                baidu,
                ProviderCredentials::BaiduTranslate {
                    app_id: "123".into(),
                    app_key: String::new(),
                },
                ProviderCredentials::BaiduTranslate {
                    app_id: "456".into(),
                    app_key: String::new(),
                },
            ),
        ] {
            assert!(blank.resolve_draft(provider, Some(&saved)).is_ok());
            assert!(changed.resolve_draft(provider, Some(&saved)).is_err());
        }
    }

    #[test]
    fn custom_speech_draft_does_not_reuse_key_or_model_for_changed_endpoint() {
        let saved = ProviderCredentials::CustomSpeech {
            endpoint: "wss://speech.example/realtime".into(),
            model: "saved-model".into(),
            api_key: "synthetic-key".into(),
        };
        let draft = ProviderCredentials::CustomSpeech {
            endpoint: String::new(),
            model: "new-model".into(),
            api_key: String::new(),
        };
        let resolved = draft
            .resolve_draft(ProviderKind::CustomOpenAIASR, Some(&saved))
            .unwrap();
        assert!(
            matches!(resolved, ProviderCredentials::CustomSpeech { api_key, model, .. } if api_key == "synthetic-key" && model == "new-model")
        );
        let changed = ProviderCredentials::CustomSpeech {
            endpoint: "wss://other.example/realtime".into(),
            model: "new-model".into(),
            api_key: String::new(),
        };
        assert!(changed
            .resolve_draft(ProviderKind::CustomOpenAIASR, Some(&saved))
            .is_err());
    }

    #[test]
    fn custom_speech_credentials_are_independent_scoped_and_debug_redacted() {
        for provider in [
            ProviderKind::CustomDashScopeASR,
            ProviderKind::CustomOpenAIASR,
        ] {
            let credentials = ProviderCredentials::CustomSpeech {
                endpoint: " wss://speech.example/recognition ".into(),
                model: " synthetic-speech-model ".into(),
                api_key: " synthetic-speech-key ".into(),
            };
            let encoded = credentials.encode_for_keychain(provider).unwrap();
            let validated = ProviderCredentials::decode_from_keychain(provider, &encoded).unwrap();
            assert_eq!(validated.alibaba_key(), None);
            assert_eq!(validated.direct_api_key(), Some("synthetic-speech-key"));
            assert!(
                matches!(&validated, ProviderCredentials::CustomSpeech { endpoint, model, .. }
                if endpoint == "wss://speech.example/recognition" && model == "synthetic-speech-model")
            );
            assert_eq!(
                validated.validated_for(ProviderKind::AlibabaCloud),
                Err(ProviderCredentialsError::ProviderMismatch)
            );
            assert_eq!(
                ProviderCredentials::api_key("synthetic").validated_for(provider),
                Err(ProviderCredentialsError::ProviderMismatch)
            );
            let profile = ServiceProfile::new("custom", "Speech", provider).unwrap();
            assert_eq!(
                validated.revealed_field(&profile, CredentialRevealField::ApiKey, None),
                Ok(Some("synthetic-speech-key"))
            );
            assert_eq!(
                validated.revealed_field(&profile, CredentialRevealField::Token, None),
                Err(ProviderCredentialsError::InvalidRevealField)
            );
            for value in [
                "speech.example",
                "synthetic-speech-model",
                "synthetic-speech-key",
            ] {
                assert!(!format!("{validated:?}").contains(value));
            }
        }
        assert!(serde_json::from_str::<ProviderCredentials>(r#"{"kind":"customSpeech","endpoint":"wss://speech.example","model":"model","apiKey":"synthetic","token":"wrong-destination"}"#).is_err());
    }

    #[test]
    fn independent_text_credentials_validate_without_any_recognition_key() {
        let destinations = [
            TextTranslationCredentials::DeepL {
                api_key: " synthetic-deepl:fx ".into(),
            },
            TextTranslationCredentials::DeepLX {
                endpoint: "https://destination.example/translate".into(),
                token: " synthetic-deeplx ".into(),
            },
            TextTranslationCredentials::OpenAICompatible {
                endpoint: "https://destination.example/v1".into(),
                model: " synthetic-text-model ".into(),
                api_key: " synthetic-text-key ".into(),
            },
        ];
        for destination in destinations {
            let validated = destination.validated().unwrap();
            assert_eq!(validated.translation(), destination.translation());
            for value in [
                "destination.example",
                "synthetic-deepl",
                "synthetic-deeplx",
                "synthetic-text-model",
                "synthetic-text-key",
            ] {
                assert!(!format!("{validated:?}").contains(value));
            }
        }
        assert_eq!(
            TextTranslationCredentials::DeepL { api_key: "".into() }.validated(),
            Err(ProviderCredentialsError::MissingTextTranslation)
        );
        let anonymous = TextTranslationCredentials::OpenAICompatible {
            endpoint: "http://127.0.0.1:8000/v1".into(),
            model: "model".into(),
            api_key: "".into(),
        }
        .validated()
        .unwrap();
        assert!(
            matches!(anonymous, TextTranslationCredentials::OpenAICompatible { api_key, .. } if api_key.is_empty())
        );
    }

    #[test]
    fn credential_reveal_is_single_field_and_provider_scoped() {
        let tencent =
            ServiceProfile::new("tencent", "Tencent", ProviderKind::TencentCloud).unwrap();
        let credentials = ProviderCredentials::TencentCloud {
            app_id: "123".into(),
            secret_id: "synthetic-id".into(),
            secret_key: "synthetic-key".into(),
        };
        assert_eq!(
            credentials.revealed_field(&tencent, CredentialRevealField::SecretId, None),
            Ok(Some("synthetic-id"))
        );
        assert_eq!(
            credentials.revealed_field(&tencent, CredentialRevealField::SecretKey, None),
            Ok(Some("synthetic-key"))
        );
        let alibaba = ServiceProfile::alibaba_default();
        assert_eq!(
            credentials.revealed_field(&alibaba, CredentialRevealField::SecretKey, None),
            Err(ProviderCredentialsError::InvalidRevealField)
        );
        assert_eq!(
            credentials.revealed_field(&tencent, CredentialRevealField::ApiKey, None),
            Err(ProviderCredentialsError::InvalidRevealField)
        );
        assert!(!format!("{credentials:?}").contains("synthetic"));
        for field in ["endpoint", "appId", "api_key", "unknown"] {
            assert!(
                serde_json::from_str::<CredentialRevealField>(&format!("\"{field}\"")).is_err()
            );
        }
    }

    #[test]
    fn credential_reveal_requires_the_saved_independent_route() {
        let mut profile = ServiceProfile::alibaba_default();
        profile.text_translation = Some(TextTranslation::DeepL);
        let official = ProviderCredentials::DeepL {
            asr_api_key: "synthetic-asr".into(),
            api_key: "synthetic-official:fx".into(),
        };
        assert_eq!(
            official.revealed_field(
                &profile,
                CredentialRevealField::Token,
                Some(TextTranslation::DeepL)
            ),
            Ok(Some("synthetic-official:fx"))
        );
        assert_eq!(
            official.revealed_field(&profile, CredentialRevealField::ApiKey, None),
            Ok(Some("synthetic-asr"))
        );
        for route in [
            None,
            Some(TextTranslation::DeepLX),
            Some(TextTranslation::FollowService),
        ] {
            assert_eq!(
                official.revealed_field(&profile, CredentialRevealField::Token, route),
                Err(ProviderCredentialsError::InvalidRevealField)
            );
        }
        profile.text_translation = Some(TextTranslation::DeepLX);
        let custom = ProviderCredentials::DeepLX {
            asr_api_key: "synthetic-asr".into(),
            endpoint: "https://example.com/translate".into(),
            token: String::new(),
        };
        assert_eq!(
            custom.revealed_field(
                &profile,
                CredentialRevealField::Token,
                Some(TextTranslation::DeepLX)
            ),
            Ok(None)
        );
        let mut legacy = ServiceProfile::new("legacy", "Legacy", ProviderKind::DeepLX).unwrap();
        legacy.text_translation = Some(TextTranslation::FollowService);
        assert_eq!(
            ProviderCredentials::api_key("synthetic-asr").revealed_field(
                &legacy,
                CredentialRevealField::AsrApiKey,
                None
            ),
            Ok(Some("synthetic-asr"))
        );
        let request = ProviderCredentials::AlibabaTranslation {
            api_key: "synthetic-asr".into(),
            text_translation: TextTranslation::DeepLX,
            endpoint: String::new(),
            token: "synthetic-request".into(),
            model: String::new(),
            clear_token: false,
        };
        assert_eq!(
            request.revealed_field(
                &profile,
                CredentialRevealField::Token,
                Some(TextTranslation::DeepLX)
            ),
            Err(ProviderCredentialsError::InvalidRevealField)
        );
    }

    #[test]
    fn credential_reveal_rejects_corrupt_field_without_echoing_it() {
        let profile = ServiceProfile::alibaba_default();
        for value in ["synthetic-private\nbody".into(), "x".repeat(1_025)] {
            let error = ProviderCredentials::api_key(value)
                .revealed_field(&profile, CredentialRevealField::ApiKey, None)
                .unwrap_err();
            assert_eq!(error, ProviderCredentialsError::InvalidStoredValue);
            assert!(!error.to_string().contains("synthetic-private"));
        }
    }

    #[test]
    fn alibaba_translation_request_is_write_only_and_debug_redacted() {
        let request: ProviderCredentials = serde_json::from_str(r#"{"kind":"alibabaTranslation","apiKey":"","textTranslation":"deepLX","endpoint":"https://example.com","token":"synthetic-token"}"#).unwrap();
        assert!(
            matches!(&request, ProviderCredentials::AlibabaTranslation { api_key, text_translation: TextTranslation::DeepLX, clear_token: false, .. } if api_key.is_empty())
        );
        let clear_request: ProviderCredentials = serde_json::from_str(r#"{"kind":"alibabaTranslation","apiKey":"","textTranslation":"openAICompatible","endpoint":"","token":"","clearToken":true}"#).unwrap();
        let encoded = serde_json::to_value(&clear_request).unwrap();
        assert_eq!(encoded["clearToken"], true);
        assert_eq!(
            serde_json::from_value::<ProviderCredentials>(encoded).unwrap(),
            clear_request
        );
        assert!(request
            .encode_for_keychain(ProviderKind::AlibabaCloud)
            .is_err());
        assert!(request.encode_for_keychain(ProviderKind::DeepLX).is_err());
        for value in ["example.com", "synthetic-token"] {
            assert!(!format!("{request:?}").contains(value));
        }
        let mut legacy = ServiceProfile::new("existing", "Existing", ProviderKind::DeepLX).unwrap();
        legacy.text_translation = Some(TextTranslation::FollowService);
        assert!(
            matches!(ProviderCredentials::decode_for_profile(&legacy, "synthetic-asr").unwrap(), ProviderCredentials::ApiKey { api_key } if api_key == "synthetic-asr")
        );
    }

    #[test]
    fn deeplx_credentials_round_trip_securely_and_validate_optional_token() {
        let value = ProviderCredentials::DeepLX {
            asr_api_key: "synthetic-asr".into(),
            endpoint: "https://example.com/api".into(),
            token: "".into(),
        };
        let ipc: ProviderCredentials = serde_json::from_str(r#"{"kind":"deepLX","asrApiKey":"synthetic-asr","endpoint":"https://example.com/api","token":""}"#).unwrap();
        assert_eq!(ipc, value);
        let encoded = value.encode_for_keychain(ProviderKind::DeepLX).unwrap();
        let decoded =
            ProviderCredentials::decode_from_keychain(ProviderKind::DeepLX, &encoded).unwrap();
        assert!(
            matches!(&decoded, ProviderCredentials::DeepLX { endpoint, token, .. } if endpoint == "https://example.com/api/translate" && token.is_empty())
        );
        assert!(!format!("{decoded:?}").contains("synthetic-asr"));
        assert!(!format!("{decoded:?}").contains("example.com"));
        assert!(value.validated_for(ProviderKind::AlibabaCloud).is_err());
        let bad = ProviderCredentials::DeepLX {
            asr_api_key: "".into(),
            endpoint: "https://example.com".into(),
            token: "token".into(),
        };
        assert!(bad.validated_for(ProviderKind::DeepLX).is_err());
        let bad = ProviderCredentials::DeepLX {
            asr_api_key: "asr".into(),
            endpoint: "https://example.com".into(),
            token: "token\nInjected".into(),
        };
        assert!(bad.validated_for(ProviderKind::DeepLX).is_err());
    }

    #[test]
    fn historical_single_api_keys_remain_raw_and_readable() {
        let credentials = ProviderCredentials::decode_from_keychain(
            ProviderKind::OpenAIRealtime,
            "  sk-legacy  ",
        )
        .unwrap();
        assert_eq!(
            credentials
                .encode_for_keychain(ProviderKind::OpenAIRealtime)
                .unwrap(),
            "sk-legacy"
        );
    }

    #[test]
    fn openai_compatible_credentials_are_scoped_trimmed_and_redacted() {
        let credentials: ProviderCredentials = serde_json::from_str(
            r#"{"kind":"openAICompatible","asrApiKey":" synthetic-asr ","endpoint":"https://example.com/v1","apiKey":" synthetic-third-party ","model":" synthetic-model "}"#,
        )
        .unwrap();
        let validated = credentials
            .validated_for(ProviderKind::AlibabaCloud)
            .unwrap();
        assert_eq!(validated.alibaba_key(), Some("synthetic-asr"));
        let ProviderCredentials::OpenAICompatible {
            endpoint,
            api_key,
            model,
            ..
        } = &validated
        else {
            panic!("expected OpenAI-compatible credentials")
        };
        assert!(endpoint.starts_with("https://example.com/"));
        assert_eq!(api_key, "synthetic-third-party");
        assert_eq!(model, "synthetic-model");
        assert_eq!(validated.direct_api_key(), None);
        assert_eq!(
            validated.validated_for(ProviderKind::OpenAIRealtime),
            Err(ProviderCredentialsError::ProviderMismatch)
        );
        for private in [
            "synthetic-asr",
            "synthetic-third-party",
            "example.com",
            "synthetic-model",
        ] {
            assert!(!format!("{validated:?}").contains(private));
        }
        let mut profile = ServiceProfile::alibaba_default();
        profile.text_translation = Some(TextTranslation::OpenAICompatible);
        assert_eq!(
            validated.revealed_field(
                &profile,
                CredentialRevealField::Token,
                Some(TextTranslation::OpenAICompatible)
            ),
            Ok(Some("synthetic-third-party"))
        );
        assert_eq!(
            validated.revealed_field(
                &profile,
                CredentialRevealField::Token,
                Some(TextTranslation::DeepL)
            ),
            Err(ProviderCredentialsError::InvalidRevealField)
        );
    }

    #[test]
    fn openai_compatible_credentials_fail_closed_on_missing_or_invalid_fields() {
        for (asr_api_key, endpoint, api_key, model, expected) in [
            (
                "",
                "https://example.com/v1",
                "synthetic",
                "model",
                ProviderCredentialsError::Missing(ProviderKind::AlibabaCloud),
            ),
            (
                "asr",
                "http://example.com/v1",
                "synthetic",
                "model",
                ProviderCredentialsError::InvalidOpenAICompatibleEndpoint,
            ),
            (
                "asr",
                "https://example.com/v1?secret=synthetic",
                "synthetic",
                "model",
                ProviderCredentialsError::InvalidOpenAICompatibleEndpoint,
            ),
            (
                "asr",
                "https://example.com/v1",
                "synthetic\ninjected",
                "model",
                ProviderCredentialsError::InvalidField,
            ),
            (
                "asr",
                "https://example.com/v1",
                "synthetic",
                "",
                ProviderCredentialsError::InvalidOpenAICompatibleModel,
            ),
            (
                "asr",
                "https://example.com/v1",
                "synthetic",
                "synthetic\ninjected",
                ProviderCredentialsError::InvalidOpenAICompatibleModel,
            ),
        ] {
            let error = ProviderCredentials::OpenAICompatible {
                asr_api_key: asr_api_key.into(),
                endpoint: endpoint.into(),
                api_key: api_key.into(),
                model: model.into(),
            }
            .validated_for(ProviderKind::AlibabaCloud)
            .unwrap_err();
            assert_eq!(error, expected);
            assert!(!error.to_string().contains("synthetic"));
        }
        assert_eq!(
            ProviderCredentials::OpenAICompatible {
                asr_api_key: "asr".into(),
                endpoint: "https://example.com/v1".into(),
                api_key: "key".into(),
                model: "x".repeat(257),
            }
            .validated_for(ProviderKind::AlibabaCloud)
            .unwrap_err(),
            ProviderCredentialsError::InvalidOpenAICompatibleModel
        );
        let request: ProviderCredentials = serde_json::from_str(
            r#"{"kind":"alibabaTranslation","apiKey":"","textTranslation":"deepLX","endpoint":"","token":""}"#,
        )
        .unwrap();
        assert!(
            matches!(request, ProviderCredentials::AlibabaTranslation { model, .. } if model.is_empty())
        );
    }

    #[test]
    fn structured_credentials_round_trip_without_debug_disclosure() {
        let secret = "tencent-secret-value";
        let credentials = ProviderCredentials::TencentCloud {
            app_id: "123456".into(),
            secret_id: "AKIDexample".into(),
            secret_key: secret.into(),
        };
        let encoded = credentials
            .encode_for_keychain(ProviderKind::TencentCloud)
            .unwrap();
        assert_eq!(
            ProviderCredentials::decode_from_keychain(ProviderKind::TencentCloud, &encoded)
                .unwrap(),
            credentials
        );
        assert!(!format!("{credentials:?}").contains(secret));
    }

    #[test]
    fn frontend_camel_case_ipc_shapes_deserialize_for_every_credential_kind() {
        let cases = [
            (
                r#"{"kind":"apiKey","apiKey":"sk-test"}"#,
                ProviderCredentials::api_key("sk-test"),
            ),
            (
                r#"{"kind":"azureOpenAI","endpoint":"https://mimi.openai.azure.com","deployment":"translate","transcriptionDeployment":"transcribe","apiKey":"azure-secret"}"#,
                ProviderCredentials::AzureOpenAI {
                    endpoint: "https://mimi.openai.azure.com".into(),
                    deployment: "translate".into(),
                    transcription_deployment: "transcribe".into(),
                    api_key: "azure-secret".into(),
                },
            ),
            (
                r#"{"kind":"tencentCloud","appId":"123456","secretId":"AKIDexample","secretKey":"tencent-secret"}"#,
                ProviderCredentials::TencentCloud {
                    app_id: "123456".into(),
                    secret_id: "AKIDexample".into(),
                    secret_key: "tencent-secret".into(),
                },
            ),
            (
                r#"{"kind":"baiduTranslate","appId":"app_123","appKey":"baidu-secret"}"#,
                ProviderCredentials::BaiduTranslate {
                    app_id: "app_123".into(),
                    app_key: "baidu-secret".into(),
                },
            ),
        ];

        for (json, expected) in cases {
            assert_eq!(
                serde_json::from_str::<ProviderCredentials>(json).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn credential_ipc_shape_rejects_snake_case_and_unknown_fields() {
        for json in [
            r#"{"kind":"apiKey","api_key":"sk-test"}"#,
            r#"{"kind":"azureOpenAI","endpoint":"https://mimi.openai.azure.com","deployment":"translate","transcription_deployment":"transcribe","apiKey":"secret"}"#,
            r#"{"kind":"tencentCloud","appId":"123456","secretId":"AKIDexample","secretKey":"secret","extra":"not-allowed"}"#,
            r#"{"kind":"baiduTranslate","appId":"app_123","app_key":"secret"}"#,
        ] {
            assert!(serde_json::from_str::<ProviderCredentials>(json).is_err());
        }
    }

    #[test]
    fn providers_reject_the_wrong_credential_shape() {
        assert_eq!(
            ProviderCredentials::api_key("sk-test")
                .validated_for(ProviderKind::TencentCloud)
                .unwrap_err(),
            ProviderCredentialsError::ProviderMismatch
        );
    }

    #[test]
    fn azure_accepts_only_official_resource_endpoints() {
        let valid = ProviderCredentials::AzureOpenAI {
            endpoint: " https://mimi-test.openai.azure.com/ ".into(),
            deployment: " translate-prod ".into(),
            transcription_deployment: " transcribe-prod ".into(),
            api_key: " azure-secret ".into(),
        }
        .validated_for(ProviderKind::AzureOpenAIRealtime)
        .unwrap();
        assert_eq!(
            valid.azure_openai(),
            Some((
                "https://mimi-test.openai.azure.com",
                "translate-prod",
                "transcribe-prod",
                "azure-secret"
            ))
        );

        for endpoint in [
            "http://mimi.openai.azure.com",
            "https://example.com",
            "https://mimi.openai.azure.com/path",
            "https://mimi.openai.azure.com?secret=value",
            "https://user@mimi.openai.azure.com",
        ] {
            assert_eq!(
                ProviderCredentials::AzureOpenAI {
                    endpoint: endpoint.into(),
                    deployment: "translate".into(),
                    transcription_deployment: "transcribe".into(),
                    api_key: "secret".into(),
                }
                .validated_for(ProviderKind::AzureOpenAIRealtime)
                .unwrap_err(),
                ProviderCredentialsError::InvalidAzureEndpoint
            );
        }
    }

    #[test]
    fn every_required_value_is_trimmed_and_empty_values_fail_closed() {
        let baidu = ProviderCredentials::BaiduTranslate {
            app_id: " app_123 ".into(),
            app_key: " secret ".into(),
        }
        .validated_for(ProviderKind::BaiduTranslate)
        .unwrap();
        assert_eq!(baidu.baidu_translate(), Some(("app_123", "secret")));

        assert!(matches!(
            ProviderCredentials::TencentCloud {
                app_id: "123".into(),
                secret_id: "".into(),
                secret_key: "secret".into(),
            }
            .validated_for(ProviderKind::TencentCloud),
            Err(ProviderCredentialsError::Missing(
                ProviderKind::TencentCloud
            ))
        ));
    }
    #[test]
    fn anonymous_openai_compatible_keys_are_isolated_from_required_recognition_credentials() {
        let credentials = ProviderCredentials::OpenAICompatible {
            asr_api_key: "synthetic-asr".into(),
            endpoint: "http://127.0.0.1:8000/v1".into(),
            api_key: String::new(),
            model: "synthetic-model".into(),
        }
        .validated_for(ProviderKind::AlibabaCloud)
        .unwrap();
        assert_eq!(credentials.alibaba_key(), Some("synthetic-asr"));
        assert!(
            matches!(credentials, ProviderCredentials::OpenAICompatible { api_key, .. } if api_key.is_empty())
        );
        for key in [
            "\n".into(),
            "bad\r\nkey".into(),
            "x".repeat(MAXIMUM_CREDENTIAL_FIELD_LENGTH + 1),
        ] {
            assert_eq!(
                TextTranslationCredentials::OpenAICompatible {
                    endpoint: "http://127.0.0.1:8000/v1".into(),
                    model: "synthetic-model".into(),
                    api_key: key,
                }
                .validated(),
                Err(ProviderCredentialsError::InvalidField)
            );
        }
        assert_eq!(
            ProviderCredentials::DeepL {
                asr_api_key: "asr".into(),
                api_key: String::new()
            }
            .validated_for(ProviderKind::AlibabaCloud),
            Err(ProviderCredentialsError::Missing(
                ProviderKind::AlibabaCloud
            ))
        );
    }
    #[test]
    fn chatmock_credentials_keep_route_identity_and_scoped_optional_authentication() {
        let text = TextTranslationCredentials::ChatMock {
            endpoint: "http://127.0.0.1:8000/v1".into(),
            model: "synthetic-model".into(),
            api_key: "".into(),
        }
        .validated()
        .unwrap();
        assert_eq!(text.translation(), TextTranslation::ChatMock);
        let request: ProviderCredentials = serde_json::from_value(serde_json::json!({
            "kind":"alibabaTranslation", "apiKey":"", "textTranslation":"chatMock", "endpoint":"", "token":"", "model":"", "clearToken":true
        })).unwrap();
        assert!(matches!(
            request,
            ProviderCredentials::AlibabaTranslation {
                text_translation: TextTranslation::ChatMock,
                clear_token: true,
                ..
            }
        ));
        let credentials = ProviderCredentials::ChatMock {
            asr_api_key: "synthetic-asr".into(),
            endpoint: "http://127.0.0.1:8000/v1".into(),
            api_key: "synthetic-chatmock".into(),
            model: "synthetic-model".into(),
        }
        .validated_for(ProviderKind::AlibabaCloud)
        .unwrap();
        assert_eq!(credentials.alibaba_key(), Some("synthetic-asr"));
        assert_eq!(credentials.direct_api_key(), None);
        assert_eq!(
            serde_json::to_value(&credentials).unwrap()["kind"],
            "chatMock"
        );
        assert!(!format!("{credentials:?}").contains("synthetic"));
        let mut profile = ServiceProfile::alibaba_default();
        profile.text_translation = Some(TextTranslation::ChatMock);
        assert_eq!(
            credentials.revealed_field(
                &profile,
                CredentialRevealField::Token,
                Some(TextTranslation::ChatMock)
            ),
            Ok(Some("synthetic-chatmock"))
        );
        assert!(credentials
            .revealed_field(
                &profile,
                CredentialRevealField::Token,
                Some(TextTranslation::OpenAICompatible)
            )
            .is_err());
        profile.text_translation = Some(TextTranslation::OpenAICompatible);
        assert!(credentials
            .revealed_field(
                &profile,
                CredentialRevealField::Token,
                Some(TextTranslation::OpenAICompatible)
            )
            .is_err());
        for (endpoint, model, key) in [
            ("http://remote.example/v1", "model", ""),
            ("http://127.0.0.1:8000/v1", "", ""),
            ("http://127.0.0.1:8000/v1", "model", "bad\nkey"),
        ] {
            assert!(TextTranslationCredentials::ChatMock {
                endpoint: endpoint.into(),
                model: model.into(),
                api_key: key.into()
            }
            .validated()
            .is_err());
        }
    }
}
