//! Exact independent translator catalogs; kept in sync with shared language fixtures.

pub const DEEPL_SOURCE_CODES: &[&str] = &[
    "ace", "af", "an", "ar", "as", "ay", "az", "ba", "be", "bg", "bho", "bn", "br", "bs", "ca",
    "ceb", "ckb", "cs", "cy", "da", "de", "el", "eo", "es", "et", "eu", "fa", "fi", "fr", "ga",
    "gl", "gn", "gom", "gu", "ha", "he", "hi", "hr", "ht", "hu", "hy", "id", "ig", "is", "it",
    "ja", "jv", "ka", "kk", "kmr", "ko", "ky", "la", "lb", "lmo", "ln", "lt", "lv", "mai", "mg",
    "mi", "mk", "ml", "mn", "mr", "ms", "mt", "my", "no", "ne", "nl", "oc", "om", "pa", "pag",
    "pam", "pl", "prs", "ps", "qu", "ro", "ru", "sa", "scn", "sk", "sl", "sq", "sr", "st", "su",
    "sv", "sw", "ta", "te", "tg", "th", "tk", "tl", "tn", "tr", "ts", "tt", "uk", "ur", "uz", "vi",
    "wo", "xh", "yi", "yue", "zh", "zu", "en", "pt",
];

pub const DEEPL_TARGET_CODES: &[&str] = &[
    "ace", "af", "an", "ar", "as", "ay", "az", "ba", "be", "bg", "bho", "bn", "br", "bs", "ca",
    "ceb", "ckb", "cs", "cy", "da", "de", "el", "eo", "es", "et", "eu", "fa", "fi", "fr", "ga",
    "gl", "gn", "gom", "gu", "ha", "he", "hi", "hr", "ht", "hu", "hy", "id", "ig", "is", "it",
    "ja", "jv", "ka", "kk", "kmr", "ko", "ky", "la", "lb", "lmo", "ln", "lt", "lv", "mai", "mg",
    "mi", "mk", "ml", "mn", "mr", "ms", "mt", "my", "no", "ne", "nl", "oc", "om", "pa", "pag",
    "pam", "pl", "prs", "ps", "qu", "ro", "ru", "sa", "scn", "sk", "sl", "sq", "sr", "st", "su",
    "sv", "sw", "ta", "te", "tg", "th", "tk", "tl", "tn", "tr", "ts", "tt", "uk", "ur", "uz", "vi",
    "wo", "xh", "yi", "yue", "zh", "zu", "en-GB", "en-US", "es-419", "pt-BR", "pt-PT", "zh_tw",
    "en", "pt",
];

pub const DEEPLX_SOURCE_CODES: &[&str] = &[
    "ar", "bg", "cs", "da", "de", "el", "en", "es", "et", "fi", "fr", "he", "hu", "id", "it", "ja",
    "ko", "lt", "lv", "no", "nl", "pl", "pt", "ro", "ru", "sk", "sl", "sv", "tr", "uk", "vi", "zh",
];

pub const DEEPLX_TARGET_CODES: &[&str] = &[
    "ar", "bg", "cs", "da", "de", "el", "en", "en-GB", "en-US", "es", "es-419", "et", "fi", "fr",
    "he", "hu", "id", "it", "ja", "ko", "lt", "lv", "no", "nl", "pl", "pt", "pt-BR", "pt-PT", "ro",
    "ru", "sk", "sl", "sv", "tr", "uk", "vi", "zh", "zh_tw",
];

pub fn wire_code(code: &str) -> String {
    match code {
        "no" => "NB".into(),
        "zh_tw" => "ZH-HANT".into(),
        _ => code.to_ascii_uppercase(),
    }
}
