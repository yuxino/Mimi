//! Production adapters do not inspect or retain recognition/translation content.
use crate::core::audio_input::AudioSource;
use crate::core::protocols::qwen_mt::QwenMTClientError;
use serde_json::Value;
use std::future::Future;

pub fn asr_request(_: Option<(AudioSource, u64, u64)>, _: &Value) {}

pub type RequestContext = ();
pub struct TranslationAttemptTicket;

pub fn begin_attempt(_: Option<RequestContext>) -> TranslationAttemptTicket {
    TranslationAttemptTicket
}

pub fn scope_attempt<F: Future>(
    _: &TranslationAttemptTicket,
    future: F,
) -> impl Future<Output = F::Output> {
    future
}

impl TranslationAttemptTicket {
    pub fn complete(self, _: &Result<String, QwenMTClientError>) {}
}

pub enum RequestProtocol {
    QwenMt,
    OpenaiCompatible,
    DeepL,
    DeepLX,
}

pub fn request(_: RequestProtocol, _: &Value) {}
