//! Core domain models shared by providers, session state, and IPC.

use serde::{Deserialize, Serialize};

/// Named presets or an opaque custom RGB color. Presentation only.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum SubtitleColor {
    #[default]
    White,
    Teal,
    Yellow,
    Green,
    Pink,
    Custom([u8; 3]),
}

impl TryFrom<String> for SubtitleColor {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "white" => Ok(Self::White),
            "teal" => Ok(Self::Teal),
            "yellow" => Ok(Self::Yellow),
            "green" => Ok(Self::Green),
            "pink" => Ok(Self::Pink),
            _ => {
                let bytes = value.as_bytes();
                if bytes.len() != 7
                    || bytes[0] != b'#'
                    || !bytes[1..].iter().all(u8::is_ascii_hexdigit)
                {
                    return Err("Expected a subtitle preset or #RRGGBB color");
                }
                let mut rgb = [0; 3];
                for (index, channel) in rgb.iter_mut().enumerate() {
                    let start = 1 + index * 2;
                    *channel = u8::from_str_radix(&value[start..start + 2], 16)
                        .map_err(|_| "Invalid RGB channel")?;
                }
                Ok(Self::Custom(rgb))
            }
        }
    }
}

impl From<SubtitleColor> for String {
    fn from(value: SubtitleColor) -> Self {
        match value {
            SubtitleColor::White => "white".into(),
            SubtitleColor::Teal => "teal".into(),
            SubtitleColor::Yellow => "yellow".into(),
            SubtitleColor::Green => "green".into(),
            SubtitleColor::Pink => "pink".into(),
            SubtitleColor::Custom([red, green, blue]) => format!("#{red:02X}{green:02X}{blue:02X}"),
        }
    }
}

/// Presentation only; this never changes provider recognition or translation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubtitleDisplayMode {
    #[default]
    Translation,
    Bilingual,
    Original,
}

impl SubtitleDisplayMode {
    pub fn next(self) -> Self {
        match self {
            Self::Translation => Self::Bilingual,
            Self::Bilingual => Self::Original,
            Self::Original => Self::Translation,
        }
    }
}

#[cfg(test)]
mod display_mode_tests {
    use super::{SubtitleColor, SubtitleDisplayMode};

    #[test]
    fn subtitle_palette_has_stable_wire_values() {
        for (color, name) in [
            (SubtitleColor::White, "white"),
            (SubtitleColor::Teal, "teal"),
            (SubtitleColor::Yellow, "yellow"),
            (SubtitleColor::Green, "green"),
            (SubtitleColor::Pink, "pink"),
        ] {
            assert_eq!(serde_json::to_value(color).unwrap(), name);
            assert_eq!(
                serde_json::from_value::<SubtitleColor>(name.into()).unwrap(),
                color
            );
        }
    }

    #[test]
    fn custom_subtitle_colors_are_validated_and_normalized() {
        let color: SubtitleColor = serde_json::from_str("\"#a1b2c3\"").unwrap();
        assert_eq!(color, SubtitleColor::Custom([0xa1, 0xb2, 0xc3]));
        assert_eq!(serde_json::to_value(color).unwrap(), "#A1B2C3");
        for invalid in [
            "",
            "red",
            "#fff",
            "#12345678",
            "#GG0011",
            "123456",
            "#é0011",
            "url(x)",
        ] {
            assert!(serde_json::from_value::<SubtitleColor>(invalid.into()).is_err());
        }
    }

    #[test]
    fn display_modes_round_trip_and_cycle_in_presentation_order() {
        let mut mode = SubtitleDisplayMode::default();
        for value in ["translation", "bilingual", "original"] {
            assert_eq!(serde_json::to_value(mode).unwrap(), value);
            assert_eq!(
                serde_json::from_value::<SubtitleDisplayMode>(value.into()).unwrap(),
                mode
            );
            mode = mode.next();
        }
        assert_eq!(mode, SubtitleDisplayMode::default());
    }
}

/// The language being recognized in system audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceLanguage {
    Automatic,
    Chinese,
    English,
    Japanese,
    Korean,
    Vietnamese,
    Thai,
    Indonesian,
    Malay,
    Filipino,
    Hindi,
    Arabic,
    French,
    German,
    Spanish,
    Portuguese,
    Russian,
    Italian,
    Dutch,
    Swedish,
    Danish,
    Finnish,
    Norwegian,
    Greek,
    Polish,
    Czech,
    Hungarian,
    Romanian,
    Bulgarian,
    Croatian,
    Slovak,
}

impl Serialize for SourceLanguage {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.raw_value())
    }
}

impl<'de> Deserialize<'de> for SourceLanguage {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::ALL
            .into_iter()
            .find(|language| language.raw_value() == value)
            .ok_or_else(|| serde::de::Error::custom("unknown source language"))
    }
}

impl SourceLanguage {
    /// Exact Audio3 hint catalog, including its real automatic mode.
    pub const ALL: [Self; 31] = [
        Self::Automatic,
        Self::Chinese,
        Self::English,
        Self::Japanese,
        Self::Korean,
        Self::Vietnamese,
        Self::Thai,
        Self::Indonesian,
        Self::Malay,
        Self::Filipino,
        Self::Hindi,
        Self::Arabic,
        Self::French,
        Self::German,
        Self::Spanish,
        Self::Portuguese,
        Self::Russian,
        Self::Italian,
        Self::Dutch,
        Self::Swedish,
        Self::Danish,
        Self::Finnish,
        Self::Norwegian,
        Self::Greek,
        Self::Polish,
        Self::Czech,
        Self::Hungarian,
        Self::Romanian,
        Self::Bulgarian,
        Self::Croatian,
        Self::Slovak,
    ];

    /// Service wire code used in protocol payloads.
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Automatic => "auto",
            Self::Chinese => "zh",
            Self::English => "en",
            Self::Japanese => "ja",
            Self::Korean => "ko",
            Self::Vietnamese => "vi",
            Self::Thai => "th",
            Self::Indonesian => "id",
            Self::Malay => "ms",
            Self::Filipino => "tl",
            Self::Hindi => "hi",
            Self::Arabic => "ar",
            Self::French => "fr",
            Self::German => "de",
            Self::Spanish => "es",
            Self::Portuguese => "pt",
            Self::Russian => "ru",
            Self::Italian => "it",
            Self::Dutch => "nl",
            Self::Swedish => "sv",
            Self::Danish => "da",
            Self::Finnish => "fi",
            Self::Norwegian => "no",
            Self::Greek => "el",
            Self::Polish => "pl",
            Self::Czech => "cs",
            Self::Hungarian => "hu",
            Self::Romanian => "ro",
            Self::Bulgarian => "bg",
            Self::Croatian => "hr",
            Self::Slovak => "sk",
        }
    }

    /// Parses only documented ASR codes and known service language names.
    /// This is not a script-equivalence test; MT bypass checks the raw report.
    pub fn from_detected(detected_language: Option<&str>) -> Option<Self> {
        let normalized = detected_language?.trim().to_ascii_lowercase();
        let code = match normalized.as_str() {
            "chinese" | "mandarin" => "zh",
            "english" => "en",
            "japanese" => "ja",
            "korean" => "ko",
            "fil" | "filipino" | "tagalog" => "tl",
            _ => normalized.split('-').next().unwrap_or(&normalized),
        };
        Self::ALL
            .into_iter()
            .find(|language| *language != Self::Automatic && language.raw_value() == code)
    }
}

/// A language code reported by the recognition service.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DetectedLanguage {
    pub code: String,
}

impl DetectedLanguage {
    pub fn from_reported(reported_language: Option<&str>) -> Option<Self> {
        let normalized = reported_language?.trim().to_lowercase();
        if normalized.is_empty() {
            return None;
        }
        let code = normalized
            .split('-')
            .next()
            .unwrap_or(&normalized)
            .to_string();
        Some(Self { code })
    }
}

/// The language subtitles are translated into; `Original` means no translation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TargetLanguage {
    Original,
    SimplifiedChinese,
    English,
    Japanese,
    TraditionalChinese,
    Korean,
    Russian,
    Spanish,
    French,
    Portuguese,
    German,
    Italian,
    Thai,
    Vietnamese,
    Indonesian,
    Malay,
    Arabic,
    Hindi,
    Hebrew,
    Urdu,
    Bengali,
    Polish,
    Dutch,
    Turkish,
    Khmer,
    Czech,
    Swedish,
    Hungarian,
    Danish,
    Finnish,
    Tagalog,
    Persian,
}

impl Serialize for TargetLanguage {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.raw_value())
    }
}

impl<'de> Deserialize<'de> for TargetLanguage {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::ALL
            .into_iter()
            .find(|language| language.raw_value() == value)
            .ok_or_else(|| serde::de::Error::custom("unknown target language"))
    }
}

impl TargetLanguage {
    /// Qwen-MT Lite's complete catalog, plus the local no-translation choice.
    pub const ALL: [Self; 32] = [
        Self::Original,
        Self::SimplifiedChinese,
        Self::English,
        Self::Japanese,
        Self::TraditionalChinese,
        Self::Korean,
        Self::Russian,
        Self::Spanish,
        Self::French,
        Self::Portuguese,
        Self::German,
        Self::Italian,
        Self::Thai,
        Self::Vietnamese,
        Self::Indonesian,
        Self::Malay,
        Self::Arabic,
        Self::Hindi,
        Self::Hebrew,
        Self::Urdu,
        Self::Bengali,
        Self::Polish,
        Self::Dutch,
        Self::Turkish,
        Self::Khmer,
        Self::Czech,
        Self::Swedish,
        Self::Hungarian,
        Self::Danish,
        Self::Finnish,
        Self::Tagalog,
        Self::Persian,
    ];

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Original => "original",
            Self::SimplifiedChinese => "zh",
            Self::English => "en",
            Self::Japanese => "ja",
            Self::TraditionalChinese => "zh_tw",
            Self::Korean => "ko",
            Self::Russian => "ru",
            Self::Spanish => "es",
            Self::French => "fr",
            Self::Portuguese => "pt",
            Self::German => "de",
            Self::Italian => "it",
            Self::Thai => "th",
            Self::Vietnamese => "vi",
            Self::Indonesian => "id",
            Self::Malay => "ms",
            Self::Arabic => "ar",
            Self::Hindi => "hi",
            Self::Hebrew => "he",
            Self::Urdu => "ur",
            Self::Bengali => "bn",
            Self::Polish => "pl",
            Self::Dutch => "nl",
            Self::Turkish => "tr",
            Self::Khmer => "km",
            Self::Czech => "cs",
            Self::Swedish => "sv",
            Self::Hungarian => "hu",
            Self::Danish => "da",
            Self::Finnish => "fi",
            Self::Tagalog => "tl",
            Self::Persian => "fa",
        }
    }

    /// Official service-side name; historical names remain unchanged.
    pub fn qwen_mt_name(self) -> &'static str {
        match self {
            Self::Original => "",
            Self::SimplifiedChinese => "Chinese",
            Self::English => "English",
            Self::Japanese => "Japanese",
            Self::TraditionalChinese => "Traditional Chinese",
            Self::Korean => "Korean",
            Self::Russian => "Russian",
            Self::Spanish => "Spanish",
            Self::French => "French",
            Self::Portuguese => "Portuguese",
            Self::German => "German",
            Self::Italian => "Italian",
            Self::Thai => "Thai",
            Self::Vietnamese => "Vietnamese",
            Self::Indonesian => "Indonesian",
            Self::Malay => "Malay",
            Self::Arabic => "Arabic",
            Self::Hindi => "Hindi",
            Self::Hebrew => "Hebrew",
            Self::Urdu => "Urdu",
            Self::Bengali => "Bengali",
            Self::Polish => "Polish",
            Self::Dutch => "Dutch",
            Self::Turkish => "Turkish",
            Self::Khmer => "Khmer",
            Self::Czech => "Czech",
            Self::Swedish => "Swedish",
            Self::Hungarian => "Hungarian",
            Self::Danish => "Danish",
            Self::Finnish => "Finnish",
            Self::Tagalog => "Tagalog",
            Self::Persian => "Persian",
        }
    }

    pub fn translates_audio(self) -> bool {
        self != TargetLanguage::Original
    }

    /// Only explicit ASR reports can skip text translation. In particular,
    /// traditional Chinese still needs conversion to the simplified target.
    pub fn matches_reported_asr(self, reported: Option<&str>) -> bool {
        let Some(reported) = reported else {
            return false;
        };
        let reported = reported.trim().to_ascii_lowercase();
        match self {
            Self::Original => false,
            Self::SimplifiedChinese => matches!(reported.as_str(), "zh" | "zh-cn" | "zh-hans"),
            Self::English => reported == "en",
            Self::TraditionalChinese => matches!(reported.as_str(), "zh_tw" | "zh-tw" | "zh-hant"),
            _ => reported == self.raw_value(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TranslationMode {
    LowLatency,
    HighQuality,
    Turbo,
}

impl Serialize for TranslationMode {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(match self {
            Self::LowLatency => "lowLatency",
            Self::HighQuality => "highQuality",
            Self::Turbo => "turbo",
        })
    }
}

impl<'de> Deserialize<'de> for TranslationMode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "lowLatency" => Ok(Self::LowLatency),
            "highQuality" => Ok(Self::HighQuality),
            "turbo" => Ok(Self::Turbo),
            other => Err(serde::de::Error::custom(format!(
                "unknown translation mode: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionStatus {
    Idle,
    Connecting,
    Listening,
    Stopping,
    Error(String),
}

impl SessionStatus {
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            SessionStatus::Connecting | SessionStatus::Listening | SessionStatus::Stopping
        )
    }
}

#[cfg(test)]
pub use mimi_core::models::PreviewSubtitlePair;
pub use mimi_core::models::{
    subtitle_text_within_limit, SourceSubtitleSnapshot, SubtitleEvent, SubtitlePair,
    SubtitleSnapshot, UtteranceRole, MAX_SUBTITLE_TEXT_BYTES,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_language_codes_round_trip_without_unknown_fallback() {
        assert_eq!(SourceLanguage::ALL.len(), 31);
        assert_eq!(TargetLanguage::ALL.len(), 32);
        let mut sources = std::collections::HashSet::new();
        for language in SourceLanguage::ALL {
            assert!(sources.insert(language.raw_value()));
            assert_eq!(
                serde_json::from_value::<SourceLanguage>(serde_json::to_value(language).unwrap())
                    .unwrap(),
                language
            );
            if language != SourceLanguage::Automatic {
                assert_eq!(
                    SourceLanguage::from_detected(Some(language.raw_value())),
                    Some(language)
                );
            }
        }
        let mut targets = std::collections::HashSet::new();
        for language in TargetLanguage::ALL {
            assert!(targets.insert(language.raw_value()));
            assert_eq!(
                serde_json::from_value::<TargetLanguage>(serde_json::to_value(language).unwrap())
                    .unwrap(),
                language
            );
        }
        for unknown in ["xx", "zh-hant", "fr-FR", "", "auto"] {
            assert!(serde_json::from_value::<TargetLanguage>(serde_json::json!(unknown)).is_err());
        }
        assert!(serde_json::from_value::<SourceLanguage>(serde_json::json!("he")).is_err());
        assert_eq!(SourceLanguage::from_detected(Some("unrecognized")), None);
    }

    #[test]
    fn same_language_bypass_requires_explicit_matching_code_and_script() {
        for source in SourceLanguage::ALL
            .into_iter()
            .filter(|source| *source != SourceLanguage::Automatic)
        {
            for target in TargetLanguage::ALL {
                assert_eq!(
                    target.matches_reported_asr(Some(source.raw_value())),
                    target.raw_value() == source.raw_value()
                );
            }
        }
        assert!(!TargetLanguage::French.matches_reported_asr(None));
        assert!(!TargetLanguage::French.matches_reported_asr(Some("fr-FR")));
        assert!(!TargetLanguage::SimplifiedChinese.matches_reported_asr(Some("zh_tw")));
        assert!(!TargetLanguage::TraditionalChinese.matches_reported_asr(Some("zh")));
        assert!(TargetLanguage::TraditionalChinese.matches_reported_asr(Some("zh-Hant")));
        assert!(!TargetLanguage::TraditionalChinese.matches_reported_asr(Some("unknown")));
    }

    #[test]
    fn target_languages_expose_service_codes_and_display_names() {
        assert!(!TargetLanguage::Original.translates_audio());
        assert_eq!(TargetLanguage::SimplifiedChinese.raw_value(), "zh");
        assert_eq!(TargetLanguage::English.qwen_mt_name(), "English");
    }

    #[test]
    fn same_language_passthrough_requires_an_explicit_compatible_asr_report() {
        for (target, reported) in [
            (TargetLanguage::English, "en"),
            (TargetLanguage::Japanese, "JA"),
            (TargetLanguage::SimplifiedChinese, "zh"),
            (TargetLanguage::SimplifiedChinese, " zh-CN "),
            (TargetLanguage::SimplifiedChinese, "zh-Hans"),
        ] {
            assert!(target.matches_reported_asr(Some(reported)));
        }
        for target in [
            TargetLanguage::Original,
            TargetLanguage::English,
            TargetLanguage::Japanese,
            TargetLanguage::SimplifiedChinese,
        ] {
            for reported in [None, Some(""), Some("unknown"), Some("yue")] {
                assert!(!target.matches_reported_asr(reported));
            }
        }
        for reported in ["zh-TW", "zh-HK", "zh-Hant", "ja", "en", "Chinese"] {
            assert!(!TargetLanguage::SimplifiedChinese.matches_reported_asr(Some(reported)));
        }
        assert!(!TargetLanguage::English.matches_reported_asr(Some("ja")));
        assert!(!TargetLanguage::Japanese.matches_reported_asr(Some("en")));
        assert!(!TargetLanguage::Original.matches_reported_asr(Some("zh")));
    }

    #[test]
    fn detected_languages_normalize_service_codes() {
        assert_eq!(
            DetectedLanguage::from_reported(Some("ja-JP")).unwrap().code,
            "ja"
        );
        assert_eq!(
            DetectedLanguage::from_reported(Some("yue")).unwrap().code,
            "yue"
        );
        assert_eq!(
            DetectedLanguage::from_reported(Some("unknown"))
                .unwrap()
                .code,
            "unknown"
        );
    }

    #[test]
    fn session_status_active_flag_matches_lifecycle_contract() {
        assert!(!SessionStatus::Idle.is_active());
        assert!(SessionStatus::Connecting.is_active());
        assert!(SessionStatus::Listening.is_active());
        assert!(SessionStatus::Stopping.is_active());
        assert!(!SessionStatus::Error("boom".into()).is_active());
    }

    #[test]
    fn subtitle_pair_equality_ignores_creation_time() {
        let a = SubtitlePair::new("s".into(), "t".into(), 1);
        let b = SubtitlePair::new("s".into(), "t".into(), 999);
        assert_eq!(a, b);
    }
}
