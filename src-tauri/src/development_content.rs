//! Explicit private dev evidence. All persistence is supplied by the app layer;
//! clients never log headers, credentials, endpoints, or arbitrary errors here.
use crate::core::audio_input::AudioSource;
use crate::core::development_debug::{Admission, ProviderObservation};
use crate::core::protocols::live_translate::LiveTranslateServerEvent;
use crate::core::protocols::qwen_mt::QwenMTClientError;
use serde_json::{json, Value};
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

pub type Sink = Arc<dyn Fn(Value) + Send + Sync>;
static ENABLED: AtomicBool = AtomicBool::new(false);
fn sink() -> &'static Mutex<Option<Sink>> {
    static SINK: OnceLock<Mutex<Option<Sink>>> = OnceLock::new();
    SINK.get_or_init(Default::default)
}
pub fn configure(value: Option<Sink>) {
    ENABLED.store(false, Ordering::SeqCst);
    let enabled = value.is_some();
    *sink().lock().unwrap() = value;
    ENABLED.store(enabled, Ordering::SeqCst);
}
fn record(make_value: impl FnOnce() -> Value) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let target = sink().lock().unwrap().clone();
    if let Some(target) = target {
        target(make_value());
    }
}

pub fn provider(
    source: AudioSource,
    generation: u64,
    revision: u64,
    sequence: Option<u64>,
    producer: crate::core::development_debug::DebugProducer,
    event: &LiveTranslateServerEvent,
    admission: Admission,
) {
    record(|| {
        let mut observation =
            ProviderObservation::new(source, generation, revision, event, admission);
        observation.transport_sequence = sequence;
        observation.producer = producer;
        json!({"kind":"provider","observation":observation,"content":if event.text_within_limit() {provider_content(event)} else {Value::Null}})
    });
}
fn provider_content(event: &LiveTranslateServerEvent) -> Value {
    use LiveTranslateServerEvent as E;
    match event {
        E::SourceDraft { text, language }
        | E::SourceFinal { text, language }
        | E::SourceUtteranceDraft { text, language, .. }
        | E::SourceUtteranceFinal { text, language, .. } => {
            json!({"source":text,"language":language})
        }
        E::TranslationDraft(text) | E::TranslationFinal(text) => json!({"translation":text}),
        E::UtteranceText {
            utterance_id,
            role,
            text,
            is_final,
            language,
        } => {
            json!({"utteranceId":utterance_id.chars().take(256).collect::<String>(),"role":match role {crate::core::models::UtteranceRole::Source=>"source",crate::core::models::UtteranceRole::Translation=>"translation"},"text":text,"isFinal":is_final,"language":language})
        }
        E::SubtitlePreviewPair {
            source,
            language,
            translation,
            ..
        }
        | E::SubtitleFinalPair {
            source,
            language,
            translation,
        }
        | E::SubtitleConfirmedPair {
            source,
            language,
            translation,
            ..
        } => json!({"source":source,"translation":translation,"language":language}),
        // Error strings can echo keys or complete requests. Never save them.
        _ => Value::Null,
    }
}

#[derive(Clone, Copy)]
pub struct RequestContext {
    pub source: AudioSource,
    pub generation: u64,
    pub revision: u64,
    pub owner: u64,
    pub preview: bool,
    pub attempt: usize,
    pub request_id: u64,
}
tokio::task_local! {
    static REQUEST_BINDING: RequestBinding;
}
static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);
struct RequestBinding {
    context: RequestContext,
    target: Sink,
}

/// Captures the originating case sink before HTTP work starts. Later stop,
/// reopen or recording replacement cannot redirect a result into a new case.
pub struct TranslationAttemptTicket {
    inner: Option<TranslationAttempt>,
}
struct TranslationAttempt {
    context: RequestContext,
    target: Sink,
    started: Instant,
}

pub fn begin_attempt(context: Option<RequestContext>) -> TranslationAttemptTicket {
    let target = if ENABLED.load(Ordering::Relaxed) {
        sink().lock().unwrap().clone()
    } else {
        None
    };
    begin_attempt_with(context, target)
}
fn begin_attempt_with(
    context: Option<RequestContext>,
    target: Option<Sink>,
) -> TranslationAttemptTicket {
    let inner = context.zip(target).map(|(mut context, target)| {
        context.request_id = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
        TranslationAttempt {
            context,
            target,
            started: Instant::now(),
        }
    });
    TranslationAttemptTicket { inner }
}
pub fn scope_attempt<F: Future>(
    ticket: &TranslationAttemptTicket,
    future: F,
) -> impl Future<Output = F::Output> {
    let binding = ticket.inner.as_ref().map(|attempt| RequestBinding {
        context: attempt.context,
        target: attempt.target.clone(),
    });
    async move {
        match binding {
            Some(binding) => REQUEST_BINDING.scope(binding, future).await,
            None => future.await,
        }
    }
}
impl TranslationAttemptTicket {
    pub fn complete(mut self, result: &Result<String, QwenMTClientError>) {
        if let Some(attempt) = self.inner.take() {
            let (outcome, boundary, output, error, status) = match result {
                Ok(text) => (
                    "decoded",
                    "localTranslationDecoded",
                    Some(text.as_str()),
                    None,
                    None,
                ),
                Err(error) => {
                    let (label, status, timeout) = safe_attempt_error(error);
                    (
                        if timeout { "timeout" } else { "failed" },
                        "localTranslationFinished",
                        None,
                        Some(label),
                        status,
                    )
                }
            };
            attempt.record(outcome, boundary, output, error, status);
        }
    }
}
impl Drop for TranslationAttemptTicket {
    fn drop(&mut self) {
        if let Some(attempt) = self.inner.take() {
            attempt.record(
                "cancelled",
                "localTranslationFutureDropped",
                None,
                None,
                None,
            );
        }
    }
}
impl TranslationAttempt {
    fn record(
        self,
        outcome: &'static str,
        boundary: &'static str,
        output: Option<&str>,
        error: Option<&'static str>,
        status: Option<u16>,
    ) {
        let context = self.context;
        let limited =
            output.is_some_and(|text| !crate::core::models::subtitle_text_within_limit(text));
        (self.target)(
            json!({"kind":"translationAttemptResult","requestId":context.request_id,"source":context.source,"generation":context.generation,"contentRevision":context.revision,"owner":context.owner,"lane":if context.preview {"preview"} else {"final"},"attempt":context.attempt,"boundary":boundary,"outcome":outcome,"durationMs":self.started.elapsed().as_millis() as u64,"serverReceiptProven":false,"outputBytes":output.map(str::len),"outputLimited":limited,"output":if limited {None} else {output},"errorLabel":error,"httpStatus":status}),
        );
    }
}
fn safe_attempt_error(error: &QwenMTClientError) -> (&'static str, Option<u16>, bool) {
    use crate::core::protocols::{
        deepl::DeepLError, deeplx::DeepLXError, openai_compatible::OpenAICompatibleError,
    };
    match error {
        QwenMTClientError::RequestTimedOut
        | QwenMTClientError::DeepL(DeepLError::Timeout)
        | QwenMTClientError::DeepLX(DeepLXError::Timeout)
        | QwenMTClientError::OpenAICompatible(OpenAICompatibleError::Timeout) => {
            ("request_timeout", None, true)
        }
        QwenMTClientError::RequestFailed { status_code, .. }
        | QwenMTClientError::DeepL(DeepLError::Rejected(status_code))
        | QwenMTClientError::DeepLX(DeepLXError::Rejected(status_code))
        | QwenMTClientError::OpenAICompatible(OpenAICompatibleError::Rejected(status_code)) => {
            ("request_rejected", Some(*status_code), false)
        }
        QwenMTClientError::DeepL(DeepLError::Connection)
        | QwenMTClientError::DeepLX(DeepLXError::Connection)
        | QwenMTClientError::OpenAICompatible(OpenAICompatibleError::Connection) => {
            ("connection_failed", None, false)
        }
        QwenMTClientError::InvalidHTTPResponse
        | QwenMTClientError::DeepL(DeepLError::Response)
        | QwenMTClientError::DeepLX(DeepLXError::Response)
        | QwenMTClientError::OpenAICompatible(OpenAICompatibleError::Response) => {
            ("invalid_response", None, false)
        }
        QwenMTClientError::ResponseTooLarge
        | QwenMTClientError::DeepL(DeepLError::TooLarge)
        | QwenMTClientError::DeepLX(DeepLXError::TooLarge)
        | QwenMTClientError::OpenAICompatible(OpenAICompatibleError::TooLarge) => {
            ("response_too_large", None, false)
        }
        QwenMTClientError::MissingAPIKey
        | QwenMTClientError::MissingTextTranslation
        | QwenMTClientError::UnsupportedSource
        | QwenMTClientError::DeepL(DeepLError::InvalidKey)
        | QwenMTClientError::DeepLX(DeepLXError::Endpoint)
        | QwenMTClientError::OpenAICompatible(
            OpenAICompatibleError::Endpoint
            | OpenAICompatibleError::Model
            | OpenAICompatibleError::APIKey
            | OpenAICompatibleError::Language,
        ) => ("configuration_invalid", None, false),
    }
}

#[derive(Clone, Copy)]
pub enum RequestProtocol {
    QwenMt,
    OpenaiCompatible,
    DeepL,
    DeepLX,
}
pub fn request(protocol: RequestProtocol, body: &Value) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let Ok((context, target)) = REQUEST_BINDING.try_with(|b| (b.context, b.target.clone())) else {
        return;
    };
    target(request_value(protocol, body, context));
}
fn request_value(protocol: RequestProtocol, body: &Value, context: RequestContext) -> Value {
    let (name, fields): (&str, &[&str]) = match protocol {
        RequestProtocol::QwenMt => (
            "qwenMt",
            &["model", "messages", "stream", "translation_options"],
        ),
        RequestProtocol::OpenaiCompatible => (
            "openaiCompatible",
            &["model", "messages", "stream", "temperature"],
        ),
        RequestProtocol::DeepL => ("deepL", &["text", "source_lang", "target_lang"]),
        RequestProtocol::DeepLX => ("deepLX", &["text", "source_lang", "target_lang"]),
    };
    let body: serde_json::Map<String, Value> = fields
        .iter()
        .filter_map(|key| {
            body.get(*key)
                .map(|value| ((*key).to_string(), value.clone()))
        })
        .collect();
    json!({"kind":"translationRequest","protocol":name,"requestId":context.request_id,"source":context.source,"generation":context.generation,"contentRevision":context.revision,"owner":context.owner,"lane":if context.preview {"preview"} else {"final"},"attempt":context.attempt,"boundary":"httpRequestPrepared","serverReceiptProven":false,"body":body})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> RequestContext {
        RequestContext {
            source: AudioSource::Microphone,
            generation: 8,
            revision: 5,
            owner: 13,
            preview: true,
            attempt: 2,
            request_id: 0,
        }
    }
    fn collector() -> (Sink, Arc<Mutex<Vec<Value>>>) {
        let values = Arc::new(Mutex::new(Vec::new()));
        let output = values.clone();
        (
            Arc::new(move |value| output.lock().unwrap().push(value)),
            values,
        )
    }

    #[tokio::test]
    async fn attempts_are_default_off_and_bound_to_the_originating_case() {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                configure(None);
            }
        }
        let _reset = Reset;
        configure(None);
        let inactive = begin_attempt(Some(context()));
        assert!(inactive.inner.is_none());
        inactive.complete(&Ok("ignored while disabled".to_string()));

        let (case_a, a) = collector();
        let (case_b, b) = collector();
        configure(Some(case_a));
        let ticket_a = begin_attempt(Some(context()));
        configure(Some(case_b));
        scope_attempt(&ticket_a, async {
            request(RequestProtocol::DeepL, &json!({"text":["synthetic input"]}));
        })
        .await;
        ticket_a.complete(&Ok("synthetic decoded output".to_string()));
        let values_a = a.lock().unwrap();
        assert_eq!(values_a.len(), 2);
        assert_eq!(values_a[0]["kind"], "translationRequest");
        assert_eq!(values_a[1]["kind"], "translationAttemptResult");
        assert_eq!(values_a[0]["requestId"], values_a[1]["requestId"]);
        assert_ne!(values_a[0]["requestId"], 0);
        for value in values_a.iter() {
            assert_eq!(value["source"], "microphone");
            assert_eq!(value["generation"], 8);
            assert_eq!(value["contentRevision"], 5);
            assert_eq!(value["owner"], 13);
            assert_eq!(value["lane"], "preview");
            assert_eq!(value["attempt"], 2);
            assert_eq!(value["serverReceiptProven"], false);
        }
        assert_eq!(values_a[1]["outcome"], "decoded");
        assert_eq!(values_a[1]["output"], "synthetic decoded output");
        assert!(b.lock().unwrap().is_empty());
        drop(values_a);

        let ticket_b = begin_attempt(Some(context()));
        configure(None);
        drop(ticket_b);
        let values_b = b.lock().unwrap();
        assert_eq!(values_b.len(), 1);
        assert_eq!(values_b[0]["outcome"], "cancelled");
        assert_eq!(values_b[0]["boundary"], "localTranslationFutureDropped");
        assert!(values_b[0]["output"].is_null());
        assert_eq!(a.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn aborting_an_inflight_attempt_records_local_cancellation_once() {
        let (target, values) = collector();
        let ticket = begin_attempt_with(Some(context()), Some(target));
        let (ready, started) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let result = scope_attempt(&ticket, async {
                ready.send(()).unwrap();
                std::future::pending::<Result<String, QwenMTClientError>>().await
            })
            .await;
            ticket.complete(&result);
        });
        started.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        let events = values.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["outcome"], "cancelled");
        assert_eq!(events[0]["serverReceiptProven"], false);
        assert!(events[0]["errorLabel"].is_null());
        assert!(events[0]["output"].is_null());
    }

    #[test]
    fn attempt_errors_are_allowlisted_and_decoded_output_is_bounded() {
        let (target, values) = collector();
        begin_attempt_with(Some(context()), Some(target.clone())).complete(&Err(
            QwenMTClientError::RequestFailed {
                status_code: 429,
                message: "untrusted error Authorization=secret endpoint=secret".into(),
            },
        ));
        begin_attempt_with(Some(context()), Some(target.clone()))
            .complete(&Err(QwenMTClientError::RequestTimedOut));
        let oversized = "x".repeat(crate::core::models::MAX_SUBTITLE_TEXT_BYTES + 1);
        begin_attempt_with(Some(context()), Some(target)).complete(&Ok(oversized));
        let events = values.lock().unwrap();
        assert_eq!(events.len(), 3);
        assert!(!serde_json::to_string(&*events).unwrap().contains("secret"));
        assert_eq!(events[0]["outcome"], "failed");
        assert_eq!(events[0]["errorLabel"], "request_rejected");
        assert_eq!(events[0]["httpStatus"], 429);
        assert!(events[0]["output"].is_null());
        assert_eq!(events[1]["outcome"], "timeout");
        assert_eq!(events[1]["errorLabel"], "request_timeout");
        assert_eq!(events[2]["outcome"], "decoded");
        assert_eq!(events[2]["outputLimited"], true);
        assert_eq!(
            events[2]["outputBytes"],
            crate::core::models::MAX_SUBTITLE_TEXT_BYTES + 1
        );
        assert!(events[2]["output"].is_null());
    }

    #[test]
    fn private_provider_fields_preserve_text_and_pairing_without_error_payloads() {
        let event = LiveTranslateServerEvent::SubtitleFinalPair {
            source: "synthetic source".into(),
            language: Some("en".into()),
            translation: "synthetic translation".into(),
        };
        assert_eq!(
            provider_content(&event),
            json!({"source":"synthetic source","language":"en","translation":"synthetic translation"})
        );
        assert!(provider_content(&LiveTranslateServerEvent::Error {
            code: "secret".into(),
            message: "secret".into()
        })
        .is_null());
    }
    #[test]
    fn private_request_allowlist_excludes_headers_endpoints_and_credentials() {
        let context = RequestContext {
            source: AudioSource::System,
            generation: 3,
            revision: 2,
            owner: 1,
            preview: false,
            attempt: 1,
            request_id: 0,
        };
        let body = json!({"model":"synthetic","messages":[{"role":"user","content":"synthetic text"}],"Authorization":"secret","api_key":"secret","endpoint":"secret"});
        let output = request_value(RequestProtocol::OpenaiCompatible, &body, context);
        assert!(!output.to_string().contains("secret"));
        assert_eq!(output["body"]["messages"], body["messages"]);
        assert_eq!(output["boundary"], "httpRequestPrepared");
        assert_eq!(output["serverReceiptProven"], false);
    }
}
