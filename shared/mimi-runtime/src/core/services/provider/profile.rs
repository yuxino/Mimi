//! Service profile metadata, validation, patches, and route resolution.

use super::capabilities::custom_speech_capabilities;
use super::{
    alibaba_workspace_id_is_valid, default_xai_text_model, realtime_asr_capabilities,
    xai_asr_capabilities, AlibabaRealtimeMode, ProviderCapabilities, ProviderKind,
    ProviderPreferences, SpeechTranslationMode, TextTranslation, XAISpeechMode,
    DEFAULT_ALIBABA_PROFILE_ID, DEFAULT_XAI_TEXT_MODEL,
};
use crate::core::models::{SourceLanguage, TargetLanguage, TranslationMode};
use crate::core::services::network_proxy::{ProxyConfig, ProxyConfigError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

/// Optional languages restored on profile activation, independent of temporary
/// session language changes. Native resource readiness is checked by adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileLanguagePreset {
    pub source_language: SourceLanguage,
    pub target_language: TargetLanguage,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileLanguagePresetPatch {
    #[serde(deserialize_with = "deserialize_language_preset")]
    pub preset: Option<ProfileLanguagePreset>,
}

fn deserialize_language_preset<'de, D>(
    deserializer: D,
) -> Result<Option<ProfileLanguagePreset>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<ProfileLanguagePreset>::deserialize(deserializer)
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ServiceProfileError {
    #[error("This service does not support the selected speech translation mode.")]
    UnsupportedSpeechTranslationMode,
    #[error("xAI speech options require an xAI configuration.")]
    UnsupportedXAISpeech,
    #[error("Enter a valid xAI text model name (up to 256 bytes).")]
    InvalidXAITextModel,
    #[error("{0}")]
    NetworkProxy(#[from] ProxyConfigError),
    #[error("The selected speech service does not support separate text translation.")]
    UnsupportedTextTranslation,
    #[error("The service profile ID is invalid.")]
    InvalidID,
    #[error("The service profile name is required.")]
    EmptyName,
    #[error("The service profile name is too long.")]
    NameTooLong,
    #[error("The text translation service name is invalid.")]
    InvalidTextTranslationName,
    #[error("Only Alibaba Cloud profiles support experimental realtime modes.")]
    UnsupportedAlibabaRealtime,
    #[error("The Alibaba workspace ID is invalid.")]
    InvalidAlibabaWorkspaceId,
    #[error("custom_speech_languages_invalid")]
    InvalidCustomSpeechLanguages,
    #[error("Only custom speech services support a recognition display name.")]
    UnsupportedSpeechRecognitionName,
    #[error("The speech recognition service name is invalid.")]
    InvalidSpeechRecognitionName,
}

/// A patch for one route's optional display name; an empty name removes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextTranslationName {
    pub route: TextTranslation,
    pub name: String,
}

/// Optional IPC patch: an absent patch preserves metadata; `languages: null`
/// clears a declaration, while an empty array deliberately keeps only default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomSpeechLanguagesPatch {
    #[serde(deserialize_with = "deserialize_declared_languages")]
    pub languages: Option<Vec<SourceLanguage>>,
}

fn deserialize_declared_languages<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<SourceLanguage>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<Vec<SourceLanguage>>::deserialize(deserializer)
}

/// Non-secret metadata for one named provider configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceProfile {
    pub id: String,
    pub name: String,
    pub provider: ProviderKind,
    /// Built-in Alibaba text translation only; historical profiles retain Lite.
    #[serde(default)]
    pub qwen_mt_model: crate::core::protocols::translation::qwen_mt::QwenMTModel,
    #[serde(default)]
    pub alibaba_realtime_mode: AlibabaRealtimeMode,
    /// Optional non-secret workspace identifier, never an API key.
    #[serde(default)]
    pub alibaba_workspace_id: String,
    #[serde(default)]
    pub xai_speech_mode: XAISpeechMode,
    #[serde(default)]
    pub speech_translation_mode: SpeechTranslationMode,
    #[serde(default = "default_xai_text_model")]
    pub xai_text_model: String,
    /// A completed same-profile upgrade must never resurrect a deleted text key.
    #[serde(default)]
    pub xai_independent_translation_migrated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language_preset: Option<ProfileLanguagePreset>,
    /// User-declared explicit ASR languages, not server discovery. None means
    /// unknown; an empty list leaves only the protocol's default behavior.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_speech_source_languages: Option<Vec<SourceLanguage>>,
    /// Optional non-secret display name for custom speech recognition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speech_recognition_name: Option<String>,
    /// None preserves historical behavior, including legacy DeepLX profiles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_translation: Option<TextTranslation>,
    /// Independent routes retain their own non-secret names across switches.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub text_translation_names: BTreeMap<TextTranslation, String>,
    /// Absent fields inherit the pre-existing global route.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speech_network_proxy: Option<ProxyConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_network_proxy: Option<ProxyConfig>,
}

impl ServiceProfile {
    pub const MAXIMUM_ID_LENGTH: usize = 64;
    pub const MAXIMUM_NAME_LENGTH: usize = 64;

    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        provider: ProviderKind,
    ) -> Result<Self, ServiceProfileError> {
        let id = id.into();
        let name = name.into().trim().to_string();
        if id.is_empty()
            || id.len() > Self::MAXIMUM_ID_LENGTH
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(ServiceProfileError::InvalidID);
        }
        if name.is_empty() {
            return Err(ServiceProfileError::EmptyName);
        }
        if name.chars().count() > Self::MAXIMUM_NAME_LENGTH {
            return Err(ServiceProfileError::NameTooLong);
        }
        Ok(Self {
            id,
            name,
            provider,
            qwen_mt_model: Default::default(),
            alibaba_realtime_mode: Default::default(),
            alibaba_workspace_id: String::new(),
            xai_speech_mode: if provider == ProviderKind::XAIRealtime {
                XAISpeechMode::AsrMt
            } else {
                XAISpeechMode::Voice
            },
            speech_translation_mode: SpeechTranslationMode::Simultaneous,
            xai_text_model: default_xai_text_model(),
            xai_independent_translation_migrated: provider == ProviderKind::XAIRealtime,
            language_preset: None,
            custom_speech_source_languages: None,
            speech_recognition_name: None,
            text_translation: (provider == ProviderKind::XAIRealtime)
                .then_some(TextTranslation::XAI),
            text_translation_names: BTreeMap::new(),
            speech_network_proxy: None,
            text_network_proxy: None,
        })
    }

    pub fn alibaba_default() -> Self {
        Self {
            id: DEFAULT_ALIBABA_PROFILE_ID.to_string(),
            name: ProviderKind::AlibabaCloud.display_name().to_string(),
            provider: ProviderKind::AlibabaCloud,
            qwen_mt_model: Default::default(),
            alibaba_realtime_mode: Default::default(),
            alibaba_workspace_id: String::new(),
            xai_speech_mode: XAISpeechMode::Voice,
            speech_translation_mode: SpeechTranslationMode::Simultaneous,
            xai_text_model: default_xai_text_model(),
            xai_independent_translation_migrated: false,
            language_preset: None,
            custom_speech_source_languages: None,
            speech_recognition_name: None,
            text_translation: None,
            text_translation_names: BTreeMap::new(),
            speech_network_proxy: None,
            text_network_proxy: None,
        }
    }

    pub fn validated(&self) -> Result<Self, ServiceProfileError> {
        let mut profile = Self::new(self.id.clone(), self.name.clone(), self.provider)?;
        // A later route edit can make a saved pair unavailable. Preserve it and
        // reject activation, rather than rejecting the whole profile catalog.
        profile.language_preset = self.language_preset;
        profile.qwen_mt_model = self.qwen_mt_model;
        profile.set_alibaba_realtime(self.alibaba_realtime_mode, &self.alibaba_workspace_id)?;
        profile.set_xai_speech(self.xai_speech_mode, &self.xai_text_model)?;
        profile.set_speech_translation_mode(self.speech_translation_mode)?;
        profile.xai_independent_translation_migrated = self.xai_independent_translation_migrated;
        if matches!(
            self.text_translation,
            Some(
                TextTranslation::DeepLX
                    | TextTranslation::XAI
                    | TextTranslation::DeepL
                    | TextTranslation::OpenAICompatible
                    | TextTranslation::ChatMock
                    | TextTranslation::Apple
            )
        ) && !self.supports_text_translation_configuration()
        {
            return Err(ServiceProfileError::UnsupportedTextTranslation);
        }
        if let Some(name) = &self.speech_recognition_name {
            profile.set_speech_recognition_name(name)?;
        }
        profile.text_translation = self.text_translation;
        if self.custom_speech_source_languages.is_some() {
            profile
                .set_custom_speech_source_languages(self.custom_speech_source_languages.clone())?;
        }
        for (&route, name) in &self.text_translation_names {
            profile.set_text_translation_name(route, name)?;
        }
        profile.speech_network_proxy = self
            .speech_network_proxy
            .as_ref()
            .map(ProxyConfig::validate)
            .transpose()?;
        profile.text_network_proxy = self
            .text_network_proxy
            .as_ref()
            .map(ProxyConfig::validate)
            .transpose()?;
        Ok(profile)
    }

    pub fn set_xai_speech(
        &mut self,
        mode: XAISpeechMode,
        model: &str,
    ) -> Result<(), ServiceProfileError> {
        if self.provider != ProviderKind::XAIRealtime
            && (mode != XAISpeechMode::Voice || model != DEFAULT_XAI_TEXT_MODEL)
        {
            return Err(ServiceProfileError::UnsupportedXAISpeech);
        }
        let model = crate::core::protocols::translation::openai_compatible::validate_model(model)
            .map_err(|_| ServiceProfileError::InvalidXAITextModel)?;
        self.xai_speech_mode = mode;
        self.xai_text_model = model;
        Ok(())
    }

    pub fn set_speech_translation_mode(
        &mut self,
        mode: SpeechTranslationMode,
    ) -> Result<(), ServiceProfileError> {
        if mode != SpeechTranslationMode::Simultaneous
            && !self.provider.supports_speech_translation_mode()
        {
            return Err(ServiceProfileError::UnsupportedSpeechTranslationMode);
        }
        self.speech_translation_mode = mode;
        Ok(())
    }

    pub fn uses_realtime_asr(&self) -> bool {
        self.provider.supports_speech_translation_mode()
            && self.speech_translation_mode == SpeechTranslationMode::AsrMt
    }

    pub fn uses_independent_asr(&self) -> bool {
        self.uses_xai_asr() || self.uses_realtime_asr()
    }

    pub fn uses_xai_asr(&self) -> bool {
        self.provider == ProviderKind::XAIRealtime && self.xai_speech_mode == XAISpeechMode::AsrMt
    }

    pub fn set_alibaba_realtime(
        &mut self,
        mode: AlibabaRealtimeMode,
        workspace_id: &str,
    ) -> Result<(), ServiceProfileError> {
        let workspace_id = workspace_id.trim();
        if self.provider != ProviderKind::AlibabaCloud
            && (mode != AlibabaRealtimeMode::Disabled || !workspace_id.is_empty())
        {
            return Err(ServiceProfileError::UnsupportedAlibabaRealtime);
        }
        // Profiles may be saved before setup is complete. Session configuration
        // still requires a valid workspace before capture or provider access.
        if !workspace_id.is_empty() && !alibaba_workspace_id_is_valid(workspace_id) {
            return Err(ServiceProfileError::InvalidAlibabaWorkspaceId);
        }
        self.alibaba_realtime_mode = mode;
        self.alibaba_workspace_id = workspace_id.to_string();
        Ok(())
    }

    pub fn set_custom_speech_source_languages(
        &mut self,
        languages: Option<Vec<SourceLanguage>>,
    ) -> Result<(), ServiceProfileError> {
        if !self.provider.is_custom_speech()
            || languages.as_ref().is_some_and(|languages| {
                languages.len() >= SourceLanguage::ALL.len()
                    || languages.contains(&SourceLanguage::Automatic)
            })
        {
            return Err(ServiceProfileError::InvalidCustomSpeechLanguages);
        }
        self.custom_speech_source_languages = languages.map(|languages| {
            SourceLanguage::ALL
                .into_iter()
                .filter(|language| languages.contains(language))
                .collect()
        });
        Ok(())
    }

    pub fn set_speech_recognition_name(&mut self, name: &str) -> Result<(), ServiceProfileError> {
        if !self.provider.is_custom_speech() {
            return Err(ServiceProfileError::UnsupportedSpeechRecognitionName);
        }
        let name = name.trim();
        if name.chars().count() > Self::MAXIMUM_NAME_LENGTH || name.chars().any(char::is_control) {
            return Err(ServiceProfileError::InvalidSpeechRecognitionName);
        }
        self.speech_recognition_name = (!name.is_empty()).then(|| name.to_string());
        Ok(())
    }

    pub fn set_text_translation_name(
        &mut self,
        route: TextTranslation,
        name: &str,
    ) -> Result<(), ServiceProfileError> {
        if route == TextTranslation::FollowService
            || !self.supports_text_translation_configuration()
        {
            return Err(ServiceProfileError::UnsupportedTextTranslation);
        }
        let name = name.trim();
        if name.chars().count() > Self::MAXIMUM_NAME_LENGTH || name.chars().any(char::is_control) {
            return Err(ServiceProfileError::InvalidTextTranslationName);
        }
        if name.is_empty() {
            self.text_translation_names.remove(&route);
        } else {
            self.text_translation_names.insert(route, name.to_string());
        }
        Ok(())
    }

    pub fn text_translation(&self) -> TextTranslation {
        if self.provider.supports_speech_translation_mode() && !self.uses_realtime_asr() {
            return TextTranslation::FollowService;
        }
        if self.provider == ProviderKind::XAIRealtime && !self.uses_xai_asr() {
            return TextTranslation::FollowService;
        }
        if self.provider == ProviderKind::AlibabaCloud
            && self.alibaba_realtime_mode != AlibabaRealtimeMode::Disabled
        {
            return TextTranslation::FollowService;
        }
        self.text_translation.unwrap_or(if self.uses_xai_asr() {
            TextTranslation::XAI
        } else if self.uses_realtime_asr() {
            TextTranslation::OpenAICompatible
        } else if self.provider == ProviderKind::DeepLX {
            TextTranslation::DeepLX
        } else {
            TextTranslation::FollowService
        })
    }

    pub fn capabilities(&self, target: TargetLanguage) -> ProviderCapabilities {
        if self.uses_realtime_asr() {
            return realtime_asr_capabilities(self.provider, self.text_translation(), target);
        }
        if self.uses_xai_asr() {
            return xai_asr_capabilities(self.text_translation(), target);
        }
        if self.provider == ProviderKind::AlibabaCloud
            && self.alibaba_realtime_mode != AlibabaRealtimeMode::Disabled
        {
            return self.alibaba_realtime_mode.capabilities();
        }
        if self.provider.is_standalone_asr() {
            let mut capabilities =
                custom_speech_capabilities(self.provider, self.text_translation(), target);
            if let Some(declared) = &self.custom_speech_source_languages {
                capabilities.source_languages.retain(|language| {
                    *language == SourceLanguage::Automatic || declared.contains(language)
                });
            }
            capabilities
        } else {
            self.effective_provider().capabilities_for_route_with_model(
                self.text_translation(),
                target,
                self.qwen_mt_model,
            )
        }
    }

    pub fn validate_language_preset(&self, preset: ProfileLanguagePreset) -> Result<(), String> {
        let normalized = self.normalize_preferences(ProviderPreferences {
            source_language: preset.source_language,
            target_language: preset.target_language,
            translation_mode: TranslationMode::Turbo,
        });
        let capabilities = self
            .capabilities(preset.target_language)
            .for_source(preset.source_language);
        if !capabilities
            .source_languages
            .contains(&preset.source_language)
            || !capabilities
                .target_languages
                .contains(&preset.target_language)
            || normalized.source_language != preset.source_language
            || normalized.target_language != preset.target_language
        {
            return Err("profile_language_preset_unsupported".into());
        }
        Ok(())
    }

    /// Normalize the target first, then resolve its source catalog. Switching
    /// away from Original must not keep an Audio3-only hint on the Lite route.
    pub fn normalize_preferences(&self, prefs: ProviderPreferences) -> ProviderPreferences {
        let normalized = self.capabilities(prefs.target_language).normalize(prefs);
        self.capabilities(normalized.target_language)
            .normalize(normalized)
    }

    /// An explicit target choice must survive normalization unchanged. This
    /// also rejects unsupported source-dependent directions before saving.
    pub fn preferences_after_target_switch(
        &self,
        prefs: ProviderPreferences,
        target: TargetLanguage,
    ) -> Option<ProviderPreferences> {
        if !self
            .capabilities(target)
            .for_source(prefs.source_language)
            .target_languages
            .contains(&target)
        {
            return None;
        }
        let selection = self.normalize_preferences(ProviderPreferences {
            target_language: target,
            ..prefs
        });
        (selection.target_language == target).then_some(selection)
    }

    pub fn effective_provider(&self) -> ProviderKind {
        match (self.provider, self.text_translation()) {
            (ProviderKind::AlibabaCloud | ProviderKind::DeepLX, TextTranslation::DeepLX) => {
                ProviderKind::DeepLX
            }
            (
                ProviderKind::DeepLX,
                TextTranslation::FollowService
                | TextTranslation::XAI
                | TextTranslation::DeepL
                | TextTranslation::OpenAICompatible
                | TextTranslation::ChatMock
                | TextTranslation::Apple,
            ) => ProviderKind::AlibabaCloud,
            _ => self.provider,
        }
    }

    pub fn is_standalone_asr(&self) -> bool {
        self.provider.is_standalone_asr() || self.uses_independent_asr()
    }

    /// Stored destinations survive integrated-mode switches. This does not
    /// turn Grok Voice into a recognizer with an independent text phase.
    pub fn supports_text_translation_configuration(&self) -> bool {
        self.provider.supports_text_translation()
            || self.provider == ProviderKind::XAIRealtime
            || self.provider.supports_speech_translation_mode()
    }
}

impl Default for ServiceProfile {
    fn default() -> Self {
        Self::alibaba_default()
    }
}
