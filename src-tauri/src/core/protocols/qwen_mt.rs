//! Qwen-MT chat-completions protocol, domain prompts, and filler glossaries.
//! The domain hints and filler tables are provider wire assets that directly
//! affect translation quality and must not be casually reworded.

use crate::core::models::{SourceLanguage, TargetLanguage};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;
use thiserror::Error;

/// Content-free classification of explicitly documented upstream error codes.
/// Never infer billing or rate-limit types from free-form server messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QwenMTRejectionCategory {
    RequestRate,
    BurstRate,
    TokenRate,
    Concurrency,
    Capacity,
    Billing,
    Unknown,
}

impl QwenMTRejectionCategory {
    pub fn from_response(data: &[u8]) -> Self {
        // Match the HTTP reader's existing bound, also protecting standalone
        // callers. Borrow only code strings; ignore messages/content entirely.
        if data.len() > 1024 * 1024 {
            return Self::Unknown;
        }
        #[derive(Deserialize)]
        struct ErrorCodes<'a> {
            #[serde(borrow)]
            error: Option<ErrorCode<'a>>,
            code: Option<&'a str>,
        }
        #[derive(Deserialize)]
        struct ErrorCode<'a> {
            code: Option<&'a str>,
        }
        let Ok(body) = serde_json::from_slice::<ErrorCodes<'_>>(data) else {
            return Self::Unknown;
        };
        match body.error.and_then(|error| error.code).or(body.code) {
            Some(
                "Throttling.RateQuota" | "LimitRequests" | "limit_requests" | "ResourceExhausted",
            ) => Self::RequestRate,
            Some("Throttling.BurstRate" | "limit_burst_rate") => Self::BurstRate,
            Some("Throttling.AllocationQuota" | "insufficient_quota") => Self::TokenRate,
            Some("Throttling.Concurrency") => Self::Concurrency,
            Some(
                "Throttling.ServiceOverloaded"
                | "ServiceOverloaded"
                | "Throttling.ResourceExhausted",
            ) => Self::Capacity,
            Some(
                "CommodityNotPurchased"
                | "PrepaidBillOverdue"
                | "PostpaidBillOverdue"
                | "BudgetLimitExceeded",
            ) => Self::Billing,
            _ => Self::Unknown,
        }
    }

    /// Fixed allowlist output suitable for diagnostics; no upstream strings.
    pub fn diagnostic_label(self) -> &'static str {
        match self {
            Self::RequestRate => "request_rate",
            Self::BurstRate => "burst_rate",
            Self::TokenRate => "token_rate",
            Self::Concurrency => "concurrency",
            Self::Capacity => "capacity",
            Self::Billing => "billing",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum QwenMTProtocolError {
    #[error("The Qwen-MT endpoint could not be created.")]
    InvalidEndpoint,
    #[error("Qwen-MT returned an invalid response.")]
    InvalidJSON,
    #[error("Qwen-MT returned no translated text.")]
    MissingTranslation,
    #[error("The selected language is not supported by the Qwen-MT model.")]
    UnsupportedLanguage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QwenMTClientError {
    Apple(&'static str),
    DeepLX(super::deeplx::DeepLXError),
    DeepL(super::deepl::DeepLError),
    OpenAICompatible(super::openai_compatible::OpenAICompatibleError),
    MissingAPIKey,
    MissingTextTranslation,
    UnsupportedSource,
    InvalidHTTPResponse,
    ResponseTooLarge,
    RequestTimedOut,
    RequestFailed { status_code: u16, message: String },
}

impl std::fmt::Display for QwenMTClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Apple(label) => write!(f, "{label}"),
            Self::DeepLX(error) => write!(f, "{error}"),
            Self::DeepL(error) => write!(f, "{error}"),
            Self::OpenAICompatible(error) => write!(f, "{error}"),
            Self::MissingAPIKey => {
                write!(f, "Add an Alibaba Cloud Model Studio API key in Settings.")
            }
            Self::MissingTextTranslation => {
                write!(f, "Configure a text translation service in Settings.")
            }
            Self::UnsupportedSource => write!(f, "translation_source_unsupported"),
            Self::InvalidHTTPResponse => write!(f, "Qwen-MT returned an invalid HTTP response."),
            Self::ResponseTooLarge => write!(f, "Qwen-MT returned a response that is too large."),
            Self::RequestTimedOut => write!(f, "Qwen-MT took too long to respond."),
            Self::RequestFailed {
                status_code,
                message,
            } => {
                if message.is_empty() {
                    write!(f, "Qwen-MT request failed with HTTP {status_code}.")
                } else {
                    write!(f, "{message}")
                }
            }
        }
    }
}

impl std::error::Error for QwenMTClientError {}

impl QwenMTClientError {
    pub fn retry_class(&self) -> mimi_core::translation_policy::RetryClass {
        use mimi_core::translation_policy::{classify_http, RetryClass};
        match self {
            Self::RequestTimedOut | Self::InvalidHTTPResponse => RetryClass::Temporary,
            Self::Apple("apple_translation_busy" | "apple_translation_timeout") => {
                RetryClass::Temporary
            }
            Self::RequestFailed { status_code, .. } => classify_http(*status_code),
            Self::DeepL(error) if error.retryable() => match error {
                super::deepl::DeepLError::Rejected(code) => classify_http(*code),
                _ => RetryClass::Temporary,
            },
            Self::DeepLX(error) if error.retryable() => match error {
                super::deeplx::DeepLXError::Rejected(code) => classify_http(*code),
                _ => RetryClass::Temporary,
            },
            Self::OpenAICompatible(error) if error.retryable() => match error {
                super::openai_compatible::OpenAICompatibleError::Rejected(code) => {
                    classify_http(*code)
                }
                _ => RetryClass::Temporary,
            },
            _ => RetryClass::Permanent,
        }
    }

    pub fn recovery_reason(&self) -> Option<crate::core::diagnostics::TranslationRecoveryReason> {
        use crate::core::diagnostics::TranslationRecoveryReason;
        use mimi_core::translation_policy::RetryClass;
        match self.retry_class() {
            RetryClass::Permanent => None,
            RetryClass::Temporary => Some(TranslationRecoveryReason::TemporarilyUnavailable),
            RetryClass::RateLimited => Some(TranslationRecoveryReason::RateLimited),
        }
    }

    pub fn is_authentication_failure(&self) -> bool {
        match self {
            Self::DeepLX(error) => error.authentication_failure(),
            Self::DeepL(error) => error.authentication_failure(),
            Self::OpenAICompatible(error) => error.authentication_failure(),
            Self::RequestFailed { status_code, .. } => *status_code == 401 || *status_code == 403,
            Self::MissingAPIKey => true,
            Self::Apple(_)
            | Self::MissingTextTranslation
            | Self::UnsupportedSource
            | Self::InvalidHTTPResponse
            | Self::ResponseTooLarge
            | Self::RequestTimedOut => false,
        }
    }

    /// Content-free diagnostic label (never includes the server message).
    pub fn diagnostic_label(&self) -> String {
        match self {
            Self::Apple(label) => (*label).into(),
            Self::DeepLX(error) => error.diagnostic_label(),
            Self::DeepL(error) => error.diagnostic_label(),
            Self::OpenAICompatible(error) => error.diagnostic_label(),
            Self::MissingAPIKey => "QwenMTClientError.missingAPIKey".to_string(),
            Self::MissingTextTranslation => {
                "TranslationClientError.missingTextTranslation".to_string()
            }
            Self::UnsupportedSource => "QwenMTClientError.unsupportedSource".to_string(),
            Self::InvalidHTTPResponse => "QwenMTClientError.invalidHTTPResponse".to_string(),
            Self::ResponseTooLarge => "QwenMTClientError.responseTooLarge".to_string(),
            Self::RequestTimedOut => "QwenMTClientError.requestTimedOut".to_string(),
            Self::RequestFailed { status_code, .. } => {
                format!("QwenMTClientError.requestFailed(status={status_code})")
            }
        }
    }
}

pub enum QwenMTRetryPolicy {}

impl QwenMTRetryPolicy {
    /// Shared bounded backoff; preserve this typed protocol adapter API.
    pub fn delay(error: &QwenMTClientError, attempt: usize) -> Option<Duration> {
        mimi_core::translation_policy::retry_delay_ms(error.retry_class(), attempt)
            .map(Duration::from_millis)
    }
}

#[derive(Clone)]
pub struct QwenMTEndpoint {
    pub url: url::Url,
}

impl QwenMTEndpoint {
    /// DashScope unified OpenAI-compatible chat-completions endpoint. The old
    /// MaaS host embedded the workspace id; the unified host authenticates
    /// with `Authorization: Bearer <key>` alone.
    pub const URL: &'static str =
        "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions";

    pub fn new() -> Result<Self, QwenMTProtocolError> {
        Ok(Self {
            url: url::Url::parse(Self::URL).map_err(|_| QwenMTProtocolError::InvalidEndpoint)?,
        })
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QwenMTModel {
    #[default]
    Lite,
    Flash,
    Plus,
}

/// Shared default for live subtitles and the selected-profile ready probe.
pub const REALTIME_MT_MODEL: QwenMTModel = QwenMTModel::Lite;

/// Exact upstream tables; Audio 3.0's 30 inputs are not the Lite MT table.
/// https://help.aliyun.com/en/model-studio/machine-translation#supported-languages
pub const QWEN_MT_LITE_LANGUAGE_CODES: &[&str] = &[
    "en", "zh", "zh_tw", "ru", "ja", "ko", "es", "fr", "pt", "de", "it", "th", "vi", "id", "ms",
    "ar", "hi", "he", "ur", "bn", "pl", "nl", "tr", "km", "cs", "sv", "hu", "da", "fi", "tl", "fa",
];

pub const QWEN_MT_FLASH_LANGUAGE_CODES: &[&str] = &[
    "en", "zh", "zh_tw", "ru", "ja", "ko", "es", "fr", "pt", "de", "it", "th", "vi", "id", "ms",
    "ar", "hi", "he", "my", "ta", "ur", "bn", "pl", "nl", "ro", "tr", "km", "lo", "yue", "cs",
    "el", "sv", "hu", "da", "fi", "uk", "bg", "sr", "te", "af", "hy", "as", "ast", "eu", "be",
    "bs", "ca", "ceb", "hr", "arz", "et", "gl", "ka", "gu", "is", "jv", "kn", "kk", "lv", "lt",
    "lb", "mk", "mai", "mt", "mr", "acm", "ary", "ars", "ne", "az", "apc", "uz", "nb", "nn", "oc",
    "or", "pag", "scn", "sd", "si", "sk", "sl", "ajp", "sw", "tl", "acq", "sq", "aeb", "vec",
    "war", "cy", "fa",
];

impl QwenMTModel {
    pub fn raw_name(self) -> &'static str {
        match self {
            Self::Lite => "qwen-mt-lite",
            Self::Flash => "qwen-mt-flash",
            Self::Plus => "qwen-mt-plus",
        }
    }

    pub fn supported_language_codes(self) -> &'static [&'static str] {
        match self {
            Self::Lite => QWEN_MT_LITE_LANGUAGE_CODES,
            Self::Flash | Self::Plus => QWEN_MT_FLASH_LANGUAGE_CODES,
        }
    }

    /// Source auto is a request mode, not a guarantee that every ASR language
    /// is translatable. A concrete report must fit the selected MT model.
    pub fn supports_reported_source(self, reported: Option<&str>) -> bool {
        let Some(reported) = reported else {
            return true;
        };
        let normalized = reported.trim().to_ascii_lowercase();
        let code = match normalized.as_str() {
            "zh-cn" | "zh-hans" | "chinese" | "mandarin" => "zh",
            "zh-tw" | "zh-hant" | "traditional chinese" => "zh_tw",
            "english" => "en",
            "japanese" => "ja",
            "korean" => "ko",
            "fil" | "filipino" => "tl",
            "no" => "nb",
            _ => normalized.split('-').next().unwrap_or(&normalized),
        };
        self.supported_language_codes().contains(&code)
            // An absent/unrecognized report retains the existing auto path;
            // only a known service code can establish an unsupported source.
            || !QWEN_MT_FLASH_LANGUAGE_CODES.contains(&code)
    }
}

fn source_lang_name(language: SourceLanguage) -> &'static str {
    match language {
        SourceLanguage::Automatic => "auto",
        SourceLanguage::Chinese => "Chinese",
        SourceLanguage::English => "English",
        SourceLanguage::Japanese => "Japanese",
        SourceLanguage::Korean => "Korean",
        SourceLanguage::Vietnamese => "Vietnamese",
        SourceLanguage::Thai => "Thai",
        SourceLanguage::Indonesian => "Indonesian",
        SourceLanguage::Malay => "Malay",
        SourceLanguage::Filipino => "Tagalog",
        SourceLanguage::Hindi => "Hindi",
        SourceLanguage::Arabic => "Arabic",
        SourceLanguage::French => "French",
        SourceLanguage::German => "German",
        SourceLanguage::Spanish => "Spanish",
        SourceLanguage::Portuguese => "Portuguese",
        SourceLanguage::Russian => "Russian",
        SourceLanguage::Italian => "Italian",
        SourceLanguage::Dutch => "Dutch",
        SourceLanguage::Swedish => "Swedish",
        SourceLanguage::Danish => "Danish",
        SourceLanguage::Finnish => "Finnish",
        SourceLanguage::Norwegian => "Norwegian Bokmål",
        SourceLanguage::Greek => "Greek",
        SourceLanguage::Polish => "Polish",
        SourceLanguage::Czech => "Czech",
        SourceLanguage::Hungarian => "Hungarian",
        SourceLanguage::Romanian => "Romanian",
        SourceLanguage::Bulgarian => "Bulgarian",
        SourceLanguage::Croatian => "Croatian",
        SourceLanguage::Slovak => "Slovak",
        _ => TargetLanguage::ALL
            .into_iter()
            .find(|target| target.raw_value() == language.raw_value())
            .map(TargetLanguage::qwen_mt_name)
            .unwrap_or(language.raw_value()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QwenMTTerm {
    pub source: String,
    pub target: String,
}

impl QwenMTTerm {
    pub fn new(source: &str, target: &str) -> Self {
        Self {
            source: source.to_string(),
            target: target.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QwenMTMemoryPair {
    pub source: String,
    pub target: String,
}

pub enum QwenMTRequestEncoder {}

impl QwenMTRequestEncoder {
    #[allow(clippy::too_many_arguments)]
    pub fn request(
        text: &str,
        source_language: SourceLanguage,
        target_language: TargetLanguage,
        model: QwenMTModel,
        stream: bool,
        domain_hint: Option<&str>,
        terms: &[QwenMTTerm],
        translation_memory: &[QwenMTMemoryPair],
    ) -> Result<Value, QwenMTProtocolError> {
        if !model
            .supported_language_codes()
            .contains(&target_language.raw_value())
            || (source_language != SourceLanguage::Automatic
                && !model
                    .supported_language_codes()
                    .contains(&match source_language {
                        SourceLanguage::Norwegian => "nb",
                        _ => source_language.raw_value(),
                    }))
        {
            return Err(QwenMTProtocolError::UnsupportedLanguage);
        }
        let mut options = json!({
            "source_lang": source_lang_name(source_language),
            "target_lang": target_language.qwen_mt_name(),
        });
        if let Some(domain_hint) = domain_hint.filter(|_| model != QwenMTModel::Lite) {
            options["domains"] = json!(domain_hint);
        }
        if !terms.is_empty() {
            options["terms"] = json!(terms);
        }
        if !translation_memory.is_empty() {
            options["tm_list"] = json!(translation_memory);
        }
        Ok(json!({
            "model": model.raw_name(),
            "messages": [{ "role": "user", "content": text }],
            "stream": stream,
            "translation_options": options
        }))
    }
}

pub enum QwenMTResponseDecoder {}

impl QwenMTResponseDecoder {
    pub fn decode(text: &str) -> Result<String, QwenMTProtocolError> {
        #[derive(Deserialize)]
        struct Response {
            choices: Vec<Choice>,
        }
        #[derive(Deserialize)]
        struct Choice {
            message: Message,
        }
        #[derive(Deserialize)]
        struct Message {
            content: String,
        }

        let response: Response =
            serde_json::from_str(text).map_err(|_| QwenMTProtocolError::InvalidJSON)?;
        let content = response
            .choices
            .first()
            .map(|c| c.message.content.trim())
            .filter(|c| !c.is_empty())
            .ok_or(QwenMTProtocolError::MissingTranslation)?;
        Ok(content.to_string())
    }
}

pub enum QwenMTStreamDecoder {}

impl QwenMTStreamDecoder {
    pub fn decode_chunk(text: &str) -> Result<Option<String>, QwenMTProtocolError> {
        #[derive(Deserialize)]
        struct Response {
            choices: Vec<Choice>,
        }
        #[derive(Deserialize)]
        struct Choice {
            delta: Delta,
        }
        #[derive(Deserialize)]
        struct Delta {
            content: Option<String>,
        }

        let response: Response =
            serde_json::from_str(text).map_err(|_| QwenMTProtocolError::InvalidJSON)?;
        Ok(response
            .choices
            .first()
            .and_then(|c| c.delta.content.clone()))
    }
}

/// Provider-facing domain prompts and filler glossaries for spoken dialogue.
pub enum QwenMTDomainHint {}

impl QwenMTDomainHint {
    pub fn spoken_dialogue(
        source_language: SourceLanguage,
        target_language: TargetLanguage,
    ) -> String {
        let language_guidance = match target_language {
            TargetLanguage::Original => "",
            TargetLanguage::SimplifiedChinese => {
                "Use concise, idiomatic Simplified Chinese, like subtitles for a TV \
                 drama, and keep every natural particle: 嗯、啊、呢、吧、嘛、哦、唉. \
                 Render Japanese fillers (えっと、あの、うーん、あぁ) and English \
                 fillers (um, uh, oh, hmm) with their natural Chinese equivalents; \
                 never drop a meaningful filler."
            }
            TargetLanguage::English => {
                "Use concise, idiomatic conversational English with natural \
                 contractions and interjections (um, uh, oh, hmm, yeah). Keep \
                 Japanese fillers (えっと、あの、うーん、あぁ) as natural English \
                 equivalents; never drop a meaningful filler."
            }
            TargetLanguage::Japanese => {
                "Use natural conversational Japanese with appropriate register, \
                 particles, and sentence endings (ね、よ、な、じゃん). Keep Chinese \
                 and English fillers as natural Japanese equivalents; never drop a \
                 meaningful filler."
            }
            _ => "",
        };
        let source_guidance = source_guidance(source_language, target_language);
        format!(
            "Natural spoken dialogue for watching TV dramas and films. {language_guidance} \
             {source_guidance} Preserve the speaker's tone, emotion, and implied subjects \
             from context. The translation memory (tm_list) is the ongoing dialogue: \
             each pair is one finished subtitle line and its translation. Use it as \
             context — keep names, pronouns, and implied subjects consistent with \
             earlier lines, keep the same speaker's tone and register, and resolve \
             ambiguous or truncated phrases from what came before. Never repeat or \
             re-translate anything already in the memory; translate only the new input. \
             Render polite or formal Japanese (です/ます、敬語) as naturally courteous \
             Chinese, never stiff or literal. Avoid literal, explanatory, or translation-like \
             wording. Prefer short, complete sentences that fit one subtitle line. \
             Faithfully preserve every vocalization represented in the source, including \
             interjections, hesitation, breaths, gasps, moans, and cries, rendered \
             naturally in the target language. Do not sanitize, euphemize, censor, or \
             omit explicit dialogue or vocal sounds. Keep deliberate repetition for \
             emphasis; collapse only clear accidental ASR repetition. Output only the \
             translation text."
        )
    }

    /// Returns a glossary that forces common Japanese, English, and Korean
    /// fillers to keep a natural counterpart in the target language.
    pub fn filler_terms(
        source_language: SourceLanguage,
        target_language: TargetLanguage,
    ) -> Vec<QwenMTTerm> {
        let sources: &[SourceLanguage] = if source_language == SourceLanguage::Automatic {
            &[
                SourceLanguage::Japanese,
                SourceLanguage::English,
                SourceLanguage::Korean,
            ]
        } else {
            std::slice::from_ref(&source_language)
        };
        sources
            .iter()
            .flat_map(|source| filler_terms_for(source, target_language))
            .collect()
    }
}

fn filler_terms_for(source: &SourceLanguage, target: TargetLanguage) -> Vec<QwenMTTerm> {
    use SourceLanguage as S;
    use TargetLanguage as T;

    fn terms(pairs: &[(&str, &str)]) -> Vec<QwenMTTerm> {
        pairs
            .iter()
            .map(|(source, target)| QwenMTTerm::new(source, target))
            .collect()
    }

    match (source, target) {
        (S::Japanese, T::SimplifiedChinese) => terms(&[
            ("えっと", "那个"),
            ("えーと", "那个"),
            ("ええと", "那个"),
            ("あの", "那个"),
            ("あのー", "那个"),
            ("あのう", "那个"),
            ("うーん", "嗯"),
            ("う〜ん", "嗯"),
            ("あぁ", "啊"),
            ("ああ", "啊"),
            ("あっ", "啊"),
            ("えっ", "诶"),
            ("ふふ", "呵呵"),
            ("うふふ", "嘿嘿"),
            ("まあ", "嘛"),
            ("ねえ", "那个"),
            ("あら", "哎呀"),
            ("おや", "哎呀"),
            ("うわ", "哇"),
            ("きゃっ", "呀"),
            ("はぁ", "唉"),
            ("んー", "嗯"),
        ]),
        (S::English, T::SimplifiedChinese) => terms(&[
            ("um", "嗯"),
            ("uh", "呃"),
            ("oh", "哦"),
            ("hmm", "嗯"),
            ("ah", "啊"),
            ("wow", "哇"),
            ("hey", "喂"),
            ("yikes", "哎呀"),
        ]),
        (S::Korean, T::SimplifiedChinese) => terms(&[
            ("어", "嗯"),
            ("아", "啊"),
            ("음", "嗯"),
            ("어우", "哎哟"),
            ("헐", "不是吧"),
            ("야", "喂"),
        ]),
        (S::Japanese, T::English) => terms(&[
            ("えっと", "Um"),
            ("えーと", "Um"),
            ("ええと", "Um"),
            ("あの", "Um"),
            ("うーん", "Hmm"),
            ("う〜ん", "Hmm"),
            ("あぁ", "Ah"),
            ("あっ", "Oh"),
            ("えっ", "Huh"),
            ("ふふ", "Heh"),
            ("まあ", "Well"),
            ("ねえ", "Hey"),
            ("あら", "Oh"),
            ("うわ", "Wow"),
            ("きゃっ", "Eek"),
        ]),
        (S::English, T::Japanese) => terms(&[
            ("um", "うーん"),
            ("uh", "あの"),
            ("oh", "あっ"),
            ("hmm", "うーん"),
            ("ah", "ああ"),
            ("wow", "わあ"),
            ("hey", "ねえ"),
            ("yikes", "ひえっ"),
        ]),
        _ => Vec::new(),
    }
}

fn source_guidance(source: SourceLanguage, target: TargetLanguage) -> &'static str {
    use SourceLanguage as S;
    use TargetLanguage as T;

    match source {
        S::Japanese => match target {
            T::SimplifiedChinese => {
                "For every Japanese filler use its natural Chinese counterpart: \
                 えっと/あの→那个，うーん→嗯，あぁ→啊，まあ→嘛，ねえ→那个。 Sentence-final \
                 particles need a counterpart too: ね→呢/吧，よ→啊/哦，な→啊，じゃん→嘛。 \
                 Dropping a filler or particle is an error."
            }
            T::English => {
                "For every Japanese filler use its natural English counterpart: \
                 えっと/あの→Um，うーん→Hmm，あぁ→Ah，まあ→Well，ねえ→Hey。 Sentence-final \
                 particles need a counterpart too: ね→huh/right，よ→you know。 Dropping a \
                 filler or particle is an error."
            }
            _ => "",
        },
        S::English => match target {
            T::SimplifiedChinese => {
                "For every English filler use its natural Chinese counterpart: \
                 um→嗯，uh→呃，oh→哦，hmm→嗯，ah→啊，wow→哇。 Dropping a filler is an error."
            }
            T::Japanese => {
                "For every English filler use its natural Japanese counterpart: \
                 um→うーん，uh→あの，oh→あっ，hmm→うーん，wow→わあ。 Dropping a filler is an error."
            }
            _ => "",
        },
        S::Korean if target == T::SimplifiedChinese => {
            "For every Korean filler use its natural Chinese counterpart: 어→嗯，아→啊，음→嗯。 Dropping a filler is an error."
        }
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apple_local_failures_retry_only_transient_request_failures() {
        use mimi_core::translation_policy::RetryClass;
        for label in [
            "apple_translation_assets_missing",
            "apple_translation_language_unsupported",
            "apple_translation_unavailable",
            "apple_translation_cancelled",
        ] {
            let error = QwenMTClientError::Apple(label);
            assert_eq!(error.retry_class(), RetryClass::Permanent);
            assert!(!error.is_authentication_failure());
            assert_eq!(error.diagnostic_label(), label);
        }
        assert_eq!(
            QwenMTClientError::Apple("apple_translation_timeout").retry_class(),
            RetryClass::Temporary
        );
        assert_eq!(
            QwenMTClientError::Apple("apple_translation_busy").retry_class(),
            RetryClass::Temporary
        );
    }

    #[test]
    fn all_lite_targets_and_explicit_intersection_sources_encode_without_model_or_prompt_changes() {
        let names = [
            "English",
            "Chinese",
            "Traditional Chinese",
            "Russian",
            "Japanese",
            "Korean",
            "Spanish",
            "French",
            "Portuguese",
            "German",
            "Italian",
            "Thai",
            "Vietnamese",
            "Indonesian",
            "Malay",
            "Arabic",
            "Hindi",
            "Hebrew",
            "Urdu",
            "Bengali",
            "Polish",
            "Dutch",
            "Turkish",
            "Khmer",
            "Czech",
            "Swedish",
            "Hungarian",
            "Danish",
            "Finnish",
            "Tagalog",
            "Persian",
        ];
        // Independent upstream full-English-name oracle, in Audio3 hint order.
        // The MT API specifies names even though the overview also lists codes.
        let source_names = [
            "auto",
            "Chinese",
            "English",
            "Japanese",
            "Korean",
            "Vietnamese",
            "Thai",
            "Indonesian",
            "Malay",
            "Tagalog",
            "Hindi",
            "Arabic",
            "French",
            "German",
            "Spanish",
            "Portuguese",
            "Russian",
            "Italian",
            "Dutch",
            "Swedish",
            "Danish",
            "Finnish",
            "Norwegian Bokmål",
            "Greek",
            "Polish",
            "Czech",
            "Hungarian",
            "Romanian",
            "Bulgarian",
            "Croatian",
            "Slovak",
        ];
        for (code, name) in QWEN_MT_LITE_LANGUAGE_CODES.iter().zip(names) {
            let target = TargetLanguage::ALL
                .into_iter()
                .find(|target| target.raw_value() == *code)
                .unwrap();
            for (source, expected_source) in SourceLanguage::ALL.into_iter().zip(source_names) {
                let result = QwenMTRequestEncoder::request(
                    "Synthetic fixture.",
                    source,
                    target,
                    REALTIME_MT_MODEL,
                    true,
                    Some("existing domain"),
                    &[],
                    &[],
                );
                if source == SourceLanguage::Automatic
                    || QWEN_MT_LITE_LANGUAGE_CODES.contains(&source.raw_value())
                {
                    let request = result.unwrap();
                    assert_eq!(request["model"], "qwen-mt-lite");
                    assert_eq!(request["translation_options"]["target_lang"], name);
                    assert_eq!(
                        request["translation_options"]["source_lang"],
                        expected_source
                    );
                    assert!(request["translation_options"].get("domains").is_none());
                    assert!(request["translation_options"].get("tm_list").is_none());
                    assert_eq!(request["stream"], true);
                } else {
                    assert_eq!(
                        result.unwrap_err(),
                        QwenMTProtocolError::UnsupportedLanguage
                    );
                }
            }
        }
        for code in ["no", "el", "ro", "bg", "hr", "sk"] {
            assert!(!REALTIME_MT_MODEL.supports_reported_source(Some(code)));
        }
        assert!(REALTIME_MT_MODEL.supports_reported_source(None));
        assert!(REALTIME_MT_MODEL.supports_reported_source(Some("unknown")));
    }

    #[test]
    fn rejection_codes_classify_documented_aliases_in_both_response_shapes() {
        let cases = [
            (
                "Throttling.RateQuota",
                QwenMTRejectionCategory::RequestRate,
                "request_rate",
            ),
            (
                "LimitRequests",
                QwenMTRejectionCategory::RequestRate,
                "request_rate",
            ),
            (
                "limit_requests",
                QwenMTRejectionCategory::RequestRate,
                "request_rate",
            ),
            (
                "ResourceExhausted",
                QwenMTRejectionCategory::RequestRate,
                "request_rate",
            ),
            (
                "Throttling.BurstRate",
                QwenMTRejectionCategory::BurstRate,
                "burst_rate",
            ),
            (
                "limit_burst_rate",
                QwenMTRejectionCategory::BurstRate,
                "burst_rate",
            ),
            (
                "Throttling.AllocationQuota",
                QwenMTRejectionCategory::TokenRate,
                "token_rate",
            ),
            (
                "insufficient_quota",
                QwenMTRejectionCategory::TokenRate,
                "token_rate",
            ),
            (
                "Throttling.Concurrency",
                QwenMTRejectionCategory::Concurrency,
                "concurrency",
            ),
            (
                "Throttling.ServiceOverloaded",
                QwenMTRejectionCategory::Capacity,
                "capacity",
            ),
            (
                "ServiceOverloaded",
                QwenMTRejectionCategory::Capacity,
                "capacity",
            ),
            (
                "Throttling.ResourceExhausted",
                QwenMTRejectionCategory::Capacity,
                "capacity",
            ),
            (
                "CommodityNotPurchased",
                QwenMTRejectionCategory::Billing,
                "billing",
            ),
            (
                "PrepaidBillOverdue",
                QwenMTRejectionCategory::Billing,
                "billing",
            ),
            (
                "PostpaidBillOverdue",
                QwenMTRejectionCategory::Billing,
                "billing",
            ),
            (
                "BudgetLimitExceeded",
                QwenMTRejectionCategory::Billing,
                "billing",
            ),
        ];
        for (code, category, label) in cases {
            for response in [json!({"code": code}), json!({"error": {"code": code}})] {
                let decoded =
                    QwenMTRejectionCategory::from_response(response.to_string().as_bytes());
                assert_eq!(decoded, category);
                assert_eq!(decoded.diagnostic_label(), label);
            }
        }
    }

    #[test]
    fn rejection_category_never_interprets_messages_or_returns_untrusted_codes() {
        const PRIVATE: &str = "private-subtitle-and-key-sentinel";
        let responses = [
            json!({"error": {"message": "insufficient_quota", "content": PRIVATE}}),
            json!({"error": {"code": PRIVATE, "message": "Throttling.RateQuota"}}),
            json!({"error": {"code": "Throttling.RateQuota\nprivate-subtitle-and-key-sentinel"}}),
            json!({"error": {"code": "throttling.ratequota"}}),
            json!({"error": {"code": 429}}),
            json!({"error": {"code": ["insufficient_quota", PRIVATE]}}),
            json!({"error": {"code": {"secret": PRIVATE}}}),
            json!({"error": {"code": PRIVATE}, "code": "insufficient_quota"}),
            json!({"message": PRIVATE, "choices": [{"content": PRIVATE}]}),
        ];
        for response in responses {
            let category = QwenMTRejectionCategory::from_response(response.to_string().as_bytes());
            assert_eq!(category, QwenMTRejectionCategory::Unknown);
            assert_eq!(category.diagnostic_label(), "unknown");
            assert!(!format!("{category:?}:{}", category.diagnostic_label()).contains(PRIVATE));
        }
        let category = QwenMTRejectionCategory::from_response(
            json!({"error": {"code": "insufficient_quota", "message": PRIVATE}, "prompt": PRIVATE})
                .to_string()
                .as_bytes(),
        );
        assert_eq!(category.diagnostic_label(), "token_rate");
    }

    #[test]
    fn rejection_category_is_bounded_and_invalid_json_remains_unknown() {
        for response in [b"".as_slice(), b"not JSON", b"{", b"[]", b"null"] {
            assert_eq!(
                QwenMTRejectionCategory::from_response(response),
                QwenMTRejectionCategory::Unknown
            );
        }
        assert_eq!(
            QwenMTRejectionCategory::from_response(&vec![b' '; 1024 * 1024 + 1]),
            QwenMTRejectionCategory::Unknown,
        );
        assert_eq!(
            QwenMTRejectionCategory::from_response(
                br#"{"error":{"message":"ignored"},"code":"Throttling.Concurrency"}"#
            ),
            QwenMTRejectionCategory::Concurrency,
        );
    }

    #[test]
    fn oversized_responses_are_content_free_and_not_retried() {
        let error = QwenMTClientError::ResponseTooLarge;
        assert!(!error.is_authentication_failure());
        assert_eq!(
            error.diagnostic_label(),
            "QwenMTClientError.responseTooLarge"
        );
        assert_eq!(QwenMTRetryPolicy::delay(&error, 1), None);
    }

    fn request(text: &str, source_language: SourceLanguage) -> Value {
        QwenMTRequestEncoder::request(
            text,
            source_language,
            TargetLanguage::SimplifiedChinese,
            QwenMTModel::Flash,
            false,
            None,
            &[],
            &[],
        )
        .unwrap()
    }

    #[test]
    fn endpoint_builds_the_unified_chat_completions_url() {
        let endpoint = QwenMTEndpoint::new().unwrap();
        assert_eq!(
            endpoint.url.as_str(),
            "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions"
        );
    }

    #[test]
    fn request_selects_model_and_language_names() {
        let json = request("今日は晴れです。", SourceLanguage::Japanese);
        let messages = json["messages"].as_array().unwrap();
        let options = &json["translation_options"];

        assert_eq!(json["model"], "qwen-mt-flash");
        assert_eq!(json["stream"], false);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "今日は晴れです。");
        assert_eq!(options["source_lang"], "Japanese");
        assert_eq!(options["target_lang"], "Chinese");
    }

    #[test]
    fn request_can_enable_incremental_streaming() {
        let json = QwenMTRequestEncoder::request(
            "今日は晴れです。",
            SourceLanguage::Japanese,
            TargetLanguage::SimplifiedChinese,
            QwenMTModel::Flash,
            true,
            None,
            &[],
            &[],
        )
        .unwrap();
        assert_eq!(json["stream"], true);
    }

    #[test]
    fn lite_default_streams_and_omits_only_unsupported_domain_prompt() {
        assert_eq!(REALTIME_MT_MODEL, QwenMTModel::Lite);
        let terms = [QwenMTTerm::new("synthetic", "fixture")];
        let memory = [QwenMTMemoryPair {
            source: "previous".into(),
            target: "confirmed".into(),
        }];
        for model in [QwenMTModel::Lite, QwenMTModel::Flash, QwenMTModel::Plus] {
            let body = QwenMTRequestEncoder::request(
                "synthetic",
                SourceLanguage::English,
                TargetLanguage::Japanese,
                model,
                true,
                Some("unchanged domain fixture"),
                &terms,
                &memory,
            )
            .unwrap();
            assert_eq!(body["model"], model.raw_name());
            assert_eq!(body["stream"], true);
            assert_eq!(
                body["translation_options"].get("domains").is_some(),
                model != QwenMTModel::Lite
            );
            assert_eq!(body["translation_options"]["terms"], json!(terms));
            assert_eq!(body["translation_options"]["tm_list"], json!(memory));
        }
        assert_eq!(
            QwenMTRequestEncoder::request(
                "synthetic",
                SourceLanguage::English,
                TargetLanguage::Original,
                QwenMTModel::Lite,
                true,
                None,
                &[],
                &[]
            ),
            Err(QwenMTProtocolError::UnsupportedLanguage)
        );
    }

    #[test]
    fn expanded_explicit_source_registry_respects_model_languages_and_names() {
        let bengali = serde_json::from_value(json!("bn")).unwrap();
        let body = QwenMTRequestEncoder::request(
            "Synthetic fixture.",
            bengali,
            TargetLanguage::English,
            QwenMTModel::Lite,
            false,
            None,
            &[],
            &[],
        )
        .unwrap();
        assert_eq!(body["translation_options"]["source_lang"], "Bengali");
        for code in ["ast", "ar-EG", "pt-BR", "zh_en"] {
            let source = serde_json::from_value(json!(code)).unwrap();
            assert_eq!(
                QwenMTRequestEncoder::request(
                    "Synthetic fixture.",
                    source,
                    TargetLanguage::English,
                    QwenMTModel::Lite,
                    false,
                    None,
                    &[],
                    &[],
                ),
                Err(QwenMTProtocolError::UnsupportedLanguage)
            );
        }
    }

    #[test]
    fn model_language_registries_are_exact_unique_and_preserve_auto_boundary() {
        use std::collections::HashSet;
        assert_eq!(QWEN_MT_LITE_LANGUAGE_CODES.len(), 31);
        assert_eq!(QWEN_MT_FLASH_LANGUAGE_CODES.len(), 92);
        for model in [QwenMTModel::Lite, QwenMTModel::Flash, QwenMTModel::Plus] {
            let codes = model.supported_language_codes();
            assert_eq!(codes.iter().collect::<HashSet<_>>().len(), codes.len());
            assert!(!codes.contains(&"auto"));
            assert!(model.supports_reported_source(None));
            for code in codes {
                assert!(model.supports_reported_source(Some(code)));
            }
            for reported in ["en-US", "zh-CN", "zh-Hant", "ja-JP", "fil"] {
                assert!(model.supports_reported_source(Some(reported)));
            }
            assert!(model.supports_reported_source(Some("unknown-private-value")));
        }
        assert!(QWEN_MT_LITE_LANGUAGE_CODES
            .iter()
            .all(|code| QWEN_MT_FLASH_LANGUAGE_CODES.contains(code)));
        for audio3_only in ["no", "ro", "el", "bg", "hr", "sk"] {
            assert!(!QwenMTModel::Lite.supports_reported_source(Some(audio3_only)));
            assert!(QwenMTModel::Flash.supports_reported_source(Some(audio3_only)));
        }
    }

    #[test]
    fn unsupported_source_is_explicit_content_free_and_not_retried() {
        let error = QwenMTClientError::UnsupportedSource;
        assert_eq!(error.to_string(), "translation_source_unsupported");
        assert_eq!(
            error.diagnostic_label(),
            "QwenMTClientError.unsupportedSource"
        );
        assert!(!error.is_authentication_failure());
        assert_eq!(error.recovery_reason(), None);
        assert_eq!(QwenMTRetryPolicy::delay(&error, 1), None);
    }

    #[test]
    fn request_selects_an_explicit_target_language() {
        let json = QwenMTRequestEncoder::request(
            "今日は晴れです。",
            SourceLanguage::Japanese,
            TargetLanguage::English,
            QwenMTModel::Flash,
            false,
            None,
            &[],
            &[],
        )
        .unwrap();
        assert_eq!(json["translation_options"]["target_lang"], "English");
    }

    #[test]
    fn request_can_select_the_highest_quality_plus_model() {
        let json = QwenMTRequestEncoder::request(
            "今日はいい天気ですね。",
            SourceLanguage::Japanese,
            TargetLanguage::SimplifiedChinese,
            QwenMTModel::Plus,
            false,
            None,
            &[],
            &[],
        )
        .unwrap();
        assert_eq!(json["model"], "qwen-mt-plus");
    }

    #[test]
    fn automatic_source_uses_auto() {
        let json = request("Hello, world.", SourceLanguage::Automatic);
        assert_eq!(json["translation_options"]["source_lang"], "auto");
    }

    #[test]
    fn spoken_dialogue_guidance_preserves_vocal_sounds() {
        let guidance = QwenMTDomainHint::spoken_dialogue(
            SourceLanguage::Japanese,
            TargetLanguage::SimplifiedChinese,
        );

        assert!(guidance.contains("gasps, moans, and cries"));
        assert!(guidance.contains("嗯、啊、呢、吧、嘛"));
        assert!(guidance.contains("えっと"));
        assert!(guidance.contains("polite or formal Japanese"));
        assert!(guidance.contains("Output only the translation text"));
        assert!(guidance.contains("tm_list"));
        assert!(guidance.contains("ongoing dialogue"));
        assert!(guidance.contains("translate only the new input"));
        assert!(guidance.contains("Do not sanitize, euphemize, censor, or omit"));
        assert!(!guidance.contains("do not mechanically translate every filler"));
    }

    #[test]
    fn filler_glossary_pins_japanese_tone_words_for_chinese() {
        let terms = QwenMTDomainHint::filler_terms(
            SourceLanguage::Japanese,
            TargetLanguage::SimplifiedChinese,
        );
        assert!(terms.contains(&QwenMTTerm::new("えっと", "那个")));
        assert!(terms.contains(&QwenMTTerm::new("うーん", "嗯")));
        assert!(terms.contains(&QwenMTTerm::new("あぁ", "啊")));
        assert!(terms.contains(&QwenMTTerm::new("まあ", "嘛")));
    }

    #[test]
    fn filler_glossary_combines_languages_for_automatic_source() {
        let terms = QwenMTDomainHint::filler_terms(
            SourceLanguage::Automatic,
            TargetLanguage::SimplifiedChinese,
        );
        assert!(terms.contains(&QwenMTTerm::new("えっと", "那个")));
        assert!(terms.contains(&QwenMTTerm::new("um", "嗯")));
    }

    #[test]
    fn request_encodes_the_filler_glossary_in_translation_options() {
        let terms = QwenMTDomainHint::filler_terms(
            SourceLanguage::Japanese,
            TargetLanguage::SimplifiedChinese,
        );
        let json = QwenMTRequestEncoder::request(
            "えっと、うーん、あぁ。",
            SourceLanguage::Japanese,
            TargetLanguage::SimplifiedChinese,
            QwenMTModel::Flash,
            true,
            None,
            &terms,
            &[],
        )
        .unwrap();
        let encoded_terms = json["translation_options"]["terms"].as_array().unwrap();

        assert_eq!(encoded_terms.len(), terms.len());
        assert_eq!(encoded_terms[0]["source"], "えっと");
        assert_eq!(encoded_terms[0]["target"], "那个");
    }

    #[test]
    fn request_omits_terms_when_none_are_provided() {
        let json = request("今日は晴れです。", SourceLanguage::Japanese);
        assert!(json["translation_options"].get("terms").is_none());
    }

    #[test]
    fn request_includes_bounded_translation_memory_pairs() {
        let json = QwenMTRequestEncoder::request(
            "そうなんですね。",
            SourceLanguage::Japanese,
            TargetLanguage::SimplifiedChinese,
            QwenMTModel::Flash,
            false,
            None,
            &[],
            &[QwenMTMemoryPair {
                source: "今日は晴れです。".to_string(),
                target: "今天天气很好。".to_string(),
            }],
        )
        .unwrap();
        let memory = json["translation_options"]["tm_list"].as_array().unwrap();
        assert_eq!(memory[0]["source"], "今日は晴れです。");
        assert_eq!(memory[0]["target"], "今天天气很好。");
    }

    #[test]
    fn asr_language_reports_resolve_to_explicit_qwen_mt_languages() {
        assert_eq!(
            SourceLanguage::from_detected(Some("ja-JP")),
            Some(SourceLanguage::Japanese)
        );
        assert_eq!(
            SourceLanguage::from_detected(Some("English")),
            Some(SourceLanguage::English)
        );
        assert_eq!(
            SourceLanguage::from_detected(Some("ko")),
            Some(SourceLanguage::Korean)
        );
        assert_eq!(
            SourceLanguage::from_detected(Some("zh-CN")),
            Some(SourceLanguage::Chinese)
        );
        assert_eq!(SourceLanguage::from_detected(Some("unknown")), None);
    }

    #[test]
    fn response_decodes_and_trims_translated_content() {
        let translation = QwenMTResponseDecoder::decode(
            r#"{"choices":[{"message":{"role":"assistant","content":"  今天天气晴朗。  "}}]}"#,
        )
        .unwrap();
        assert_eq!(translation, "今天天气晴朗。");
    }

    #[test]
    fn response_requires_translated_content() {
        assert!(matches!(
            QwenMTResponseDecoder::decode(r#"{"choices":[]}"#),
            Err(QwenMTProtocolError::MissingTranslation)
        ));
    }

    #[test]
    fn stream_chunk_decodes_incremental_content() {
        let content = QwenMTStreamDecoder::decode_chunk(
            r#"{"choices":[{"delta":{"role":"assistant","content":"今天"}}]}"#,
        )
        .unwrap();
        let terminal = QwenMTStreamDecoder::decode_chunk(r#"{"choices":[]}"#).unwrap();
        assert_eq!(content.as_deref(), Some("今天"));
        assert_eq!(terminal, None);
    }

    #[test]
    fn timeout_has_a_useful_error_message() {
        assert_eq!(
            QwenMTClientError::RequestTimedOut.to_string(),
            "Qwen-MT took too long to respond."
        );
    }

    #[test]
    fn diagnostics_retain_status_without_response_content() {
        assert_eq!(
            QwenMTClientError::RequestFailed {
                status_code: 429,
                message: "sensitive response detail".into()
            }
            .diagnostic_label(),
            "QwenMTClientError.requestFailed(status=429)"
        );
    }

    #[test]
    fn retry_policy_backs_off_only_for_transient_failures() {
        assert_eq!(
            QwenMTRetryPolicy::delay(&QwenMTClientError::RequestTimedOut, 1),
            Some(Duration::from_millis(600))
        );
        assert_eq!(
            QwenMTRetryPolicy::delay(
                &QwenMTClientError::RequestFailed {
                    status_code: 429,
                    message: "busy".into()
                },
                3
            ),
            Some(Duration::from_secs(8))
        );
        assert_eq!(
            QwenMTRetryPolicy::delay(
                &QwenMTClientError::RequestFailed {
                    status_code: 503,
                    message: "down".into()
                },
                8
            ),
            Some(Duration::from_secs(8))
        );
        assert_eq!(
            QwenMTRetryPolicy::delay(
                &QwenMTClientError::RequestFailed {
                    status_code: 401,
                    message: "bad key".into()
                },
                1
            ),
            None
        );
        assert_eq!(
            QwenMTRetryPolicy::delay(
                &QwenMTClientError::RequestFailed {
                    status_code: 400,
                    message: "bad request".into()
                },
                1
            ),
            None
        );
    }

    #[test]
    fn recovery_classification_preserves_authentication_and_bounded_rate_limit_backoff() {
        use crate::core::diagnostics::TranslationRecoveryReason;
        for error in [
            QwenMTClientError::RequestFailed {
                status_code: 429,
                message: "synthetic provider detail".into(),
            },
            QwenMTClientError::DeepL(super::super::deepl::DeepLError::Rejected(429)),
            QwenMTClientError::DeepLX(super::super::deeplx::DeepLXError::Rejected(429)),
            QwenMTClientError::OpenAICompatible(
                super::super::openai_compatible::OpenAICompatibleError::Rejected(429),
            ),
        ] {
            assert_eq!(
                error.recovery_reason(),
                Some(TranslationRecoveryReason::RateLimited)
            );
            assert!(!error.is_authentication_failure());
            assert_eq!(
                QwenMTRetryPolicy::delay(&error, 1),
                Some(Duration::from_secs(4))
            );
            assert_eq!(
                QwenMTRetryPolicy::delay(&error, 2),
                Some(Duration::from_secs(8))
            );
            assert_eq!(
                QwenMTRetryPolicy::delay(&error, usize::MAX),
                Some(Duration::from_secs(8))
            );
        }
        for status_code in [401, 403] {
            let error = QwenMTClientError::RequestFailed {
                status_code,
                message: "synthetic auth rejection".into(),
            };
            assert!(error.is_authentication_failure());
            assert_eq!(error.recovery_reason(), None);
            let error = QwenMTClientError::OpenAICompatible(
                super::super::openai_compatible::OpenAICompatibleError::Rejected(status_code),
            );
            assert!(error.is_authentication_failure());
            assert_eq!(error.recovery_reason(), None);
            assert_eq!(QwenMTRetryPolicy::delay(&error, 1), None);
        }
        for error in [
            QwenMTClientError::RequestTimedOut,
            QwenMTClientError::OpenAICompatible(
                super::super::openai_compatible::OpenAICompatibleError::Timeout,
            ),
            QwenMTClientError::OpenAICompatible(
                super::super::openai_compatible::OpenAICompatibleError::Rejected(503),
            ),
            QwenMTClientError::RequestFailed {
                status_code: 503,
                message: String::new(),
            },
        ] {
            assert_eq!(
                error.recovery_reason(),
                Some(TranslationRecoveryReason::TemporarilyUnavailable)
            );
        }
    }

    #[test]
    fn missing_independent_translation_is_a_configuration_error_without_retry_or_auth_claims() {
        let error = QwenMTClientError::MissingTextTranslation;
        assert_eq!(error.recovery_reason(), None);
        assert!(!error.is_authentication_failure());
        assert_eq!(QwenMTRetryPolicy::delay(&error, 1), None);
        assert_eq!(
            error.diagnostic_label(),
            "TranslationClientError.missingTextTranslation"
        );
        assert!(!error.to_string().contains("Alibaba"));
    }
}
