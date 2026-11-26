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
    #[error("Qwen-MT returned an incomplete translation.")]
    IncompleteTranslation,
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

    pub fn recovery_reason(
        &self,
    ) -> Option<crate::core::diagnostics::latency::TranslationRecoveryReason> {
        use crate::core::diagnostics::latency::TranslationRecoveryReason;
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

pub const MAX_TRANSLATION_BYTES: usize = 64 * 1024;

/// The OpenAI-compatible Qwen-MT stream uses cumulative snapshots for Plus,
/// but incremental chunks for Lite and Flash. Bound the resulting translation
/// before changing it so neither partials nor JNI adapters can grow it unchecked.
#[derive(Default)]
pub struct QwenMTStreamText {
    model: QwenMTModel,
    text: String,
}

impl QwenMTStreamText {
    pub fn new(model: QwenMTModel) -> Self {
        Self {
            model,
            text: String::new(),
        }
    }

    /// Empty completion chunks keep the latest value. A duplicate cumulative
    /// stop chunk confirms completion without publishing the same preview again.
    pub fn apply(&mut self, content: &str) -> Result<bool, QwenMTClientError> {
        if content.is_empty() {
            return Ok(false);
        }
        if self.model == QwenMTModel::Plus {
            if content.len() > MAX_TRANSLATION_BYTES {
                return Err(QwenMTClientError::ResponseTooLarge);
            }
            if self.text == content {
                return Ok(false);
            }
            self.text.clear();
        } else if content.len() > MAX_TRANSLATION_BYTES - self.text.len() {
            return Err(QwenMTClientError::ResponseTooLarge);
        }
        self.text.push_str(content);
        Ok(true)
    }

    pub fn text(&self) -> &str {
        &self.text
    }
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
            .contains(&match target_language {
                TargetLanguage::Norwegian => "nb",
                _ => target_language.raw_value(),
            })
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
            "target_lang": match target_language { TargetLanguage::Norwegian => "Norwegian Bokmål", _ => target_language.qwen_mt_name() },
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
            #[serde(flatten)]
            metadata: serde_json::Map<String, Value>,
        }
        #[derive(Deserialize)]
        struct Message {
            content: String,
        }

        let response: Response =
            serde_json::from_str(text).map_err(|_| QwenMTProtocolError::InvalidJSON)?;
        if let Some(choice) = response.choices.first() {
            // Keep compatibility with complete JSON responses that omit the
            // field, but never confirm an explicitly unfinished result.
            match choice.metadata.get("finish_reason") {
                None => {}
                Some(Value::String(reason)) if reason == "stop" => {}
                _ => return Err(QwenMTProtocolError::IncompleteTranslation),
            }
        }
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
        Self::decode_chunk_with_completion(text).map(|(content, _)| content)
    }

    /// A null/omitted reason is an intermediate chunk; only `stop` confirms
    /// completion. A length limit or filtered result must never become a final.
    pub fn decode_chunk_with_completion(
        text: &str,
    ) -> Result<(Option<String>, bool), QwenMTProtocolError> {
        #[derive(Deserialize)]
        struct Response {
            choices: Vec<Choice>,
        }
        #[derive(Deserialize)]
        struct Choice {
            delta: Delta,
            #[serde(flatten)]
            metadata: serde_json::Map<String, Value>,
        }
        #[derive(Deserialize)]
        struct Delta {
            content: Option<String>,
        }

        let response: Response =
            serde_json::from_str(text).map_err(|_| QwenMTProtocolError::InvalidJSON)?;
        let Some(choice) = response.choices.into_iter().next() else {
            return Ok((None, false));
        };
        let complete = match choice.metadata.get("finish_reason") {
            None | Some(Value::Null) => false,
            Some(Value::String(reason)) if reason == "stop" => true,
            _ => return Err(QwenMTProtocolError::IncompleteTranslation),
        };
        Ok((choice.delta.content, complete))
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
mod tests;
