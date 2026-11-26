//! Route-aware provider language catalogs and preference normalization.

use super::{AlibabaRealtimeMode, ProviderKind, TextTranslation, XAISpeechMode};
use crate::core::models::{SourceLanguage, TargetLanguage, TranslationMode};
use crate::core::protocols::{
    realtime::baidu_translate, realtime::gemini_live, realtime::live_translate,
    realtime::openai_realtime, realtime::tencent_cloud, realtime::volcano_engine,
    realtime::xai_realtime, recognition::audio3, translation::deepl_languages,
};

impl XAISpeechMode {
    pub fn capabilities(self) -> ProviderCapabilities {
        match self {
            Self::Voice => ProviderKind::XAIRealtime.capabilities(),
            Self::AsrMt => ProviderCapabilities {
                source_languages: [
                    vec![SourceLanguage::Automatic],
                    source_languages(crate::core::protocols::recognition::xai_stt::LANGUAGE_CODES),
                ]
                .concat(),
                target_languages: TargetLanguage::ALL.to_vec(),
                translation_modes: vec![TranslationMode::Turbo],
                input_sample_rate_hz: 16_000,
                target_pairs: Vec::new(),
            },
        }
    }
}

impl AlibabaRealtimeMode {
    /// Qwen3.8 recognizes input automatically; all 60 text targets are available.
    pub fn capabilities(self) -> ProviderCapabilities {
        ProviderCapabilities {
            source_languages: vec![SourceLanguage::Automatic],
            target_languages: [
                vec![TargetLanguage::Original],
                target_languages(
                    &live_translate::TRANSLATION_LANGUAGE_CODES
                        .iter()
                        .map(|code| match *code {
                            "nb" => "no",
                            "fil" => "tl",
                            code => code,
                        })
                        .collect::<Vec<_>>(),
                ),
            ]
            .concat(),
            translation_modes: vec![TranslationMode::Turbo],
            input_sample_rate_hz: 16_000,
            target_pairs: Vec::new(),
        }
    }
}

impl ProviderKind {
    pub fn capabilities_for_route(
        self,
        route: TextTranslation,
        target: TargetLanguage,
    ) -> ProviderCapabilities {
        self.capabilities_for_route_with_model(
            route,
            target,
            crate::core::protocols::translation::qwen_mt::REALTIME_MT_MODEL,
        )
    }

    pub fn capabilities_for_route_with_model(
        self,
        route: TextTranslation,
        target: TargetLanguage,
        model: crate::core::protocols::translation::qwen_mt::QwenMTModel,
    ) -> ProviderCapabilities {
        if self.is_standalone_asr() {
            custom_speech_capabilities(self, route, target)
        } else if matches!(self, Self::AlibabaCloud | Self::DeepLX) {
            let route = if self == Self::DeepLX && route == TextTranslation::FollowService {
                TextTranslation::DeepLX
            } else {
                route
            };
            alibaba_capabilities_with_model(route, target, model)
        } else {
            self.capabilities()
        }
    }

    pub fn capabilities(self) -> ProviderCapabilities {
        match self {
            Self::AlibabaCloud => alibaba_capabilities(
                TextTranslation::FollowService,
                TargetLanguage::SimplifiedChinese,
            ),
            Self::OpenAIRealtime | Self::AzureOpenAIRealtime => catalog_capabilities(
                &["auto"],
                openai_realtime::TRANSLATION_LANGUAGE_CODES,
                24_000,
                false,
            ),
            Self::XAIRealtime => {
                let mut capabilities = catalog_capabilities(
                    xai_realtime::LANGUAGE_CODES,
                    xai_realtime::LANGUAGE_CODES,
                    24_000,
                    false,
                );
                capabilities
                    .source_languages
                    .insert(0, SourceLanguage::Automatic);
                capabilities
            }
            Self::DeepLX => {
                alibaba_capabilities(TextTranslation::DeepLX, TargetLanguage::SimplifiedChinese)
            }
            Self::GoogleGeminiLive => {
                catalog_capabilities(&["auto"], gemini_live::LANGUAGE_CODES, 16_000, false)
            }
            Self::VolcanoEngine => {
                let mut capabilities = catalog_capabilities(
                    volcano_engine::SOURCE_LANGUAGE_CODES,
                    volcano_engine::TARGET_LANGUAGE_CODES,
                    16_000,
                    false,
                );
                capabilities.target_pairs = capabilities
                    .source_languages
                    .iter()
                    .map(|source| {
                        (
                            *source,
                            target_languages(volcano_engine::supported_target_codes(
                                source.raw_value(),
                            )),
                        )
                    })
                    .collect();
                capabilities
            }
            Self::TencentCloud => {
                let sources = ["zh", "en", "zh_en", "ja", "ko", "yue", "id", "th", "ru"];
                let targets = ["zh", "en", "ja", "ko", "yue", "id", "th", "ru", "zh_en"];
                let mut capabilities = catalog_capabilities(&sources, &targets, 16_000, false);
                capabilities.target_pairs = capabilities
                    .source_languages
                    .iter()
                    .map(|source| {
                        (
                            *source,
                            target_languages(tencent_cloud::supported_target_codes(
                                source.raw_value(),
                            )),
                        )
                    })
                    .collect();
                capabilities
            }
            Self::BaiduTranslate => {
                let codes = baidu_translate::LANGUAGE_CODES
                    .iter()
                    .map(|(code, _)| *code)
                    .collect::<Vec<_>>();
                catalog_capabilities(&codes, &codes, 16_000, false)
            }
            Self::CustomDashScopeASR | Self::CustomOpenAIASR | Self::AppleSpeech => {
                custom_speech_capabilities(
                    self,
                    TextTranslation::FollowService,
                    TargetLanguage::Original,
                )
            }
        }
    }
}

pub(super) fn custom_speech_capabilities(
    provider: ProviderKind,
    route: TextTranslation,
    target: TargetLanguage,
) -> ProviderCapabilities {
    let mut capabilities = match route {
        TextTranslation::DeepL | TextTranslation::DeepLX => {
            let (sources, targets) = independent_catalog(route);
            catalog_capabilities(sources, targets, 16_000, true)
        }
        _ => ProviderCapabilities {
            source_languages: SourceLanguage::ALL.to_vec(),
            target_languages: if route == TextTranslation::FollowService {
                vec![TargetLanguage::Original]
            } else {
                TargetLanguage::ALL.to_vec()
            },
            translation_modes: vec![TranslationMode::Turbo],
            input_sample_rate_hz: 16_000,
            target_pairs: Vec::new(),
        },
    };
    if target == TargetLanguage::Original {
        capabilities.source_languages = SourceLanguage::ALL.to_vec();
    }
    if provider == ProviderKind::AppleSpeech
        || (route == TextTranslation::Apple && target.translates_audio())
    {
        capabilities
            .source_languages
            .retain(|language| *language != SourceLanguage::Automatic);
    } else if !capabilities
        .source_languages
        .contains(&SourceLanguage::Automatic)
    {
        capabilities
            .source_languages
            .insert(0, SourceLanguage::Automatic);
    }
    if provider == ProviderKind::CustomOpenAIASR {
        capabilities.input_sample_rate_hz = 24_000;
    }
    capabilities
}

/// xAI STT supplies recognition; the selected text route supplies its own
/// language limits. Original captions retain the complete STT catalog.
pub fn xai_asr_capabilities(
    route: TextTranslation,
    target: TargetLanguage,
) -> ProviderCapabilities {
    let mut capabilities =
        custom_speech_capabilities(ProviderKind::CustomDashScopeASR, route, target);
    let speech = XAISpeechMode::AsrMt.capabilities();
    capabilities
        .source_languages
        .retain(|language| speech.source_languages.contains(language));
    capabilities
}

/// Recognition model language hints intersect the independently selected text service.
pub fn realtime_asr_capabilities(
    provider: ProviderKind,
    route: TextTranslation,
    target: TargetLanguage,
) -> ProviderCapabilities {
    let mut capabilities = custom_speech_capabilities(ProviderKind::CustomOpenAIASR, route, target);
    capabilities.source_languages.retain(|language| {
        crate::core::protocols::recognition::realtime_asr::supports_language(provider, *language)
    });
    capabilities.input_sample_rate_hz = if provider == ProviderKind::GoogleGeminiLive {
        16_000
    } else {
        24_000
    };
    capabilities
}

pub(super) fn independent_catalog(
    route: TextTranslation,
) -> (&'static [&'static str], &'static [&'static str]) {
    if route == TextTranslation::DeepLX {
        (
            deepl_languages::DEEPLX_SOURCE_CODES,
            deepl_languages::DEEPLX_TARGET_CODES,
        )
    } else {
        (
            deepl_languages::DEEPL_SOURCE_CODES,
            deepl_languages::DEEPL_TARGET_CODES,
        )
    }
}

pub(super) fn source_languages(codes: &[&str]) -> Vec<SourceLanguage> {
    codes
        .iter()
        .filter_map(|code| {
            SourceLanguage::ALL
                .into_iter()
                .find(|language| language.raw_value() == *code)
        })
        .collect()
}

pub(super) fn target_languages(codes: &[&str]) -> Vec<TargetLanguage> {
    codes
        .iter()
        .filter_map(|code| {
            TargetLanguage::ALL
                .into_iter()
                .find(|language| language.raw_value() == *code)
        })
        .collect()
}

fn catalog_capabilities(
    sources: &[&str],
    targets: &[&str],
    rate: u32,
    original: bool,
) -> ProviderCapabilities {
    let mut target_languages = target_languages(targets);
    if original {
        target_languages.insert(0, TargetLanguage::Original);
    }
    ProviderCapabilities {
        source_languages: source_languages(sources),
        target_languages,
        translation_modes: vec![TranslationMode::Turbo],
        input_sample_rate_hz: rate,
        target_pairs: Vec::new(),
    }
}

/// Route-aware language capabilities. A representable code is not permission
/// to send it to a different service. Original uses the full Audio3 catalog;
/// translated output intersects it with the selected text service's catalog.
fn alibaba_capabilities(route: TextTranslation, target: TargetLanguage) -> ProviderCapabilities {
    alibaba_capabilities_with_model(
        route,
        target,
        crate::core::protocols::translation::qwen_mt::REALTIME_MT_MODEL,
    )
}

fn alibaba_capabilities_with_model(
    route: TextTranslation,
    target: TargetLanguage,
    model: crate::core::protocols::translation::qwen_mt::QwenMTModel,
) -> ProviderCapabilities {
    if matches!(route, TextTranslation::DeepL | TextTranslation::DeepLX) {
        let (sources, targets) = independent_catalog(route);
        let mut capabilities = catalog_capabilities(sources, targets, 16_000, true);
        capabilities.source_languages = SourceLanguage::ALL
            .into_iter()
            .filter(|source| {
                *source == SourceLanguage::Automatic
                    || (audio3::LANGUAGE_CODES.contains(&source.raw_value())
                        && (target == TargetLanguage::Original
                            || sources.contains(&source.raw_value())))
            })
            .collect();
        return capabilities;
    }
    if route.uses_chat_completions() || route == TextTranslation::Apple {
        return ProviderCapabilities {
            source_languages: SourceLanguage::ALL
                .into_iter()
                .filter(|source| {
                    (*source == SourceLanguage::Automatic
                        || audio3::LANGUAGE_CODES.contains(&source.raw_value()))
                        && (route != TextTranslation::Apple
                            || target == TargetLanguage::Original
                            || *source != SourceLanguage::Automatic)
                })
                .collect(),
            target_languages: TargetLanguage::ALL.to_vec(),
            translation_modes: vec![TranslationMode::Turbo],
            input_sample_rate_hz: 16_000,
            target_pairs: Vec::new(),
        };
    }
    let source_languages = SourceLanguage::ALL
        .into_iter()
        .filter(|source| {
            *source == SourceLanguage::Automatic
                || (audio3::LANGUAGE_CODES.contains(&source.raw_value())
                    && (target == TargetLanguage::Original
                        || model.supported_language_codes().contains(&match source {
                            SourceLanguage::Norwegian => "nb",
                            _ => source.raw_value(),
                        })))
        })
        .collect();
    let mut target_languages = vec![TargetLanguage::Original];
    target_languages.extend(
        model
            .supported_language_codes()
            .iter()
            .filter_map(|code| {
                TargetLanguage::ALL
                    .into_iter()
                    .find(|target| match target { TargetLanguage::Norwegian => "nb", _ => target.raw_value() } == *code)
            }),
    );
    ProviderCapabilities {
        source_languages,
        target_languages,
        translation_modes: vec![TranslationMode::Turbo],
        input_sample_rate_hz: 16_000,
        target_pairs: Vec::new(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCapabilities {
    pub source_languages: Vec<SourceLanguage>,
    pub target_languages: Vec<TargetLanguage>,
    pub translation_modes: Vec<TranslationMode>,
    pub input_sample_rate_hz: u32,
    target_pairs: Vec<(SourceLanguage, Vec<TargetLanguage>)>,
}

impl ProviderCapabilities {
    pub fn for_source(mut self, source: SourceLanguage) -> Self {
        if let Some((_, targets)) = self
            .target_pairs
            .iter()
            .find(|(candidate, _)| *candidate == source)
        {
            self.target_languages = targets.clone();
        }
        self
    }

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
        let targets = self
            .clone()
            .for_source(normalized.source_language)
            .target_languages;
        if !targets.contains(&normalized.target_language) {
            if let Some(fallback) = targets.first().copied() {
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
        if self.target_pairs.is_empty()
            && !targets.contains(&TargetLanguage::Original)
            && source_matches_target(normalized.source_language, normalized.target_language)
        {
            if let Some(fallback) = targets
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
