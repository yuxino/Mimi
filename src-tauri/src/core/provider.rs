//! Provider identity, service profiles, and capability normalization.
//!
//! Profiles contain display metadata only. Credentials deliberately live in
//! a private local file and must never be serialized with a profile.

use crate::core::models::{SourceLanguage, TargetLanguage, TranslationMode};
use crate::core::network_proxy::{ProxyConfig, ProxyConfigError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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
    #[serde(rename = "customDashScopeASR")]
    CustomDashScopeASR,
    #[serde(rename = "customOpenAIASR")]
    CustomOpenAIASR,
}

impl ProviderKind {
    pub fn capabilities_for_route(
        self,
        route: TextTranslation,
        target: TargetLanguage,
    ) -> ProviderCapabilities {
        if self.is_custom_speech() {
            custom_speech_capabilities(self, route)
        } else if self == Self::AlibabaCloud {
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
            Self::CustomDashScopeASR => "customDashScopeASR",
            Self::CustomOpenAIASR => "customOpenAIASR",
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
            Self::CustomDashScopeASR => "Custom DashScope ASR",
            Self::CustomOpenAIASR => "Custom OpenAI ASR",
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
            Self::CustomDashScopeASR | Self::CustomOpenAIASR => {
                custom_speech_capabilities(self, TextTranslation::FollowService)
            }
        }
    }

    pub const fn is_custom_speech(self) -> bool {
        matches!(self, Self::CustomDashScopeASR | Self::CustomOpenAIASR)
    }

    pub const fn supports_text_translation(self) -> bool {
        matches!(self, Self::AlibabaCloud | Self::DeepLX) || self.is_custom_speech()
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

fn custom_speech_capabilities(
    provider: ProviderKind,
    route: TextTranslation,
) -> ProviderCapabilities {
    let mut target_languages = vec![TargetLanguage::Original];
    if route != TextTranslation::FollowService {
        target_languages.extend([
            TargetLanguage::SimplifiedChinese,
            TargetLanguage::English,
            TargetLanguage::Japanese,
        ]);
    }
    ProviderCapabilities {
        source_languages: vec![
            SourceLanguage::Automatic,
            SourceLanguage::Chinese,
            SourceLanguage::English,
            SourceLanguage::Japanese,
            SourceLanguage::Korean,
        ],
        target_languages,
        translation_modes: vec![TranslationMode::Turbo],
        input_sample_rate_hz: if provider == ProviderKind::CustomOpenAIASR {
            24_000
        } else {
            16_000
        },
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
        TextTranslation::DeepL | TextTranslation::OpenAICompatible | TextTranslation::ChatMock
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
}

/// Independent text translation for supported speech-recognition chains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TextTranslation {
    #[serde(rename = "followService")]
    FollowService,
    #[serde(rename = "deepL")]
    DeepL,
    #[serde(rename = "deepLX")]
    DeepLX,
    #[serde(rename = "openAICompatible")]
    OpenAICompatible,
    #[serde(rename = "chatMock")]
    ChatMock,
}

/// A patch for one route's optional display name; an empty name removes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextTranslationName {
    pub route: TextTranslation,
    pub name: String,
}

impl TextTranslation {
    pub const fn uses_chat_completions(self) -> bool {
        matches!(self, Self::OpenAICompatible | Self::ChatMock)
    }
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
            text_translation: None,
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
            text_translation: None,
            text_translation_names: BTreeMap::new(),
            speech_network_proxy: None,
            text_network_proxy: None,
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
                    | TextTranslation::ChatMock
            )
        ) && !self.provider.supports_text_translation()
        {
            return Err(ServiceProfileError::UnsupportedTextTranslation);
        }
        profile.text_translation = self.text_translation;
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

    pub fn set_text_translation_name(
        &mut self,
        route: TextTranslation,
        name: &str,
    ) -> Result<(), ServiceProfileError> {
        if route == TextTranslation::FollowService || !self.provider.supports_text_translation() {
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
        self.text_translation
            .unwrap_or(if self.provider == ProviderKind::DeepLX {
                TextTranslation::DeepLX
            } else {
                TextTranslation::FollowService
            })
    }

    pub fn capabilities(&self, target: TargetLanguage) -> ProviderCapabilities {
        if self.provider.is_custom_speech() {
            custom_speech_capabilities(self.provider, self.text_translation())
        } else if self.effective_provider() == ProviderKind::AlibabaCloud {
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
                | TextTranslation::OpenAICompatible
                | TextTranslation::ChatMock,
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
    #[test]
    fn profile_proxy_fields_read_legacy_and_normalize_without_losing_preferences() {
        use crate::core::network_proxy::ProxyMode;
        let legacy: ServiceProfile =
            serde_json::from_str(r#"{"id":"legacy","name":"Legacy","provider":"alibabaCloud"}"#)
                .unwrap();
        assert_eq!(legacy.speech_network_proxy, None);
        assert_eq!(legacy.text_network_proxy, None);
        let mut profile = legacy;
        profile.speech_network_proxy = Some(ProxyConfig {
            mode: ProxyMode::Custom,
            url: Some(" socks5h://127.0.0.1 ".into()),
        });
        profile.text_network_proxy = Some(ProxyConfig {
            mode: ProxyMode::Direct,
            url: Some("stale-address".into()),
        });
        let validated = profile.validated().unwrap();
        assert_eq!(
            validated
                .speech_network_proxy
                .as_ref()
                .unwrap()
                .url
                .as_deref(),
            Some("socks5h://127.0.0.1:1080")
        );
        assert_eq!(validated.text_network_proxy.as_ref().unwrap().url, None);
        let saved: ServiceProfile =
            serde_json::from_str(&serde_json::to_string(&validated).unwrap()).unwrap();
        assert_eq!(saved, validated);
        profile.speech_network_proxy.as_mut().unwrap().url =
            Some("http://user:private-value@localhost:7890".into());
        let error = profile.validated().unwrap_err();
        assert!(!error.to_string().contains("private-value"));
        assert_eq!(
            error.to_string(),
            "network_proxy_authentication_unsupported"
        );
        assert!(!format!("{profile:?}").contains("private-value"));
    }

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
    fn chatmock_has_its_own_wire_identity_without_changing_generic_profiles() {
        for provider in [
            ProviderKind::AlibabaCloud,
            ProviderKind::DeepLX,
            ProviderKind::CustomDashScopeASR,
            ProviderKind::CustomOpenAIASR,
        ] {
            let mut generic = ServiceProfile::new("synthetic", "Synthetic", provider).unwrap();
            generic.text_translation = Some(TextTranslation::OpenAICompatible);
            let mut chatmock = generic.clone();
            chatmock.text_translation = Some(TextTranslation::ChatMock);
            assert_eq!(
                serde_json::to_value(&generic).unwrap()["textTranslation"],
                "openAICompatible"
            );
            assert_eq!(
                serde_json::to_value(&chatmock).unwrap()["textTranslation"],
                "chatMock"
            );
            let decoded: ServiceProfile =
                serde_json::from_value(serde_json::to_value(&chatmock).unwrap()).unwrap();
            assert_eq!(decoded.validated().unwrap(), chatmock);
            assert_eq!(chatmock.effective_provider(), generic.effective_provider());
            assert_eq!(
                chatmock.capabilities(TargetLanguage::SimplifiedChinese),
                generic.capabilities(TargetLanguage::SimplifiedChinese)
            );
        }
        let mut unsupported =
            ServiceProfile::new("other", "Other", ProviderKind::OpenAIRealtime).unwrap();
        unsupported.text_translation = Some(TextTranslation::ChatMock);
        assert!(unsupported.validated().is_err());
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
            (ProviderKind::CustomDashScopeASR, "customDashScopeASR"),
            (ProviderKind::CustomOpenAIASR, "customOpenAIASR"),
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
    fn custom_speech_keeps_protocol_identity_and_requires_an_independent_translation_route() {
        for (provider, sample_rate) in [
            (ProviderKind::CustomDashScopeASR, 16_000),
            (ProviderKind::CustomOpenAIASR, 24_000),
        ] {
            let mut profile = ServiceProfile::new("custom", "Custom speech", provider).unwrap();
            assert_eq!(profile.validated().unwrap().effective_provider(), provider);
            assert_eq!(
                profile
                    .capabilities(TargetLanguage::English)
                    .target_languages,
                vec![TargetLanguage::Original]
            );
            for translation in [
                TextTranslation::DeepL,
                TextTranslation::DeepLX,
                TextTranslation::OpenAICompatible,
            ] {
                profile.text_translation = Some(translation);
                assert_eq!(profile.validated().unwrap().effective_provider(), provider);
                let caps = profile.capabilities(TargetLanguage::English);
                assert_eq!(caps.input_sample_rate_hz, sample_rate);
                assert_eq!(caps.source_languages.len(), 5);
                assert_eq!(
                    caps.target_languages,
                    vec![
                        TargetLanguage::Original,
                        TargetLanguage::SimplifiedChinese,
                        TargetLanguage::English,
                        TargetLanguage::Japanese
                    ]
                );
                assert!(!caps.source_languages.contains(&SourceLanguage::French));
            }
            profile.text_translation = Some(TextTranslation::FollowService);
            let normalized = profile.normalize_preferences(ProviderPreferences {
                source_language: SourceLanguage::French,
                target_language: TargetLanguage::English,
                translation_mode: TranslationMode::Turbo,
            });
            assert_eq!(normalized.target_language, TargetLanguage::Original);
            assert_eq!(normalized.source_language, SourceLanguage::Automatic);
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
    fn text_translation_names_are_optional_trimmed_route_metadata() {
        let mut profile: ServiceProfile = serde_json::from_value(serde_json::json!({
            "id": "legacy", "name": "Legacy", "provider": "alibabaCloud"
        }))
        .unwrap();
        assert!(profile.text_translation_names.is_empty());
        profile
            .set_text_translation_name(TextTranslation::OpenAICompatible, "  My translator  ")
            .unwrap();
        profile
            .set_text_translation_name(TextTranslation::DeepLX, "Local translator")
            .unwrap();
        let restored = profile.validated().unwrap();
        assert_eq!(restored, profile);
        let json = serde_json::to_value(&restored).unwrap();
        assert_eq!(
            json["textTranslationNames"]["openAICompatible"],
            "My translator"
        );
        assert_eq!(json["textTranslationNames"]["deepLX"], "Local translator");
        profile
            .set_text_translation_name(TextTranslation::OpenAICompatible, "  ")
            .unwrap();
        assert!(!profile
            .text_translation_names
            .contains_key(&TextTranslation::OpenAICompatible));
        assert_eq!(profile.text_translation_names.len(), 1);
    }

    #[test]
    fn text_translation_names_reject_unsupported_routes_and_invalid_names() {
        let mut profile = ServiceProfile::alibaba_default();
        assert_eq!(
            profile.set_text_translation_name(TextTranslation::FollowService, "Alias"),
            Err(ServiceProfileError::UnsupportedTextTranslation)
        );
        for name in [
            "x".repeat(65),
            "line\nbreak".into(),
            "control\u{0000}".into(),
        ] {
            assert_eq!(
                profile.set_text_translation_name(TextTranslation::OpenAICompatible, &name),
                Err(ServiceProfileError::InvalidTextTranslationName)
            );
        }
        profile
            .set_text_translation_name(TextTranslation::OpenAICompatible, &"名".repeat(64))
            .unwrap();
        profile.provider = ProviderKind::OpenAIRealtime;
        assert_eq!(
            profile.validated(),
            Err(ServiceProfileError::UnsupportedTextTranslation)
        );
    }

    #[test]
    fn default_profile_preserves_alibaba_compatibility() {
        let profile = ServiceProfile::default();
        assert_eq!(profile.id, DEFAULT_ALIBABA_PROFILE_ID);
        assert_eq!(profile.provider, ProviderKind::AlibabaCloud);
    }
}
