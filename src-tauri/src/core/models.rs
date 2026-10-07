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
    Shanghainese,
    Asturian,
    Acehnese,
    Afrikaans,
    Akan,
    Amharic,
    Aragonese,
    ArabicUnitedArabEmirates,
    ArabicEgypt,
    ArabicSaudiArabia,
    Assamese,
    Aymara,
    Azerbaijani,
    Bashkir,
    Belarusian,
    Bhojpuri,
    Bangla,
    Breton,
    Bosnian,
    Catalan,
    Cebuano,
    CentralKurdish,
    Welsh,
    BritishEnglish,
    AmericanEnglish,
    Esperanto,
    LatinAmericanSpanish,
    EuropeanSpanish,
    MexicanSpanish,
    Estonian,
    Basque,
    Persian,
    Irish,
    Galician,
    Guarani,
    Konkani,
    Gujarati,
    Hausa,
    Hebrew,
    HaitianCreole,
    Armenian,
    Igbo,
    Icelandic,
    Javanese,
    Georgian,
    Kazakh,
    Khmer,
    NorthernKurdish,
    Kannada,
    Kyrgyz,
    Latin,
    Luxembourgish,
    Lombard,
    Lingala,
    Lao,
    Lithuanian,
    Latvian,
    Maithili,
    Malagasy,
    Maori,
    Macedonian,
    Malayalam,
    Mongolian,
    Marathi,
    Maltese,
    Burmese,
    Nepali,
    Occitan,
    Oromo,
    Punjabi,
    Pangasinan,
    Pampanga,
    Dari,
    Pashto,
    BrazilianPortuguese,
    EuropeanPortuguese,
    Quechua,
    Kinyarwanda,
    Sanskrit,
    Sicilian,
    Sindhi,
    Sinhala,
    Slovenian,
    Albanian,
    Serbian,
    SouthernSotho,
    Sundanese,
    Swahili,
    Tamil,
    Telugu,
    Tajik,
    Turkmen,
    Tswana,
    Turkish,
    Tsonga,
    Tatar,
    Ukrainian,
    Urdu,
    Uzbek,
    Wolof,
    Xhosa,
    Yiddish,
    Cantonese,
    ChineseEnglishMixed,
    TraditionalChinese,
    Zulu,
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
    /// Serializable language vocabulary; each provider applies its own allowlist.
    pub const ALL: [Self; 137] = [
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
        Self::Shanghainese,
        Self::Asturian,
        Self::Acehnese,
        Self::Afrikaans,
        Self::Akan,
        Self::Amharic,
        Self::Aragonese,
        Self::ArabicUnitedArabEmirates,
        Self::ArabicEgypt,
        Self::ArabicSaudiArabia,
        Self::Assamese,
        Self::Aymara,
        Self::Azerbaijani,
        Self::Bashkir,
        Self::Belarusian,
        Self::Bhojpuri,
        Self::Bangla,
        Self::Breton,
        Self::Bosnian,
        Self::Catalan,
        Self::Cebuano,
        Self::CentralKurdish,
        Self::Welsh,
        Self::BritishEnglish,
        Self::AmericanEnglish,
        Self::Esperanto,
        Self::LatinAmericanSpanish,
        Self::EuropeanSpanish,
        Self::MexicanSpanish,
        Self::Estonian,
        Self::Basque,
        Self::Persian,
        Self::Irish,
        Self::Galician,
        Self::Guarani,
        Self::Konkani,
        Self::Gujarati,
        Self::Hausa,
        Self::Hebrew,
        Self::HaitianCreole,
        Self::Armenian,
        Self::Igbo,
        Self::Icelandic,
        Self::Javanese,
        Self::Georgian,
        Self::Kazakh,
        Self::Khmer,
        Self::NorthernKurdish,
        Self::Kannada,
        Self::Kyrgyz,
        Self::Latin,
        Self::Luxembourgish,
        Self::Lombard,
        Self::Lingala,
        Self::Lao,
        Self::Lithuanian,
        Self::Latvian,
        Self::Maithili,
        Self::Malagasy,
        Self::Maori,
        Self::Macedonian,
        Self::Malayalam,
        Self::Mongolian,
        Self::Marathi,
        Self::Maltese,
        Self::Burmese,
        Self::Nepali,
        Self::Occitan,
        Self::Oromo,
        Self::Punjabi,
        Self::Pangasinan,
        Self::Pampanga,
        Self::Dari,
        Self::Pashto,
        Self::BrazilianPortuguese,
        Self::EuropeanPortuguese,
        Self::Quechua,
        Self::Kinyarwanda,
        Self::Sanskrit,
        Self::Sicilian,
        Self::Sindhi,
        Self::Sinhala,
        Self::Slovenian,
        Self::Albanian,
        Self::Serbian,
        Self::SouthernSotho,
        Self::Sundanese,
        Self::Swahili,
        Self::Tamil,
        Self::Telugu,
        Self::Tajik,
        Self::Turkmen,
        Self::Tswana,
        Self::Turkish,
        Self::Tsonga,
        Self::Tatar,
        Self::Ukrainian,
        Self::Urdu,
        Self::Uzbek,
        Self::Wolof,
        Self::Xhosa,
        Self::Yiddish,
        Self::Cantonese,
        Self::ChineseEnglishMixed,
        Self::TraditionalChinese,
        Self::Zulu,
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
            Self::Shanghainese => "wuu",
            Self::Asturian => "ast",
            Self::Acehnese => "ace",
            Self::Afrikaans => "af",
            Self::Akan => "ak",
            Self::Amharic => "am",
            Self::Aragonese => "an",
            Self::ArabicUnitedArabEmirates => "ar-AE",
            Self::ArabicEgypt => "ar-EG",
            Self::ArabicSaudiArabia => "ar-SA",
            Self::Assamese => "as",
            Self::Aymara => "ay",
            Self::Azerbaijani => "az",
            Self::Bashkir => "ba",
            Self::Belarusian => "be",
            Self::Bhojpuri => "bho",
            Self::Bangla => "bn",
            Self::Breton => "br",
            Self::Bosnian => "bs",
            Self::Catalan => "ca",
            Self::Cebuano => "ceb",
            Self::CentralKurdish => "ckb",
            Self::Welsh => "cy",
            Self::BritishEnglish => "en-GB",
            Self::AmericanEnglish => "en-US",
            Self::Esperanto => "eo",
            Self::LatinAmericanSpanish => "es-419",
            Self::EuropeanSpanish => "es-ES",
            Self::MexicanSpanish => "es-MX",
            Self::Estonian => "et",
            Self::Basque => "eu",
            Self::Persian => "fa",
            Self::Irish => "ga",
            Self::Galician => "gl",
            Self::Guarani => "gn",
            Self::Konkani => "gom",
            Self::Gujarati => "gu",
            Self::Hausa => "ha",
            Self::Hebrew => "he",
            Self::HaitianCreole => "ht",
            Self::Armenian => "hy",
            Self::Igbo => "ig",
            Self::Icelandic => "is",
            Self::Javanese => "jv",
            Self::Georgian => "ka",
            Self::Kazakh => "kk",
            Self::Khmer => "km",
            Self::NorthernKurdish => "kmr",
            Self::Kannada => "kn",
            Self::Kyrgyz => "ky",
            Self::Latin => "la",
            Self::Luxembourgish => "lb",
            Self::Lombard => "lmo",
            Self::Lingala => "ln",
            Self::Lao => "lo",
            Self::Lithuanian => "lt",
            Self::Latvian => "lv",
            Self::Maithili => "mai",
            Self::Malagasy => "mg",
            Self::Maori => "mi",
            Self::Macedonian => "mk",
            Self::Malayalam => "ml",
            Self::Mongolian => "mn",
            Self::Marathi => "mr",
            Self::Maltese => "mt",
            Self::Burmese => "my",
            Self::Nepali => "ne",
            Self::Occitan => "oc",
            Self::Oromo => "om",
            Self::Punjabi => "pa",
            Self::Pangasinan => "pag",
            Self::Pampanga => "pam",
            Self::Dari => "prs",
            Self::Pashto => "ps",
            Self::BrazilianPortuguese => "pt-BR",
            Self::EuropeanPortuguese => "pt-PT",
            Self::Quechua => "qu",
            Self::Kinyarwanda => "rw",
            Self::Sanskrit => "sa",
            Self::Sicilian => "scn",
            Self::Sindhi => "sd",
            Self::Sinhala => "si",
            Self::Slovenian => "sl",
            Self::Albanian => "sq",
            Self::Serbian => "sr",
            Self::SouthernSotho => "st",
            Self::Sundanese => "su",
            Self::Swahili => "sw",
            Self::Tamil => "ta",
            Self::Telugu => "te",
            Self::Tajik => "tg",
            Self::Turkmen => "tk",
            Self::Tswana => "tn",
            Self::Turkish => "tr",
            Self::Tsonga => "ts",
            Self::Tatar => "tt",
            Self::Ukrainian => "uk",
            Self::Urdu => "ur",
            Self::Uzbek => "uz",
            Self::Wolof => "wo",
            Self::Xhosa => "xh",
            Self::Yiddish => "yi",
            Self::Cantonese => "yue",
            Self::ChineseEnglishMixed => "zh_en",
            Self::TraditionalChinese => "zh_tw",
            Self::Zulu => "zu",
        }
    }

    /// Parses the registry and known service aliases, preserving explicit regions.
    /// This is not a script-equivalence test; MT bypass checks the raw report.
    pub fn from_detected(detected_language: Option<&str>) -> Option<Self> {
        let normalized = detected_language?.trim().to_ascii_lowercase();
        let code = match normalized.as_str() {
            "chinese" | "mandarin" => "zh",
            "english" => "en",
            "japanese" => "ja",
            "korean" => "ko",
            "fil" | "filipino" | "tagalog" => "tl",
            "zh-hant" | "zh-tw" => "zh_tw",
            "zh-hans" | "zh-cn" => "zh",
            "nb" => "no",
            "sh-cn" => "wuu",
            _ => normalized.as_str(),
        };
        Self::ALL
            .into_iter()
            .find(|language| {
                *language != Self::Automatic && language.raw_value().eq_ignore_ascii_case(code)
            })
            .or_else(|| {
                let base = code.split('-').next()?;
                Self::ALL
                    .into_iter()
                    .find(|language| *language != Self::Automatic && language.raw_value() == base)
            })
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
    Shanghainese,
    Asturian,
    Acehnese,
    Afrikaans,
    Akan,
    Amharic,
    Aragonese,
    ArabicUnitedArabEmirates,
    ArabicEgypt,
    ArabicSaudiArabia,
    Assamese,
    Aymara,
    Azerbaijani,
    Bashkir,
    Belarusian,
    Bulgarian,
    Bhojpuri,
    Breton,
    Bosnian,
    Catalan,
    Cebuano,
    CentralKurdish,
    Welsh,
    Greek,
    BritishEnglish,
    AmericanEnglish,
    Esperanto,
    LatinAmericanSpanish,
    EuropeanSpanish,
    MexicanSpanish,
    Estonian,
    Basque,
    Irish,
    Galician,
    Guarani,
    Konkani,
    Gujarati,
    Hausa,
    Croatian,
    HaitianCreole,
    Armenian,
    Igbo,
    Icelandic,
    Javanese,
    Georgian,
    Kazakh,
    NorthernKurdish,
    Kannada,
    Kyrgyz,
    Latin,
    Luxembourgish,
    Lombard,
    Lingala,
    Lao,
    Lithuanian,
    Latvian,
    Maithili,
    Malagasy,
    Maori,
    Macedonian,
    Malayalam,
    Mongolian,
    Marathi,
    Maltese,
    Burmese,
    Nepali,
    Norwegian,
    Occitan,
    Oromo,
    Punjabi,
    Pangasinan,
    Pampanga,
    Dari,
    Pashto,
    BrazilianPortuguese,
    EuropeanPortuguese,
    Quechua,
    Romanian,
    Kinyarwanda,
    Sanskrit,
    Sicilian,
    Sindhi,
    Sinhala,
    Slovak,
    Slovenian,
    Albanian,
    Serbian,
    SouthernSotho,
    Sundanese,
    Swahili,
    Tamil,
    Telugu,
    Tajik,
    Turkmen,
    Tswana,
    Tsonga,
    Tatar,
    Ukrainian,
    Uzbek,
    Wolof,
    Xhosa,
    Yiddish,
    Cantonese,
    ChineseEnglishMixed,
    Zulu,
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
    /// Serializable language vocabulary; each provider applies its own allowlist.
    pub const ALL: [Self; 137] = [
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
        Self::Shanghainese,
        Self::Asturian,
        Self::Acehnese,
        Self::Afrikaans,
        Self::Akan,
        Self::Amharic,
        Self::Aragonese,
        Self::ArabicUnitedArabEmirates,
        Self::ArabicEgypt,
        Self::ArabicSaudiArabia,
        Self::Assamese,
        Self::Aymara,
        Self::Azerbaijani,
        Self::Bashkir,
        Self::Belarusian,
        Self::Bulgarian,
        Self::Bhojpuri,
        Self::Breton,
        Self::Bosnian,
        Self::Catalan,
        Self::Cebuano,
        Self::CentralKurdish,
        Self::Welsh,
        Self::Greek,
        Self::BritishEnglish,
        Self::AmericanEnglish,
        Self::Esperanto,
        Self::LatinAmericanSpanish,
        Self::EuropeanSpanish,
        Self::MexicanSpanish,
        Self::Estonian,
        Self::Basque,
        Self::Irish,
        Self::Galician,
        Self::Guarani,
        Self::Konkani,
        Self::Gujarati,
        Self::Hausa,
        Self::Croatian,
        Self::HaitianCreole,
        Self::Armenian,
        Self::Igbo,
        Self::Icelandic,
        Self::Javanese,
        Self::Georgian,
        Self::Kazakh,
        Self::NorthernKurdish,
        Self::Kannada,
        Self::Kyrgyz,
        Self::Latin,
        Self::Luxembourgish,
        Self::Lombard,
        Self::Lingala,
        Self::Lao,
        Self::Lithuanian,
        Self::Latvian,
        Self::Maithili,
        Self::Malagasy,
        Self::Maori,
        Self::Macedonian,
        Self::Malayalam,
        Self::Mongolian,
        Self::Marathi,
        Self::Maltese,
        Self::Burmese,
        Self::Nepali,
        Self::Norwegian,
        Self::Occitan,
        Self::Oromo,
        Self::Punjabi,
        Self::Pangasinan,
        Self::Pampanga,
        Self::Dari,
        Self::Pashto,
        Self::BrazilianPortuguese,
        Self::EuropeanPortuguese,
        Self::Quechua,
        Self::Romanian,
        Self::Kinyarwanda,
        Self::Sanskrit,
        Self::Sicilian,
        Self::Sindhi,
        Self::Sinhala,
        Self::Slovak,
        Self::Slovenian,
        Self::Albanian,
        Self::Serbian,
        Self::SouthernSotho,
        Self::Sundanese,
        Self::Swahili,
        Self::Tamil,
        Self::Telugu,
        Self::Tajik,
        Self::Turkmen,
        Self::Tswana,
        Self::Tsonga,
        Self::Tatar,
        Self::Ukrainian,
        Self::Uzbek,
        Self::Wolof,
        Self::Xhosa,
        Self::Yiddish,
        Self::Cantonese,
        Self::ChineseEnglishMixed,
        Self::Zulu,
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
            Self::Shanghainese => "wuu",
            Self::Asturian => "ast",
            Self::Acehnese => "ace",
            Self::Afrikaans => "af",
            Self::Akan => "ak",
            Self::Amharic => "am",
            Self::Aragonese => "an",
            Self::ArabicUnitedArabEmirates => "ar-AE",
            Self::ArabicEgypt => "ar-EG",
            Self::ArabicSaudiArabia => "ar-SA",
            Self::Assamese => "as",
            Self::Aymara => "ay",
            Self::Azerbaijani => "az",
            Self::Bashkir => "ba",
            Self::Belarusian => "be",
            Self::Bulgarian => "bg",
            Self::Bhojpuri => "bho",
            Self::Breton => "br",
            Self::Bosnian => "bs",
            Self::Catalan => "ca",
            Self::Cebuano => "ceb",
            Self::CentralKurdish => "ckb",
            Self::Welsh => "cy",
            Self::Greek => "el",
            Self::BritishEnglish => "en-GB",
            Self::AmericanEnglish => "en-US",
            Self::Esperanto => "eo",
            Self::LatinAmericanSpanish => "es-419",
            Self::EuropeanSpanish => "es-ES",
            Self::MexicanSpanish => "es-MX",
            Self::Estonian => "et",
            Self::Basque => "eu",
            Self::Irish => "ga",
            Self::Galician => "gl",
            Self::Guarani => "gn",
            Self::Konkani => "gom",
            Self::Gujarati => "gu",
            Self::Hausa => "ha",
            Self::Croatian => "hr",
            Self::HaitianCreole => "ht",
            Self::Armenian => "hy",
            Self::Igbo => "ig",
            Self::Icelandic => "is",
            Self::Javanese => "jv",
            Self::Georgian => "ka",
            Self::Kazakh => "kk",
            Self::NorthernKurdish => "kmr",
            Self::Kannada => "kn",
            Self::Kyrgyz => "ky",
            Self::Latin => "la",
            Self::Luxembourgish => "lb",
            Self::Lombard => "lmo",
            Self::Lingala => "ln",
            Self::Lao => "lo",
            Self::Lithuanian => "lt",
            Self::Latvian => "lv",
            Self::Maithili => "mai",
            Self::Malagasy => "mg",
            Self::Maori => "mi",
            Self::Macedonian => "mk",
            Self::Malayalam => "ml",
            Self::Mongolian => "mn",
            Self::Marathi => "mr",
            Self::Maltese => "mt",
            Self::Burmese => "my",
            Self::Nepali => "ne",
            Self::Norwegian => "no",
            Self::Occitan => "oc",
            Self::Oromo => "om",
            Self::Punjabi => "pa",
            Self::Pangasinan => "pag",
            Self::Pampanga => "pam",
            Self::Dari => "prs",
            Self::Pashto => "ps",
            Self::BrazilianPortuguese => "pt-BR",
            Self::EuropeanPortuguese => "pt-PT",
            Self::Quechua => "qu",
            Self::Romanian => "ro",
            Self::Kinyarwanda => "rw",
            Self::Sanskrit => "sa",
            Self::Sicilian => "scn",
            Self::Sindhi => "sd",
            Self::Sinhala => "si",
            Self::Slovak => "sk",
            Self::Slovenian => "sl",
            Self::Albanian => "sq",
            Self::Serbian => "sr",
            Self::SouthernSotho => "st",
            Self::Sundanese => "su",
            Self::Swahili => "sw",
            Self::Tamil => "ta",
            Self::Telugu => "te",
            Self::Tajik => "tg",
            Self::Turkmen => "tk",
            Self::Tswana => "tn",
            Self::Tsonga => "ts",
            Self::Tatar => "tt",
            Self::Ukrainian => "uk",
            Self::Uzbek => "uz",
            Self::Wolof => "wo",
            Self::Xhosa => "xh",
            Self::Yiddish => "yi",
            Self::Cantonese => "yue",
            Self::ChineseEnglishMixed => "zh_en",
            Self::Zulu => "zu",
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
            Self::Shanghainese => "Shanghainese",
            Self::Asturian => "Asturian",
            Self::Acehnese => "Acehnese",
            Self::Afrikaans => "Afrikaans",
            Self::Akan => "Akan",
            Self::Amharic => "Amharic",
            Self::Aragonese => "Aragonese",
            Self::ArabicUnitedArabEmirates => "Arabic (United Arab Emirates)",
            Self::ArabicEgypt => "Arabic (Egypt)",
            Self::ArabicSaudiArabia => "Arabic (Saudi Arabia)",
            Self::Assamese => "Assamese",
            Self::Aymara => "Aymara",
            Self::Azerbaijani => "Azerbaijani",
            Self::Bashkir => "Bashkir",
            Self::Belarusian => "Belarusian",
            Self::Bulgarian => "Bulgarian",
            Self::Bhojpuri => "Bhojpuri",
            Self::Breton => "Breton",
            Self::Bosnian => "Bosnian",
            Self::Catalan => "Catalan",
            Self::Cebuano => "Cebuano",
            Self::CentralKurdish => "Central Kurdish",
            Self::Welsh => "Welsh",
            Self::Greek => "Greek",
            Self::BritishEnglish => "British English",
            Self::AmericanEnglish => "American English",
            Self::Esperanto => "Esperanto",
            Self::LatinAmericanSpanish => "Latin American Spanish",
            Self::EuropeanSpanish => "European Spanish",
            Self::MexicanSpanish => "Mexican Spanish",
            Self::Estonian => "Estonian",
            Self::Basque => "Basque",
            Self::Irish => "Irish",
            Self::Galician => "Galician",
            Self::Guarani => "Guarani",
            Self::Konkani => "Konkani",
            Self::Gujarati => "Gujarati",
            Self::Hausa => "Hausa",
            Self::Croatian => "Croatian",
            Self::HaitianCreole => "Haitian Creole",
            Self::Armenian => "Armenian",
            Self::Igbo => "Igbo",
            Self::Icelandic => "Icelandic",
            Self::Javanese => "Javanese",
            Self::Georgian => "Georgian",
            Self::Kazakh => "Kazakh",
            Self::NorthernKurdish => "Northern Kurdish",
            Self::Kannada => "Kannada",
            Self::Kyrgyz => "Kyrgyz",
            Self::Latin => "Latin",
            Self::Luxembourgish => "Luxembourgish",
            Self::Lombard => "Lombard",
            Self::Lingala => "Lingala",
            Self::Lao => "Lao",
            Self::Lithuanian => "Lithuanian",
            Self::Latvian => "Latvian",
            Self::Maithili => "Maithili",
            Self::Malagasy => "Malagasy",
            Self::Maori => "Māori",
            Self::Macedonian => "Macedonian",
            Self::Malayalam => "Malayalam",
            Self::Mongolian => "Mongolian",
            Self::Marathi => "Marathi",
            Self::Maltese => "Maltese",
            Self::Burmese => "Burmese",
            Self::Nepali => "Nepali",
            Self::Norwegian => "Norwegian",
            Self::Occitan => "Occitan",
            Self::Oromo => "Oromo",
            Self::Punjabi => "Punjabi",
            Self::Pangasinan => "Pangasinan",
            Self::Pampanga => "Pampanga",
            Self::Dari => "Dari",
            Self::Pashto => "Pashto",
            Self::BrazilianPortuguese => "Brazilian Portuguese",
            Self::EuropeanPortuguese => "European Portuguese",
            Self::Quechua => "Quechua",
            Self::Romanian => "Romanian",
            Self::Kinyarwanda => "Kinyarwanda",
            Self::Sanskrit => "Sanskrit",
            Self::Sicilian => "Sicilian",
            Self::Sindhi => "Sindhi",
            Self::Sinhala => "Sinhala",
            Self::Slovak => "Slovak",
            Self::Slovenian => "Slovenian",
            Self::Albanian => "Albanian",
            Self::Serbian => "Serbian",
            Self::SouthernSotho => "Southern Sotho",
            Self::Sundanese => "Sundanese",
            Self::Swahili => "Swahili",
            Self::Tamil => "Tamil",
            Self::Telugu => "Telugu",
            Self::Tajik => "Tajik",
            Self::Turkmen => "Turkmen",
            Self::Tswana => "Tswana",
            Self::Tsonga => "Tsonga",
            Self::Tatar => "Tatar",
            Self::Ukrainian => "Ukrainian",
            Self::Uzbek => "Uzbek",
            Self::Wolof => "Wolof",
            Self::Xhosa => "Xhosa",
            Self::Yiddish => "Yiddish",
            Self::Cantonese => "Cantonese",
            Self::ChineseEnglishMixed => "Chinese and English (mixed)",
            Self::Zulu => "Zulu",
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
            Self::Original | Self::ChineseEnglishMixed => false,
            Self::SimplifiedChinese => matches!(reported.as_str(), "zh" | "zh-cn" | "zh-hans"),
            Self::English => reported == "en",
            Self::TraditionalChinese => matches!(reported.as_str(), "zh_tw" | "zh-tw" | "zh-hant"),
            _ => reported.eq_ignore_ascii_case(self.raw_value()),
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
        assert_eq!(SourceLanguage::ALL.len(), 137);
        assert_eq!(TargetLanguage::ALL.len(), 137);
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
        assert!(serde_json::from_value::<SourceLanguage>(serde_json::json!("xx")).is_err());
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
                        && target != TargetLanguage::ChineseEnglishMixed
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
    fn region_and_script_codes_remain_distinct_in_detected_languages() {
        for code in ["ar-EG", "pt-BR", "pt-PT", "es-MX", "zh_en", "zh_tw"] {
            let source = SourceLanguage::ALL
                .into_iter()
                .find(|source| source.raw_value() == code)
                .unwrap();
            assert_eq!(
                SourceLanguage::from_detected(Some(&code.to_ascii_lowercase())),
                Some(source)
            );
        }
        assert_eq!(
            SourceLanguage::from_detected(Some("zh-Hant")),
            Some(SourceLanguage::TraditionalChinese)
        );
        assert_eq!(
            SourceLanguage::from_detected(Some("fr-FR")),
            Some(SourceLanguage::French)
        );
        let brazil = TargetLanguage::ALL
            .into_iter()
            .find(|target| target.raw_value() == "pt-BR")
            .unwrap();
        assert!(brazil.matches_reported_asr(Some("pt-br")));
        assert!(!brazil.matches_reported_asr(Some("pt-PT")));
        assert!(!brazil.matches_reported_asr(Some("pt")));
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
