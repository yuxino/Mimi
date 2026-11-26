//! Credential slot storage model, readback-verified writes, and bounded lookup cache.

mod editor;
mod mutations;
mod state;

pub use editor::CredentialEditorState;

use super::{SecretCacheKey, SecretStoreError, SettingsStore, CREDENTIAL_STORE_UNAVAILABLE};
use crate::core::services::credentials::{ProviderCredentials, TextTranslationCredentials};
use crate::core::services::provider::{
    ProviderKind, ServiceProfile, TextTranslation, DEFAULT_ALIBABA_PROFILE_ID,
};
use serde::{Deserialize, Serialize};

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

    #[cfg(test)]
    pub(super) fn load_api_key(&self) -> Result<Option<String>, String> {
        let profile = self.active_profile()?;
        self.load_api_key_for_profile(&profile)
            .map_err(SecretStoreError::public_error)
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
        let text_translation = profile.text_translation();
        if matches!(
            profile.provider,
            ProviderKind::AlibabaCloud | ProviderKind::DeepLX
        ) && !matches!(
            text_translation,
            TextTranslation::FollowService | TextTranslation::Apple | TextTranslation::XAI
        ) {
            let Some(destination) = self.destination_value(profile)? else {
                if profile.provider == ProviderKind::DeepLX
                    && matches!(
                        (text_translation, &credentials),
                        (TextTranslation::DeepLX, ProviderCredentials::DeepLX { .. })
                            | (TextTranslation::DeepL, ProviderCredentials::DeepL { .. })
                    )
                {
                    return Ok(Some(credentials));
                }
                return Err(
                    crate::core::services::credentials::ProviderCredentialsError::InvalidStoredValue
                        .to_string(),
                );
            };
            let destination: TextTranslationDestination = serde_json::from_str(&destination)
                .map_err(|_| {
                    crate::core::services::credentials::ProviderCredentialsError::InvalidStoredValue
                        .to_string()
                })?;
            let asr_api_key = credentials.alibaba_key().unwrap_or_default().into();
            let credentials = match text_translation {
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
                TextTranslation::FollowService | TextTranslation::Apple | TextTranslation::XAI => {
                    unreachable!()
                }
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
                crate::core::services::credentials::ProviderCredentialsError::InvalidStoredValue
                    .to_string()
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
                crate::core::services::credentials::ProviderCredentialsError::ProviderMismatch
                    .to_string(),
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
