//! Assemble listening and independent speech/text probe configurations.

use super::credentials::TextTranslationDestination;
use super::preferences::{
    normalize_preferences_value, profile_selection_preferences, PreferenceWrite,
};
use super::{Preferences, SecretStoreError, SettingsStore};
use crate::core::models::{SourceLanguage, TargetLanguage, TranslationMode};
use crate::core::services::configuration::LiveTranslationConfiguration;
use crate::core::services::credentials::{ProviderCredentials, TextTranslationCredentials};
use crate::core::services::network_proxy::ProxyConfig;
use crate::core::services::provider::{
    ProviderKind, ProviderPreferences, ServiceProfile, TextTranslation,
};

impl SettingsStore {
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
        if (configuration.is_standalone_asr()
            || matches!(
                profile.text_translation(),
                TextTranslation::Apple | TextTranslation::XAI
            ))
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
        use crate::core::services::credentials::ProviderCredentialsError as Error;

        if !speech_only
            && (profile.provider.supports_text_translation() || profile.uses_independent_asr())
        {
            return Err("draft_connection_check_stage_required".into());
        }
        let provider = if (profile.provider.supports_text_translation()
            || profile.uses_independent_asr())
            && !profile.is_standalone_asr()
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
        let credentials =
            match draft.validated_for_speech_mode(provider, profile.speech_translation_mode) {
                Ok(complete) => complete,
                Err(_) => {
                    self.retry_profile_credential_errors(profile, true, false);
                    let saved = self
                        .load_api_key_for_profile(profile)
                        .map_err(SecretStoreError::public_error)?
                        .map(|value| Self::decode_speech_editor_credentials(profile, &value))
                        .transpose()
                        .map_err(|error| error.to_string())?;
                    if profile.provider.supports_speech_translation_mode() {
                        draft.resolve_draft_for_profile(profile, saved.as_ref())
                    } else {
                        draft.resolve_draft(provider, saved.as_ref())
                    }
                    .map_err(|error| error.to_string())?
                }
            };
        let prefs = self.preferences();
        let target_language = if speech_only
            && (profile.provider.supports_text_translation() || profile.uses_independent_asr())
        {
            TargetLanguage::Original
        } else {
            prefs.target_language
        };
        let capabilities = if provider.is_custom_speech()
            || profile.uses_independent_asr()
            || profile.alibaba_realtime_mode
                != crate::core::services::provider::AlibabaRealtimeMode::Disabled
        {
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
        .with_alibaba_realtime(profile.alibaba_realtime_mode, &profile.alibaba_workspace_id)
        .with_xai_speech(profile.xai_speech_mode, &profile.xai_text_model)
        .with_speech_translation_mode(profile.speech_translation_mode)
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
    ) -> Result<crate::core::services::configuration::TextTranslationProbeConfiguration, String>
    {
        use crate::core::services::configuration::TextTranslationProbeCredentials;
        use crate::core::services::credentials::ProviderCredentialsError as Error;

        if !(profile.provider.supports_text_translation() || profile.uses_independent_asr()) {
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
            if profile.is_standalone_asr() {
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
                    crate::core::protocols::translation::openai_compatible::endpoint(endpoint)
                        .map_err(|_| Error::InvalidOpenAICompatibleEndpoint.to_string())?;
                } else if route == TextTranslation::DeepLX {
                    crate::core::protocols::translation::deeplx::endpoint(endpoint)
                        .map_err(|_| Error::InvalidDeepLXEndpoint.to_string())?;
                }
            }
            if route.uses_chat_completions() && !model.trim().is_empty() {
                crate::core::protocols::translation::openai_compatible::validate_model(model)
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
        if profile.uses_independent_asr() {
            let credentials = self
                .credentials_for_profile(profile)?
                .ok_or("custom_speech_credentials_missing")?;
            return self.configuration_for_speech_draft_probe(profile, &credentials, true);
        }
        if !(profile.provider.supports_text_translation() || profile.uses_independent_asr()) {
            return self.configuration_for_profile_probe(profile);
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
        let capabilities = if provider.is_custom_speech()
            || profile.uses_independent_asr()
            || profile.alibaba_realtime_mode
                != crate::core::services::provider::AlibabaRealtimeMode::Disabled
        {
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
        .with_alibaba_realtime(profile.alibaba_realtime_mode, &profile.alibaba_workspace_id)
        .with_xai_speech(profile.xai_speech_mode, &profile.xai_text_model)
        .with_speech_translation_mode(profile.speech_translation_mode)
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
    ) -> Result<crate::core::services::configuration::TextTranslationProbeConfiguration, String>
    {
        use crate::core::services::configuration::TextTranslationProbeCredentials;
        if !(profile.provider.supports_text_translation() || profile.uses_independent_asr())
            || (profile.is_standalone_asr()
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
                None if profile.provider == ProviderKind::DeepLX => {
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

    pub(super) fn text_probe_with_credentials(
        &self,
        profile: &ServiceProfile,
        credentials: crate::core::services::configuration::TextTranslationProbeCredentials,
    ) -> Result<crate::core::services::configuration::TextTranslationProbeConfiguration, String>
    {
        use crate::core::services::configuration::TextTranslationProbeConfiguration;
        let prefs = self.preferences();
        let native = matches!(
            credentials,
            crate::core::services::configuration::TextTranslationProbeCredentials::Independent(
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
                    mode: crate::core::services::network_proxy::ProxyMode::Direct,
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

    pub(super) fn configuration_for_profile_options(
        &self,
        profile: &ServiceProfile,
        for_probe: bool,
    ) -> Result<LiveTranslationConfiguration, String> {
        let mut prefs = self.prefs.lock().unwrap().clone();
        if for_probe {
            // Explicit checks verify a configured text destination even when
            // listening is recognition-only. This local choice never persists.
            if profile.is_standalone_asr()
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

    pub(super) fn configuration_with_profile_preferences(
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
            return Err(crate::core::services::configuration::LiveTranslationConfigurationError::UnsupportedSourceLanguage.to_string());
        }
        let credentials = self.credentials_for_profile(profile)?.ok_or_else(|| {
            crate::core::services::credentials::ProviderCredentialsError::Missing(
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
        .with_alibaba_realtime(profile.alibaba_realtime_mode, &profile.alibaba_workspace_id)
        .with_xai_speech(profile.xai_speech_mode, &profile.xai_text_model)
        .with_speech_translation_mode(profile.speech_translation_mode)
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
            || ((profile.is_standalone_asr() || profile.text_translation() == TextTranslation::XAI)
                && prefs.target_language.translates_audio())
        {
            let text_credentials =
                self.text_credentials_for_profile(profile)?.ok_or_else(|| {
                    crate::core::services::credentials::ProviderCredentialsError::MissingTextTranslation
                        .to_string()
                })?;
            configuration = configuration.with_text_credentials(text_credentials);
        }
        configuration.validated().map_err(|error| error.to_string())
    }

    /// Applies listening-time constraints without changing profile metadata.
    pub fn prepare_for_listening(&self) -> Result<(), String> {
        let profile = self.active_profile()?;
        self.save_preferences_validated(
            |prefs| normalize_preferences_value(prefs, &profile),
            |_, _| Ok(()),
            PreferenceWrite::IfDifferent,
        )
    }
}
