//! Credential save/delete transactions and rollback handling.

use super::TextTranslationDestination;
use crate::core::services::credentials::{ProviderCredentials, TextTranslationCredentials};
use crate::core::services::provider::{ProviderKind, ServiceProfile, TextTranslation};
use crate::settings_store::preferences::normalize_preferences_value;
use crate::settings_store::{
    SecretStoreError, SettingsStore, CREDENTIAL_STORE_UNAVAILABLE, PROFILE_CATALOG_UNAVAILABLE,
    PROFILE_NOT_FOUND,
};

impl SettingsStore {
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
                    crate::core::services::credentials::ProviderCredentialsError::InvalidField
                        .to_string(),
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

    fn save_custom_speech(
        &self,
        profile: &ServiceProfile,
        endpoint: &str,
        model: &str,
        api_key: &str,
    ) -> Result<(), String> {
        use crate::core::services::credentials::ProviderCredentialsError as Error;
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
                crate::core::protocols::recognition::custom_speech::endpoint(
                    endpoint,
                    profile.provider,
                )
                .map_err(|_| Error::InvalidCustomSpeechEndpoint.to_string())?
                .to_string(),
            )
        };
        let entered_model = if model.trim().is_empty() {
            None
        } else {
            Some(
                crate::core::protocols::recognition::custom_speech::validate_model(model)
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

    fn save_custom_text_translation(
        &self,
        profile: &ServiceProfile,
        translation: TextTranslation,
        endpoint: &str,
        token_update: Option<&str>,
        model: &str,
    ) -> Result<(), String> {
        use crate::core::services::credentials::ProviderCredentialsError as Error;
        if self.catalog_write_blocked {
            return Err(PROFILE_CATALOG_UNAVAILABLE.into());
        }
        let entered_endpoint = if endpoint.trim().is_empty() {
            None
        } else if translation.uses_chat_completions() {
            Some(
                crate::core::protocols::translation::openai_compatible::endpoint(endpoint)
                    .map_err(|_| Error::InvalidOpenAICompatibleEndpoint.to_string())?
                    .to_string(),
            )
        } else if translation == TextTranslation::DeepLX {
            Some(
                crate::core::protocols::translation::deeplx::endpoint(endpoint)
                    .map_err(|_| Error::InvalidDeepLXEndpoint.to_string())?
                    .to_string(),
            )
        } else {
            None
        };
        let entered_model = if translation.uses_chat_completions() && !model.trim().is_empty() {
            Some(
                crate::core::protocols::translation::openai_compatible::validate_model(model)
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
        use crate::core::services::credentials::ProviderCredentialsError as Error;
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
                    crate::core::services::credentials::ProviderCredentialsError::InvalidField
                        .to_string(),
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
            crate::core::protocols::translation::deeplx::endpoint(endpoint).map_err(|_| {
                crate::core::services::credentials::ProviderCredentialsError::InvalidDeepLXEndpoint
                    .to_string()
            })?;
        }
        if translation.uses_chat_completions() {
            if !endpoint.trim().is_empty() {
                crate::core::protocols::translation::openai_compatible::endpoint(endpoint).map_err(|_| {
                    crate::core::services::credentials::ProviderCredentialsError::InvalidOpenAICompatibleEndpoint
                        .to_string()
                })?;
            }
            if !model.trim().is_empty() {
                crate::core::protocols::translation::openai_compatible::validate_model(model).map_err(|_| {
                    crate::core::services::credentials::ProviderCredentialsError::InvalidOpenAICompatibleModel
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
                crate::core::services::credentials::ProviderCredentialsError::InvalidStoredValue
                    .to_string()
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

    pub(in crate::settings_store) fn delete_profile_credentials(
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
}
