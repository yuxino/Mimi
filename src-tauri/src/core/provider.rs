//! Provider identity, service profiles, and capability normalization.
//!
//! Profiles contain display metadata only. Credentials deliberately live in
//! the OS keychain and must never be serialized with a profile.

use crate::core::models::{SourceLanguage, TargetLanguage, TranslationMode};
use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

pub const DEFAULT_ALIBABA_PROFILE_ID: &str = "alibaba-default";

/// A stable identifier used in settings JSON and provider-scoped keychain
/// account names. The serde values are part of the frontend contract.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderKind {
    #[default]
    #[serde(rename = "alibabaCloud")]
    AlibabaCloud,
    #[serde(rename = "openAIRealtime")]
    OpenAIRealtime,
    #[serde(rename = "googleGeminiLive")]
    GoogleGeminiLive,
    #[serde(rename = "azureOpenAIRealtime")]
    AzureOpenAIRealtime,
    #[serde(rename = "volcanoEngine")]
    VolcanoEngine,
    #[serde(rename = "tencentCloud")]
    TencentCloud,
    #[serde(rename = "baiduTranslate")]
    BaiduTranslate,
    #[serde(rename = "xAIRealtime")]
    XAIRealtime,
    #[serde(rename = "deepLX")]
    DeepLX,
}

impl ProviderKind {
    pub fn capabilities_for_route(
        self,
        route: TextTranslation,
        target: TargetLanguage,
    ) -> ProviderCapabilities {
        if self == Self::AlibabaCloud {
            alibaba_capabilities(route, target)
        } else {
            self.capabilities()
        }
    }

    pub const fn wire_value(self) -> &'static str {
        match self {
            Self::AlibabaCloud => "alibabaCloud",
            Self::OpenAIRealtime => "openAIRealtime",
            Self::GoogleGeminiLive => "googleGeminiLive",
            Self::AzureOpenAIRealtime => "azureOpenAIRealtime",
            Self::VolcanoEngine => "volcanoEngine",
            Self::TencentCloud => "tencentCloud",
            Self::BaiduTranslate => "baiduTranslate",
            Self::XAIRealtime => "xAIRealtime",
            Self::DeepLX => "deepLX",
        }
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::AlibabaCloud => "Alibaba Cloud",
            Self::OpenAIRealtime => "OpenAI Realtime",
            Self::GoogleGeminiLive => "Google Gemini",
            Self::AzureOpenAIRealtime => "Azure OpenAI",
            Self::VolcanoEngine => "Volcano Engine",
            Self::TencentCloud => "Tencent Cloud",
            Self::BaiduTranslate => "Baidu Translate",
            Self::XAIRealtime => "xAI Grok",
            Self::DeepLX => "DeepLX (Audio 3.0 ASR)",
        }
    }

    pub fn capabilities(self) -> ProviderCapabilities {
        match self {
            Self::AlibabaCloud => alibaba_capabilities(
                TextTranslation::FollowService,
                TargetLanguage::SimplifiedChinese,
            ),
            Self::OpenAIRealtime | Self::AzureOpenAIRealtime | Self::XAIRealtime => {
                realtime_capabilities(vec![SourceLanguage::Automatic], 24_000)
            }
            Self::DeepLX => realtime_capabilities(
                vec![
                    SourceLanguage::Automatic,
                    SourceLanguage::Chinese,
                    SourceLanguage::English,
                    SourceLanguage::Japanese,
                    SourceLanguage::Korean,
                ],
                16_000,
            ),
            Self::GoogleGeminiLive => {
                realtime_capabilities(vec![SourceLanguage::Automatic], 16_000)
            }
            Self::VolcanoEngine => realtime_capabilities(
                vec![
                    SourceLanguage::Japanese,
                    SourceLanguage::English,
                    SourceLanguage::Chinese,
                ],
                16_000,
            ),
            Self::TencentCloud | Self::BaiduTranslate => realtime_capabilities(
                vec![
                    SourceLanguage::Japanese,
                    SourceLanguage::English,
                    SourceLanguage::Korean,
                    SourceLanguage::Chinese,
                ],
                16_000,
            ),
        }
    }

    pub const fn uses_api_key_only(self) -> bool {
        matches!(
            self,
            Self::AlibabaCloud
                | Self::OpenAIRealtime
                | Self::GoogleGeminiLive
                | Self::VolcanoEngine
                | Self::XAIRealtime
        )
    }
}

/// Route-aware language capabilities. A representable code is not permission
/// to send it to a different service. Original broadens only the default
/// Audio3 route; independent text destinations retain their verified subset.
fn alibaba_capabilities(route: TextTranslation, target: TargetLanguage) -> ProviderCapabilities {
    if route == TextTranslation::DeepLX {
        return ProviderKind::DeepLX.capabilities();
    }
    if matches!(
        route,
        TextTranslation::DeepL | TextTranslation::OpenAICompatible
    ) {
        return ProviderCapabilities {
            source_languages: vec![
                SourceLanguage::Automatic,
                SourceLanguage::Chinese,
                SourceLanguage::English,
                SourceLanguage::Japanese,
                SourceLanguage::Korean,
            ],
            target_languages: vec![
                TargetLanguage::Original,
                TargetLanguage::SimplifiedChinese,
                TargetLanguage::English,
                TargetLanguage::Japanese,
            ],
            translation_modes: vec![TranslationMode::Turbo],
            input_sample_rate_hz: 16_000,
        };
    }
    let source_languages = SourceLanguage::ALL
        .into_iter()
        .filter(|source| {
            target == TargetLanguage::Original
                || *source == SourceLanguage::Automatic
                || crate::core::protocols::qwen_mt::REALTIME_MT_MODEL
                    .supported_language_codes()
                    .contains(&source.raw_value())
        })
        .collect();
    let mut target_languages = vec![TargetLanguage::Original];
    target_languages.extend(
        crate::core::protocols::qwen_mt::REALTIME_MT_MODEL
            .supported_language_codes()
            .iter()
            .filter_map(|code| {
                TargetLanguage::ALL
                    .into_iter()
                    .find(|target| target.raw_value() == *code)
            }),
    );
    ProviderCapabilities {
        source_languages,
        target_languages,
        translation_modes: vec![TranslationMode::Turbo],
        input_sample_rate_hz: 16_000,
    }
}

fn realtime_capabilities(
    source_languages: Vec<SourceLanguage>,
    input_sample_rate_hz: u32,
) -> ProviderCapabilities {
    ProviderCapabilities {
        source_languages,
        target_languages: vec![
            TargetLanguage::SimplifiedChinese,
            TargetLanguage::English,
            TargetLanguage::Japanese,
        ],
        translation_modes: vec![TranslationMode::Turbo],
        input_sample_rate_hz,
    }
}

impl fmt::Display for ProviderKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.display_name())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCapabilities {
    pub source_languages: Vec<SourceLanguage>,
    pub target_languages: Vec<TargetLanguage>,
    pub translation_modes: Vec<TranslationMode>,
    pub input_sample_rate_hz: u32,
}

impl ProviderCapabilities {
    /// Normalizes stale preferences after changing providers. The settings UI
    /// compares the before/after snapshots when it needs to explain a change.
    pub fn normalize(&self, preferences: ProviderPreferences) -> ProviderPreferences {
        let mut normalized = preferences;

        if !self.source_languages.contains(&normalized.source_language) {
            if let Some(fallback) = self
                .source_languages
                .iter()
                .copied()
                .find(|source| !source_matches_target(*source, normalized.target_language))
                .or_else(|| self.source_languages.first().copied())
            {
                normalized.source_language = fallback;
            }
        }
        if !self.target_languages.contains(&normalized.target_language) {
            if let Some(fallback) = self.target_languages.first().copied() {
                normalized.target_language = fallback;
            }
        }
        if !self
            .translation_modes
            .contains(&normalized.translation_mode)
        {
            if let Some(fallback) = self.translation_modes.first().copied() {
                normalized.translation_mode = fallback;
            }
        }

        // Dedicated translation services require different explicit source
        // and target languages. Avoid persisting a no-op pair after a source
        // switch or when loading stale preferences. Routes with a separate
        // Original-subtitle mode allow same-language selections and retain
        // the user's explicit target.
        if !self.target_languages.contains(&TargetLanguage::Original)
            && source_matches_target(normalized.source_language, normalized.target_language)
        {
            if let Some(fallback) = self
                .target_languages
                .iter()
                .copied()
                .find(|target| !source_matches_target(normalized.source_language, *target))
            {
                normalized.target_language = fallback;
            }
        }

        normalized
    }

    /// Source pickers retain the explicit target unless provider normalization
    /// requires a supported fallback or a different translation language.
    pub fn target_language_after_source_switch(
        &self,
        source_language: SourceLanguage,
        current_target: TargetLanguage,
    ) -> TargetLanguage {
        self.normalize(ProviderPreferences {
            source_language,
            target_language: current_target,
            translation_mode: self
                .translation_modes
                .first()
                .copied()
                .unwrap_or(TranslationMode::Turbo),
        })
        .target_language
    }
}

fn source_matches_target(source: SourceLanguage, target: TargetLanguage) -> bool {
    source != SourceLanguage::Automatic
        && target.translates_audio()
        && source.raw_value() == target.raw_value()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderPreferences {
    pub source_language: SourceLanguage,
    pub target_language: TargetLanguage,
    pub translation_mode: TranslationMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ServiceProfileError {
    #[error("Separate text translation requires Alibaba speech recognition.")]
    UnsupportedTextTranslation,
    #[error("The service profile ID is invalid.")]
    InvalidID,
    #[error("The service profile name is required.")]
    EmptyName,
    #[error("The service profile name is too long.")]
    NameTooLong,
}

/// Text translation override, supported only by the Alibaba Audio 3.0 chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextTranslation {
    #[serde(rename = "followService")]
    FollowService,
    #[serde(rename = "deepL")]
    DeepL,
    #[serde(rename = "deepLX")]
    DeepLX,
    #[serde(rename = "openAICompatible")]
    OpenAICompatible,
}

/// Non-secret metadata for one named provider configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceProfile {
    pub id: String,
    pub name: String,
    pub provider: ProviderKind,
    /// None preserves historical behavior, including legacy DeepLX profiles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_translation: Option<TextTranslation>,
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
            text_translation: None,
        })
    }

    pub fn alibaba_default() -> Self {
        Self {
            id: DEFAULT_ALIBABA_PROFILE_ID.to_string(),
            name: ProviderKind::AlibabaCloud.display_name().to_string(),
            provider: ProviderKind::AlibabaCloud,
            text_translation: None,
        }
    }

    pub fn validated(&self) -> Result<Self, ServiceProfileError> {
        let mut profile = Self::new(self.id.clone(), self.name.clone(), self.provider)?;
        if matches!(
            self.text_translation,
            Some(
                TextTranslation::DeepLX
                    | TextTranslation::DeepL
                    | TextTranslation::OpenAICompatible
            )
        ) && !matches!(
            self.provider,
            ProviderKind::AlibabaCloud | ProviderKind::DeepLX
        ) {
            return Err(ServiceProfileError::UnsupportedTextTranslation);
        }
        profile.text_translation = self.text_translation;
        Ok(profile)
    }

    pub fn text_translation(&self) -> TextTranslation {
        self.text_translation
            .unwrap_or(if self.provider == ProviderKind::DeepLX {
                TextTranslation::DeepLX
            } else {
                TextTranslation::FollowService
            })
    }

    pub fn capabilities(&self, target: TargetLanguage) -> ProviderCapabilities {
        if self.effective_provider() == ProviderKind::AlibabaCloud {
            alibaba_capabilities(self.text_translation(), target)
        } else {
            self.effective_provider().capabilities()
        }
    }

    /// Normalize the target first, then resolve its source catalog. Switching
    /// away from Original must not keep an Audio3-only hint on the Lite route.
    pub fn normalize_preferences(&self, prefs: ProviderPreferences) -> ProviderPreferences {
        let normalized = self.capabilities(prefs.target_language).normalize(prefs);
        self.capabilities(normalized.target_language)
            .normalize(normalized)
    }

    pub fn effective_provider(&self) -> ProviderKind {
        match (self.provider, self.text_translation()) {
            (ProviderKind::AlibabaCloud | ProviderKind::DeepLX, TextTranslation::DeepLX) => {
                ProviderKind::DeepLX
            }
            (
                ProviderKind::DeepLX,
                TextTranslation::FollowService
                | TextTranslation::DeepL
                | TextTranslation::OpenAICompatible,
            ) => ProviderKind::AlibabaCloud,
            _ => self.provider,
        }
    }
}

impl Default for ServiceProfile {
    fn default() -> Self {
        Self::alibaba_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alibaba_route_catalogs_match_audio3_lite_intersection_and_original() {
        let profile = ServiceProfile::alibaba_default();
        let translated = profile.capabilities(TargetLanguage::French);
        assert_eq!(
            translated
                .source_languages
                .iter()
                .map(|source| source.raw_value())
                .collect::<Vec<_>>(),
            [
                "auto", "zh", "en", "ja", "ko", "vi", "th", "id", "ms", "tl", "hi", "ar", "fr",
                "de", "es", "pt", "ru", "it", "nl", "sv", "da", "fi", "pl", "cs", "hu"
            ]
        );
        assert_eq!(translated.target_languages[0], TargetLanguage::Original);
        assert_eq!(
            translated.target_languages[1..]
                .iter()
                .map(|target| target.raw_value())
                .collect::<Vec<_>>(),
            crate::core::protocols::qwen_mt::QWEN_MT_LITE_LANGUAGE_CODES
        );
        assert_eq!(
            profile
                .capabilities(TargetLanguage::Original)
                .source_languages,
            SourceLanguage::ALL
        );
        let normalized = profile.normalize_preferences(ProviderPreferences {
            source_language: SourceLanguage::Greek,
            target_language: TargetLanguage::French,
            translation_mode: TranslationMode::Turbo,
        });
        assert_eq!(normalized.source_language, SourceLanguage::Automatic);
        assert_eq!(normalized.target_language, TargetLanguage::French);
    }

    #[test]
    fn independent_text_routes_and_other_providers_do_not_inherit_lite() {
        let mut profile = ServiceProfile::alibaba_default();
        profile.text_translation = Some(TextTranslation::DeepL);
        let deepl = profile.capabilities(TargetLanguage::Original);
        assert_eq!(deepl.source_languages.len(), 5);
        assert_eq!(deepl.target_languages.len(), 4);
        assert!(!deepl.source_languages.contains(&SourceLanguage::French));
        assert!(!deepl
            .target_languages
            .contains(&TargetLanguage::TraditionalChinese));
        profile.text_translation = Some(TextTranslation::DeepLX);
        assert_eq!(
            profile
                .capabilities(TargetLanguage::French)
                .target_languages
                .len(),
            3
        );
        for provider in [
            ProviderKind::OpenAIRealtime,
            ProviderKind::AzureOpenAIRealtime,
            ProviderKind::GoogleGeminiLive,
            ProviderKind::XAIRealtime,
            ProviderKind::VolcanoEngine,
            ProviderKind::TencentCloud,
            ProviderKind::BaiduTranslate,
            ProviderKind::DeepLX,
        ] {
            let caps = provider.capabilities();
            assert!(!caps.source_languages.contains(&SourceLanguage::French));
            assert!(!caps.target_languages.contains(&TargetLanguage::French));
        }
    }

    #[test]
    fn openai_compatible_route_stays_on_alibaba_and_has_bounded_language_catalog() {
        for provider in [ProviderKind::AlibabaCloud, ProviderKind::DeepLX] {
            let mut profile =
                ServiceProfile::new("custom", "Custom translation", provider).unwrap();
            profile.text_translation = Some(TextTranslation::OpenAICompatible);
            assert_eq!(
                profile.validated().unwrap().effective_provider(),
                ProviderKind::AlibabaCloud
            );
            let body = serde_json::to_value(&profile).unwrap();
            assert_eq!(body["textTranslation"], "openAICompatible");
            assert_eq!(
                serde_json::from_value::<ServiceProfile>(body).unwrap(),
                profile
            );
            let caps = profile.capabilities(TargetLanguage::Original);
            assert_eq!(
                caps.source_languages,
                vec![
                    SourceLanguage::Automatic,
                    SourceLanguage::Chinese,
                    SourceLanguage::English,
                    SourceLanguage::Japanese,
                    SourceLanguage::Korean
                ]
            );
            assert_eq!(
                caps.target_languages,
                vec![
                    TargetLanguage::Original,
                    TargetLanguage::SimplifiedChinese,
                    TargetLanguage::English,
                    TargetLanguage::Japanese
                ]
            );
            assert_eq!(caps.translation_modes, vec![TranslationMode::Turbo]);
        }
        let mut unsupported =
            ServiceProfile::new("openai", "OpenAI", ProviderKind::OpenAIRealtime).unwrap();
        unsupported.text_translation = Some(TextTranslation::OpenAICompatible);
        assert_eq!(
            unsupported.validated(),
            Err(ServiceProfileError::UnsupportedTextTranslation)
        );
    }

    use serde_json::json;

    #[test]
    fn historical_profiles_and_supported_text_routes_round_trip() {
        let legacy: ServiceProfile =
            serde_json::from_str(r#"{"id":"old","name":"Custom","provider":"deepLX"}"#).unwrap();
        assert_eq!(legacy.effective_provider(), ProviderKind::DeepLX);
        assert_eq!(legacy.validated().unwrap(), legacy);
        let mut ordinary = ServiceProfile::alibaba_default();
        assert_eq!(ordinary.text_translation(), TextTranslation::FollowService);
        ordinary.text_translation = Some(TextTranslation::DeepLX);
        let json = serde_json::to_string(&ordinary).unwrap();
        assert!(json.contains(r#""textTranslation":"deepLX""#));
        assert_eq!(
            serde_json::from_str::<ServiceProfile>(&json)
                .unwrap()
                .validated()
                .unwrap()
                .effective_provider(),
            ProviderKind::DeepLX
        );
        let mut unsupported =
            ServiceProfile::new("other", "Other", ProviderKind::OpenAIRealtime).unwrap();
        unsupported.text_translation = Some(TextTranslation::DeepLX);
        assert_eq!(
            unsupported.validated().unwrap_err(),
            ServiceProfileError::UnsupportedTextTranslation
        );
    }

    #[test]
    fn provider_wire_values_are_stable() {
        let cases = [
            (ProviderKind::AlibabaCloud, "alibabaCloud"),
            (ProviderKind::OpenAIRealtime, "openAIRealtime"),
            (ProviderKind::GoogleGeminiLive, "googleGeminiLive"),
            (ProviderKind::AzureOpenAIRealtime, "azureOpenAIRealtime"),
            (ProviderKind::VolcanoEngine, "volcanoEngine"),
            (ProviderKind::TencentCloud, "tencentCloud"),
            (ProviderKind::BaiduTranslate, "baiduTranslate"),
            (ProviderKind::XAIRealtime, "xAIRealtime"),
        ];
        for (provider, wire_value) in cases {
            assert_eq!(serde_json::to_value(provider).unwrap(), json!(wire_value));
            assert_eq!(
                serde_json::from_value::<ProviderKind>(json!(wire_value)).unwrap(),
                provider
            );
            assert_eq!(provider.wire_value(), wire_value);
        }
    }

    #[test]
    fn provider_capabilities_match_transport_constraints() {
        let alibaba = ProviderKind::AlibabaCloud.capabilities();
        assert_eq!(alibaba.source_languages.len(), 25);
        assert_eq!(alibaba.target_languages.len(), 32);
        assert_eq!(alibaba.translation_modes, vec![TranslationMode::Turbo]);
        assert_eq!(alibaba.input_sample_rate_hz, 16_000);

        let openai = ProviderKind::OpenAIRealtime.capabilities();
        assert_eq!(openai.source_languages, vec![SourceLanguage::Automatic]);
        assert_eq!(
            openai.target_languages,
            vec![
                TargetLanguage::SimplifiedChinese,
                TargetLanguage::English,
                TargetLanguage::Japanese
            ]
        );
        assert_eq!(openai.translation_modes, vec![TranslationMode::Turbo]);
        assert_eq!(openai.input_sample_rate_hz, 24_000);

        for provider in [
            ProviderKind::GoogleGeminiLive,
            ProviderKind::AzureOpenAIRealtime,
            ProviderKind::VolcanoEngine,
            ProviderKind::TencentCloud,
            ProviderKind::BaiduTranslate,
            ProviderKind::XAIRealtime,
        ] {
            let capabilities = provider.capabilities();
            assert_eq!(capabilities.target_languages.len(), 3);
            assert_eq!(capabilities.translation_modes, vec![TranslationMode::Turbo]);
        }
        assert_eq!(
            ProviderKind::GoogleGeminiLive
                .capabilities()
                .input_sample_rate_hz,
            16_000
        );
        assert_eq!(
            ProviderKind::AzureOpenAIRealtime
                .capabilities()
                .input_sample_rate_hz,
            24_000
        );
        assert_eq!(
            ProviderKind::VolcanoEngine.capabilities().source_languages,
            vec![
                SourceLanguage::Japanese,
                SourceLanguage::English,
                SourceLanguage::Chinese
            ]
        );
        for provider in [ProviderKind::TencentCloud, ProviderKind::BaiduTranslate] {
            let capabilities = provider.capabilities();
            assert_eq!(
                capabilities.source_languages,
                vec![
                    SourceLanguage::Japanese,
                    SourceLanguage::English,
                    SourceLanguage::Korean,
                    SourceLanguage::Chinese
                ]
            );
            assert!(!capabilities
                .source_languages
                .contains(&SourceLanguage::Automatic));
        }
    }

    #[test]
    fn openai_normalization_uses_supported_fallbacks() {
        let normalized =
            ProviderKind::OpenAIRealtime
                .capabilities()
                .normalize(ProviderPreferences {
                    source_language: SourceLanguage::Japanese,
                    target_language: TargetLanguage::Original,
                    translation_mode: TranslationMode::HighQuality,
                });
        assert_eq!(
            normalized,
            ProviderPreferences {
                source_language: SourceLanguage::Automatic,
                target_language: TargetLanguage::SimplifiedChinese,
                translation_mode: TranslationMode::Turbo,
            }
        );
    }

    #[test]
    fn explicit_source_providers_do_not_fallback_to_the_target_language() {
        let normalized =
            ProviderKind::VolcanoEngine
                .capabilities()
                .normalize(ProviderPreferences {
                    source_language: SourceLanguage::Automatic,
                    target_language: TargetLanguage::Japanese,
                    translation_mode: TranslationMode::HighQuality,
                });
        assert_eq!(normalized.source_language, SourceLanguage::English);
        assert_eq!(normalized.translation_mode, TranslationMode::Turbo);
    }

    #[test]
    fn explicit_source_providers_keep_chinese_translation_enabled() {
        for provider in [
            ProviderKind::VolcanoEngine,
            ProviderKind::TencentCloud,
            ProviderKind::BaiduTranslate,
            ProviderKind::DeepLX,
        ] {
            let capabilities = provider.capabilities();
            assert_eq!(
                capabilities.target_language_after_source_switch(
                    SourceLanguage::Chinese,
                    TargetLanguage::English,
                ),
                TargetLanguage::English
            );
            assert_eq!(
                capabilities.target_language_after_source_switch(
                    SourceLanguage::Chinese,
                    TargetLanguage::SimplifiedChinese,
                ),
                TargetLanguage::English
            );
        }
    }

    #[test]
    fn alibaba_source_selection_preserves_an_explicit_translation_target() {
        let profile = ServiceProfile::alibaba_default();
        let capabilities = profile.capabilities(TargetLanguage::English);
        assert_eq!(
            capabilities.target_language_after_source_switch(
                SourceLanguage::Chinese,
                TargetLanguage::English,
            ),
            TargetLanguage::English
        );
    }

    #[test]
    fn alibaba_source_selection_keeps_original_for_audio3_only_languages() {
        let profile = ServiceProfile::alibaba_default();
        let capabilities = profile.capabilities(TargetLanguage::Original);
        for source in [
            SourceLanguage::Norwegian,
            SourceLanguage::Romanian,
            SourceLanguage::Greek,
            SourceLanguage::Bulgarian,
            SourceLanguage::Croatian,
            SourceLanguage::Slovak,
        ] {
            let target =
                capabilities.target_language_after_source_switch(source, TargetLanguage::Original);
            assert_eq!(target, TargetLanguage::Original);
            let configuration =
                crate::core::configuration::LiveTranslationConfiguration::for_provider(
                    profile.provider,
                    "synthetic",
                    source,
                    target,
                    TranslationMode::Turbo,
                );
            let validated = configuration.validated().unwrap();
            assert_eq!(validated.source_language, source);
            assert_eq!(validated.target_language, TargetLanguage::Original);
        }
    }

    #[test]
    fn profiles_are_non_secret_and_trim_names() {
        let profile =
            ServiceProfile::new("openai-1", "  OpenAI Work  ", ProviderKind::OpenAIRealtime)
                .unwrap();
        assert_eq!(profile.name, "OpenAI Work");
        let json = serde_json::to_value(&profile).unwrap();
        assert_eq!(json["provider"], "openAIRealtime");
        assert!(json.get("apiKey").is_none());
        assert!(json.get("credential").is_none());
    }

    #[test]
    fn default_profile_preserves_alibaba_compatibility() {
        let profile = ServiceProfile::default();
        assert_eq!(profile.id, DEFAULT_ALIBABA_PROFILE_ID);
        assert_eq!(profile.provider, ProviderKind::AlibabaCloud);
    }
}
