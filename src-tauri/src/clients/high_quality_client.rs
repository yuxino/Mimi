//! Bounded recognition + independently configured text-translation pipeline.
//!
//! Replaceable ASR drafts use a latest-only preview lane. Only authoritative
//! server finals (plus a bounded session-finish fallback) enter the durable,
//! serial final-translation queue.

use crate::clients::provider_events::{
    provider_event_channel, ProviderEventReceiver, ProviderEventSender,
};
use crate::clients::provider_network::{ProviderNetwork, ProviderNetworkError};
use crate::clients::qwen_mt_client::QwenMTClient;
use crate::clients::recognition_client::{RecognitionClient, RecognitionClientError};
use crate::core::committer::ASRDraftCommitter;
use crate::core::configuration::LiveTranslationConfiguration;
use crate::core::credentials::{ProviderCredentials, TextTranslationCredentials};
use crate::core::diagnostics::{
    TranslationLatency, TranslationLatencyKind, TranslationRecovery, TranslationRecoveryReason,
};
use crate::core::models::{SourceLanguage, TargetLanguage};
use crate::core::preview_pacing::{MTRequestBudget, PreviewCandidate, PreviewRequestPacer};
use crate::core::protocols::live_translate::LiveTranslateServerEvent;
use crate::core::protocols::qwen_mt::{QwenMTClientError, QwenMTModel};
use crate::core::subtitle_reducer::trim;
use crate::pipeline_log;
#[cfg(test)]
use mimi_core::translation_policy::MAX_TRANSLATION_ATTEMPTS;
use mimi_core::translation_policy::{self, FINAL_DEADLINE_MS, MAX_FINAL_QUEUE_DEPTH};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{oneshot, Mutex};
use tokio::task::JoinHandle;
use uuid::Uuid;

const MAX_FINAL_REQUEST_AGE: Duration = Duration::from_millis(FINAL_DEADLINE_MS);
const MAX_PREVIEW_REQUEST_AGE: Duration = Duration::from_secs(12);
const ASR_BRIDGE_FINISH_TIMEOUT: Duration =
    Duration::from_millis(translation_policy::RECOGNITION_FINISH_TIMEOUT_MS);
const OVERLOAD_ERROR_CODE: &str = "translation_backlog_overflow";
const OVERLOAD_ERROR_MESSAGE: &str = "Translation fell behind live audio. mimi is reconnecting.";

type PartialHandler = Arc<dyn Fn(String) + Send + Sync>;

fn recognition_error(error: RecognitionClientError) -> QwenMTClientError {
    QwenMTClientError::RequestFailed {
        status_code: 0,
        message: error.to_string(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FinalBoundary {
    ServerFinal,
    SessionFinish,
}

impl FinalBoundary {
    fn label(self) -> &'static str {
        match self {
            Self::ServerFinal => "server-final",
            Self::SessionFinish => "session-finish",
        }
    }
}

#[derive(Clone)]
struct TranslationRequest {
    text: String,
    language: Option<String>,
    boundary: FinalBoundary,
    utterance_revision: u64,
    source_utterance_id: Option<u64>,
    content_revision: u64,
    enqueued_at: tokio::time::Instant,
}

impl TranslationRequest {
    fn key(&self) -> FinalRequestKey {
        FinalRequestKey {
            text: self.text.clone(),
            boundary: self.boundary,
            utterance_revision: self.utterance_revision,
            source_utterance_id: self.source_utterance_id,
            content_revision: self.content_revision,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
struct FinalRequestKey {
    text: String,
    boundary: FinalBoundary,
    utterance_revision: u64,
    source_utterance_id: Option<u64>,
    content_revision: u64,
}

impl FinalRequestKey {
    fn matches(&self, request: &TranslationRequest) -> bool {
        // A finish fallback may alias a server final only for an identified
        // source. Missing IDs never make equal text a causal identity.
        self.text == request.text
            && self.content_revision == request.content_revision
            && self.source_utterance_id == request.source_utterance_id
            && (self.utterance_revision == request.utterance_revision
                || (self.source_utterance_id.is_some()
                    && (self.boundary == FinalBoundary::SessionFinish
                        || request.boundary == FinalBoundary::SessionFinish)))
    }
}

struct TaskSlot {
    id: u64,
    handle: JoinHandle<()>,
}

#[derive(Default)]
struct ASRBridgeState {
    task: Option<JoinHandle<()>>,
    finish_ack: Option<oneshot::Receiver<()>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DraftTimerKind {
    Stable,
    Maximum,
}

#[derive(Clone, Copy)]
enum PreviewCancellationReason {
    SameLanguage,
    SourceBoundary,
    SourceFinal,
    RevertedWaiter,
    ReplacedWaiter,
    FinalWorker,
    SessionFinish,
    Stop,
    Reset,
}

impl PreviewCancellationReason {
    fn label(self) -> &'static str {
        match self {
            Self::SameLanguage => "same-language",
            Self::SourceBoundary => "source-boundary",
            Self::SourceFinal => "source-final",
            Self::RevertedWaiter => "reverted-waiter",
            Self::ReplacedWaiter => "replaced-waiter",
            Self::FinalWorker => "final-worker",
            Self::SessionFinish => "session-finish",
            Self::Stop => "stop",
            Self::Reset => "reset",
        }
    }
}

enum EnqueueOutcome {
    Queued,
    Overloaded,
    DeferredOverload,
}

#[derive(Clone, Copy)]
enum TranslationWorkOwner {
    Preview(u64),
    Final(u64),
}

#[derive(Clone, Copy)]
struct MTCooldown {
    until: tokio::time::Instant,
    reason: TranslationRecoveryReason,
}

struct MeasuredTranslation {
    text: String,
    request_ms: Option<u64>,
}

#[derive(Clone, Copy, Default)]
struct TranslationEvidenceIdentity {
    #[cfg(any(test, feature = "development-debugger"))]
    source_utterance_id: Option<u64>,
    #[cfg(any(test, feature = "development-debugger"))]
    pair_id: Option<u64>,
    #[cfg(any(test, feature = "development-debugger"))]
    final_boundary: Option<&'static str>,
}

struct Inner {
    committer: ASRDraftCommitter,
    latest_draft_language: Option<String>,
    draft_revision: u64,
    next_confirmation_id: u64,
    /// `(text, revision_after_final)`. An identical final with no intervening
    /// draft is a duplicate; the same spoken line after a new draft is not.
    last_server_final: Option<(String, u64)>,
    /// Positive Audio3 sentence IDs belong to the current recognizer task.
    /// One watermark rejects replays without depending on draft delivery.
    last_source_utterance_id: Option<u64>,
    /// Identity of the newest observed cumulative draft, independently of
    /// the final watermark: a previous final can arrive after the next begin.
    current_source_utterance_id: Option<u64>,
    final_queue: VecDeque<TranslationRequest>,
    active_final: Option<FinalRequestKey>,
    draft_stability_task: Option<TaskSlot>,
    draft_maximum_wait_task: Option<TaskSlot>,
    preview_task: Option<TaskSlot>,
    preview_candidate: Option<PreviewCandidate>,
    preview_http_pending: Option<u64>,
    /// One marker, not a text queue. The bounded committer owns the newest
    /// same-utterance draft while an already-started HTTP request completes.
    pending_preview_revision: Option<u64>,
    final_worker: Option<TaskSlot>,
    next_task_id: u64,
    pipeline_failed: bool,
    final_completion_in_progress: bool,
    deferred_overload: bool,
    /// Throttles the per-draft diagnostic log (drafts stream several times
    /// per second while speech flows).
    last_draft_log_at: Option<tokio::time::Instant>,
    mt_cooldown: Option<MTCooldown>,
    mt_failure_streak: usize,
    preview_request_pacer: PreviewRequestPacer,
}

impl Inner {
    fn cancel_preview(&mut self) -> Option<u64> {
        abort_task(&mut self.preview_task);
        self.preview_candidate = None;
        self.pending_preview_revision = None;
        self.preview_http_pending.take()
    }

    fn final_lane_busy(&self) -> bool {
        self.final_worker.is_some()
            || self.active_final.is_some()
            || !self.final_queue.is_empty()
            || self.final_completion_in_progress
    }
}

#[derive(Clone)]
enum TextTranslationClient {
    Disabled,
    Qwen(QwenMTClient, QwenMTModel),
    DeepLX(crate::clients::deeplx_client::DeepLXClient),
    DeepL(crate::clients::deepl_client::DeepLClient),
    OpenAICompatible(crate::clients::openai_compatible_client::OpenAICompatibleClient),
}
impl TextTranslationClient {
    fn set_network(&mut self, network: ProviderNetwork) -> Result<(), ProviderNetworkError> {
        match self {
            Self::Disabled => Ok(()),
            Self::Qwen(client, _) => client.set_network(network),
            Self::DeepL(client) => client.set_network(network),
            Self::DeepLX(client) => client.set_network(network),
            Self::OpenAICompatible(client) => client.set_network(network),
        }
    }
    fn supports_reported_source(&self, language: Option<&str>) -> bool {
        match self {
            Self::Disabled => true,
            Self::Qwen(_, model) => model.supports_reported_source(language),
            Self::DeepLX(_) | Self::DeepL(_) | Self::OpenAICompatible(_) => true,
        }
    }
    async fn translate(
        &self,
        text: &str,
        source: Option<SourceLanguage>,
    ) -> Result<String, QwenMTClientError> {
        match self {
            Self::Disabled => Err(QwenMTClientError::MissingTextTranslation),
            Self::Qwen(client, _) => client.translate(text, source, &[]).await,
            Self::DeepL(client) => client
                .translate(text, source)
                .await
                .map_err(QwenMTClientError::DeepL),
            Self::DeepLX(client) => client
                .translate(text, source)
                .await
                .map_err(QwenMTClientError::DeepLX),
            Self::OpenAICompatible(client) => client
                .translate(text, source)
                .await
                .map_err(QwenMTClientError::OpenAICompatible),
        }
    }
    async fn translate_streaming(
        &self,
        text: &str,
        source: Option<SourceLanguage>,
        on_partial: impl Fn(String) + Send + Sync,
    ) -> Result<String, QwenMTClientError> {
        match self {
            Self::Disabled => Err(QwenMTClientError::MissingTextTranslation),
            Self::Qwen(client, _) => {
                client
                    .translate_streaming(text, source, &[], on_partial)
                    .await
            }
            Self::DeepLX(client) => client
                .translate(text, source)
                .await
                .map_err(QwenMTClientError::DeepLX),
            Self::DeepL(client) => client
                .translate(text, source)
                .await
                .map_err(QwenMTClientError::DeepL),
            Self::OpenAICompatible(client) => client
                .translate(text, source)
                .await
                .map_err(QwenMTClientError::OpenAICompatible),
        }
    }
}

#[derive(Clone)]
pub struct HighQualityTranslationClient {
    asr_client: RecognitionClient,
    mt: Arc<TextTranslationClient>,
    source_language: SourceLanguage,
    target_language: TargetLanguage,
    translates_audio: bool,
    events: ProviderEventSender,
    inner: Arc<Mutex<Inner>>,
    asr_bridge: Arc<Mutex<ASRBridgeState>>,
    content_operation: Arc<Mutex<()>>,
    preview_epoch: Arc<AtomicU64>,
    mt_work_allowed: Arc<AtomicBool>,
    translation_latency: Arc<std::sync::Mutex<Option<TranslationLatency>>>,
    stable_draft_delay: Duration,
    maximum_wait_delay: Duration,
    streams_finals: bool,
}

impl HighQualityTranslationClient {
    pub fn set_audio_pending_gate(&self, gate: crate::core::pending_pcm::PendingPcmGate) {
        self.asr_client.set_audio_pending_gate(gate);
    }

    #[cfg(test)]
    pub fn set_network(&mut self, network: ProviderNetwork) -> Result<(), ProviderNetworkError> {
        self.set_stage_networks(network.clone(), network)
    }

    /// Install independent immutable routes before connecting.
    pub fn set_stage_networks(
        &mut self,
        speech: ProviderNetwork,
        text: ProviderNetwork,
    ) -> Result<(), ProviderNetworkError> {
        self.asr_client.set_network(speech)?;
        Arc::make_mut(&mut self.mt).set_network(text)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        api_key: &str,
        source_language: SourceLanguage,
        target_language: TargetLanguage,
        final_model: QwenMTModel,
        stable_draft_delay: Duration,
        maximum_wait_delay: Duration,
        long_incomplete_commit_threshold: usize,
        events: ProviderEventSender,
    ) -> Result<Self, QwenMTClientError> {
        let asr_client = RecognitionClient::alibaba(api_key, source_language)
            .map_err(|_| QwenMTClientError::MissingAPIKey)?;
        let streams_finals = final_model != QwenMTModel::Plus;
        let domain_hint = Some(
            crate::core::protocols::qwen_mt::QwenMTDomainHint::spoken_dialogue(
                source_language,
                target_language,
            ),
        );
        let filler_terms = crate::core::protocols::qwen_mt::QwenMTDomainHint::filler_terms(
            source_language,
            target_language,
        );
        let mt = QwenMTClient::new(
            api_key,
            source_language,
            target_language,
            final_model,
            domain_hint,
            filler_terms,
            Duration::from_secs(8),
        )?;
        Ok(Self::from_components(
            asr_client,
            TextTranslationClient::Qwen(mt, final_model),
            source_language,
            target_language,
            stable_draft_delay,
            maximum_wait_delay,
            long_incomplete_commit_threshold,
            streams_finals,
            events,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn from_components(
        asr_client: RecognitionClient,
        mt: TextTranslationClient,
        source_language: SourceLanguage,
        target_language: TargetLanguage,
        stable_draft_delay: Duration,
        maximum_wait_delay: Duration,
        long_incomplete_commit_threshold: usize,
        streams_finals: bool,
        events: ProviderEventSender,
    ) -> Self {
        Self {
            asr_client,
            mt: Arc::new(mt),
            source_language,
            target_language,
            translates_audio: target_language.translates_audio(),
            events,
            inner: Arc::new(Mutex::new(Inner {
                committer: ASRDraftCommitter::new(long_incomplete_commit_threshold),
                latest_draft_language: None,
                draft_revision: 0,
                next_confirmation_id: 0,
                last_server_final: None,
                last_source_utterance_id: None,
                current_source_utterance_id: None,
                final_queue: VecDeque::new(),
                active_final: None,
                draft_stability_task: None,
                draft_maximum_wait_task: None,
                preview_task: None,
                preview_candidate: None,
                preview_http_pending: None,
                pending_preview_revision: None,
                final_worker: None,
                next_task_id: 0,
                pipeline_failed: false,
                final_completion_in_progress: false,
                deferred_overload: false,
                last_draft_log_at: None,
                mt_cooldown: None,
                mt_failure_streak: 0,
                preview_request_pacer: PreviewRequestPacer::default(),
            })),
            asr_bridge: Arc::new(Mutex::new(ASRBridgeState::default())),
            content_operation: Arc::new(Mutex::new(())),
            preview_epoch: Arc::new(AtomicU64::new(0)),
            mt_work_allowed: Arc::new(AtomicBool::new(true)),
            translation_latency: Default::default(),
            stable_draft_delay,
            maximum_wait_delay,
            streams_finals,
        }
    }

    pub fn new_deeplx(
        asr_key: &str,
        endpoint: &str,
        token: &str,
        source: SourceLanguage,
        target: TargetLanguage,
        events: ProviderEventSender,
    ) -> Result<Self, QwenMTClientError> {
        let asr = RecognitionClient::alibaba(asr_key, source)
            .map_err(|_| QwenMTClientError::MissingAPIKey)?;
        let mt = TextTranslationClient::DeepLX(
            crate::clients::deeplx_client::DeepLXClient::new(endpoint, token, source, target)
                .map_err(QwenMTClientError::DeepLX)?,
        );
        Ok(Self::from_components(
            asr,
            mt,
            source,
            target,
            Duration::from_millis(250),
            Duration::from_millis(1_000),
            12,
            false,
            events,
        ))
    }

    pub fn new_deepl(
        asr_key: &str,
        api_key: &str,
        source: SourceLanguage,
        target: TargetLanguage,
        events: ProviderEventSender,
    ) -> Result<Self, QwenMTClientError> {
        let asr = RecognitionClient::alibaba(asr_key, source)
            .map_err(|_| QwenMTClientError::MissingAPIKey)?;
        let mt = TextTranslationClient::DeepL(
            crate::clients::deepl_client::DeepLClient::new(api_key, source, target)
                .map_err(QwenMTClientError::DeepL)?,
        );
        Ok(Self::from_components(
            asr,
            mt,
            source,
            target,
            Duration::from_millis(250),
            Duration::from_millis(1_000),
            12,
            false,
            events,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_openai_compatible(
        asr_key: &str,
        endpoint: &str,
        api_key: &str,
        model: &str,
        source: SourceLanguage,
        target: TargetLanguage,
        events: ProviderEventSender,
    ) -> Result<Self, QwenMTClientError> {
        let asr = RecognitionClient::alibaba(asr_key, source)
            .map_err(|_| QwenMTClientError::MissingAPIKey)?;
        let mt = TextTranslationClient::OpenAICompatible(
            crate::clients::openai_compatible_client::OpenAICompatibleClient::new(
                endpoint, api_key, model, source, target,
            )
            .map_err(QwenMTClientError::OpenAICompatible)?,
        );
        Ok(Self::from_components(
            asr,
            mt,
            source,
            target,
            Duration::from_millis(250),
            Duration::from_millis(1_000),
            12,
            false,
            events,
        ))
    }

    /// Custom recognition never supplies a key to a built-in translation service.
    pub fn new_custom(
        configuration: &LiveTranslationConfiguration,
        events: ProviderEventSender,
    ) -> Result<Self, QwenMTClientError> {
        let ProviderCredentials::CustomSpeech {
            endpoint,
            model,
            api_key,
        } = &configuration.credentials
        else {
            return Err(QwenMTClientError::RequestFailed {
                status_code: 0,
                message: "Invalid speech recognition configuration.".into(),
            });
        };
        let source = configuration.source_language;
        let target = configuration.target_language;
        let asr =
            RecognitionClient::custom(configuration.provider, endpoint, model, api_key, source)
                .map_err(recognition_error)?;
        let mt = if !target.translates_audio() {
            TextTranslationClient::Disabled
        } else {
            match configuration
                .text_credentials
                .as_ref()
                .ok_or(QwenMTClientError::MissingTextTranslation)?
            {
                TextTranslationCredentials::DeepL { api_key } => TextTranslationClient::DeepL(
                    crate::clients::deepl_client::DeepLClient::new(api_key, source, target)
                        .map_err(QwenMTClientError::DeepL)?,
                ),
                TextTranslationCredentials::DeepLX { endpoint, token } => {
                    TextTranslationClient::DeepLX(
                        crate::clients::deeplx_client::DeepLXClient::new(
                            endpoint, token, source, target,
                        )
                        .map_err(QwenMTClientError::DeepLX)?,
                    )
                }
                TextTranslationCredentials::OpenAICompatible {
                    endpoint,
                    model,
                    api_key,
                }
                | TextTranslationCredentials::ChatMock {
                    endpoint,
                    model,
                    api_key,
                } => TextTranslationClient::OpenAICompatible(
                    crate::clients::openai_compatible_client::OpenAICompatibleClient::new(
                        endpoint, api_key, model, source, target,
                    )
                    .map_err(QwenMTClientError::OpenAICompatible)?,
                ),
            }
        };
        Ok(Self::from_components(
            asr,
            mt,
            source,
            target,
            Duration::from_millis(250),
            Duration::from_millis(1_000),
            12,
            false,
            events,
        ))
    }

    /// Connects the recognizer and resets all draft/final workers.
    pub async fn connect(&self) -> Result<(), QwenMTClientError> {
        self.mt_work_allowed.store(false, Ordering::SeqCst);
        let budget = self
            .inner
            .lock()
            .await
            .preview_request_pacer
            .export_budget();
        self.reset_draft_state().await;
        self.cancel_final_translations().await;
        self.restore_request_budget(budget).await;
        self.disconnect_asr_bridge().await;

        let task_id = Uuid::new_v4().simple().to_string();
        let (asr_tx, asr_rx) = provider_event_channel();
        if let Some((source, generation)) = self.events.debug_context() {
            asr_tx.set_recognition_debug_context(source, generation);
        }
        self.asr_client.set_event_sender(asr_tx).await;
        self.asr_client
            .connect(&task_id)
            .await
            .map_err(recognition_error)?;

        self.mt_work_allowed.store(true, Ordering::SeqCst);
        self.install_asr_bridge(asr_rx).await;
        Ok(())
    }

    pub async fn send_audio(&self, pcm_data: &[u8]) -> Result<(), QwenMTClientError> {
        self.asr_client
            .send_audio(pcm_data)
            .await
            .map_err(recognition_error)
    }

    pub async fn ping(&self, timeout: Duration) -> Result<(), QwenMTClientError> {
        self.asr_client
            .ping(timeout)
            .await
            .map_err(recognition_error)
    }

    /// Latest current successful preview/final request, including retries but
    /// excluding ASR, queue waiting and a pre-existing service cooldown.
    pub fn translation_latency(&self) -> Option<TranslationLatency> {
        if !self.translates_audio {
            return None;
        }
        *self.translation_latency.lock().unwrap()
    }

    pub(crate) async fn suspend_request_budget(&self) -> MTRequestBudget {
        // Stop new starts before sampling; any slot already claimed is charged
        // in the same inner lock before the actual HTTP await.
        self.mt_work_allowed.store(false, Ordering::SeqCst);
        self.inner
            .lock()
            .await
            .preview_request_pacer
            .export_budget()
    }

    pub(crate) async fn restore_request_budget(&self, budget: MTRequestBudget) {
        let mut inner = self.inner.lock().await;
        inner.preview_request_pacer.restore_budget(budget);
        let now = std::time::Instant::now();
        inner.mt_cooldown = budget
            .shared_cooldown_until()
            .filter(|until| *until > now)
            .map(|until| MTCooldown {
                until: tokio::time::Instant::from_std(until),
                reason: if budget.is_preview_suppressed(now) {
                    TranslationRecoveryReason::RateLimited
                } else {
                    TranslationRecoveryReason::TemporarilyUnavailable
                },
            });
    }

    pub async fn finish(&self) {
        // A user stop during service backoff cancels MT immediately. New ASR
        // teardown finals must not restart requests while the bridge drains.
        let has_mt_cooldown = { self.inner.lock().await.mt_cooldown.is_some() };
        if has_mt_cooldown {
            self.mt_work_allowed.store(false, Ordering::SeqCst);
            self.cancel_replaceable_work().await;
            self.cancel_final_translations().await;
        }
        self.asr_client.finish(ASR_BRIDGE_FINISH_TIMEOUT).await;
        if !self
            .wait_for_asr_bridge_finish(ASR_BRIDGE_FINISH_TIMEOUT)
            .await
        {
            pipeline_log!("audio3 asr bridge finish timed out");
        }
        if self.mt_work_allowed.load(Ordering::SeqCst) {
            self.flush_pending_draft().await;
            self.wait_for_final_translations(Duration::from_millis(
                translation_policy::FINISH_DRAIN_TIMEOUT_MS,
            ))
            .await;
        }
        self.reset_draft_state().await;
        self.cancel_final_translations().await;
        self.disconnect_asr_bridge().await;
    }

    pub async fn disconnect(&self) {
        self.mt_work_allowed.store(false, Ordering::SeqCst);
        self.reset_draft_state().await;
        self.cancel_final_translations().await;
        self.asr_client.disconnect().await;
        self.disconnect_asr_bridge().await;
    }

    // MARK: ASR event handling

    pub fn content_revision(&self) -> u64 {
        self.events.content_revision()
    }

    /// Discards local work without disconnecting ASR or refunding an already
    /// started request's spacing, quota suppression or shared cooldown.
    pub async fn clear_content(&self) -> u64 {
        let _content = self.content_operation.lock().await;
        let revision = {
            let mut inner = self.inner.lock().await;
            abort_task(&mut inner.draft_stability_task);
            abort_task(&mut inner.draft_maximum_wait_task);
            self.cancel_preview(&mut inner, PreviewCancellationReason::Reset);
            self.advance_preview_epoch();
            abort_task(&mut inner.final_worker);
            inner.final_queue.clear();
            inner.active_final = None;
            inner.final_completion_in_progress = false;
            inner.deferred_overload = false;
            inner.committer.reset();
            inner.latest_draft_language = None;
            next_nonzero(&mut inner.draft_revision);
            // Keep provider final-ID/replay watermarks and quota state.
            inner.preview_request_pacer.finish_utterance();
            *self.translation_latency.lock().unwrap() = None;
            self.events.advance_content_revision()
        };
        self.asr_client.clear_content().await;
        revision
    }

    async fn handle_asr_event(&self, event: LiveTranslateServerEvent) {
        if !self.mt_work_allowed.load(Ordering::SeqCst)
            && matches!(
                event,
                LiveTranslateServerEvent::SourceDraft { .. }
                    | LiveTranslateServerEvent::SourceUtteranceDraft { .. }
                    | LiveTranslateServerEvent::SourceFinal { .. }
                    | LiveTranslateServerEvent::SourceUtteranceFinal { .. }
            )
        {
            return;
        }
        if !event.text_within_limit() {
            self.emit(LiveTranslateServerEvent::text_limit_error());
            return;
        }
        let (event, draft_source_id) = match event {
            LiveTranslateServerEvent::SourceUtteranceDraft {
                utterance_id,
                text,
                language,
            } => {
                if !self.prepare_identified_draft(utterance_id).await {
                    return;
                }
                (
                    LiveTranslateServerEvent::SourceDraft { text, language },
                    Some(utterance_id),
                )
            }
            event => (event, None),
        };
        match event {
            LiveTranslateServerEvent::SourceDraft { text, language } => {
                let text = trim(&text);
                if text.is_empty() {
                    return;
                }
                let now = tokio::time::Instant::now();
                let same_language = self
                    .target_language
                    .matches_reported_asr(language.as_deref());
                let (uncommitted_text, has_pending, revision, log_due, passed_through) = {
                    let mut inner = self.inner.lock().await;
                    let uncommitted = inner.committer.update_draft(&text);
                    inner.current_source_utterance_id = draft_source_id;
                    inner.latest_draft_language = language.clone();
                    next_nonzero(&mut inner.draft_revision);
                    let has_pending = inner.committer.has_pending_text();
                    let log_due = inner
                        .last_draft_log_at
                        .is_none_or(|at| now.duration_since(at).as_millis() >= 1000);
                    if log_due {
                        inner.last_draft_log_at = Some(now);
                    }
                    // Publish under the same short state lock used when
                    // preview/final workers claim a source line. Otherwise a
                    // timer could claim it after this check but before emit.
                    let passed_through = has_pending
                        && same_language
                        && !inner.pipeline_failed
                        && !inner.final_lane_busy();
                    if passed_through {
                        abort_task(&mut inner.draft_stability_task);
                        abort_task(&mut inner.draft_maximum_wait_task);
                        self.cancel_preview(&mut inner, PreviewCancellationReason::SameLanguage);
                        self.advance_preview_epoch();
                        *self.translation_latency.lock().unwrap() = None;
                        self.emit(source_draft_event(
                            draft_source_id,
                            uncommitted.clone(),
                            language.clone(),
                        ));
                        self.emit(LiveTranslateServerEvent::TranslationDraft(
                            uncommitted.clone(),
                        ));
                    } else if has_pending
                        && (!self.translates_audio
                            || (!inner.final_lane_busy()
                                && inner.preview_task.is_none()
                                && inner
                                    .preview_request_pacer
                                    .has_changed(PreviewCandidate::new(
                                        &uncommitted,
                                        language.as_deref(),
                                        0,
                                    ))))
                    {
                        // The preview gate skips identical words and cosmetic
                        // ASR revisions. Preserve their completed translation
                        // instead of clearing it for work that will not run.
                        self.emit(source_draft_event(
                            draft_source_id,
                            uncommitted.clone(),
                            language.clone(),
                        ));
                        if self.translates_audio {
                            self.emit(LiveTranslateServerEvent::TranslationDraft(String::new()));
                        }
                    }
                    (
                        uncommitted,
                        has_pending,
                        inner.draft_revision,
                        log_due,
                        passed_through,
                    )
                };
                if log_due {
                    pipeline_log!(
                        "audio3 asr draft length={} pendingLength={} language={}",
                        text.chars().count(),
                        uncommitted_text.chars().count(),
                        language
                            .as_deref()
                            .unwrap_or(self.source_language.raw_value())
                    );
                }
                if !has_pending {
                    return;
                }
                if !self.translates_audio {
                    self.emit(LiveTranslateServerEvent::TranslationDraft(uncommitted_text));
                } else if !passed_through {
                    self.schedule_draft_finalization(revision).await;
                }
            }
            LiveTranslateServerEvent::SourceFinal { text, language } => {
                self.handle_server_final(text, language, None).await;
            }
            LiveTranslateServerEvent::SourceUtteranceFinal {
                utterance_id,
                text,
                language,
            } => {
                self.handle_server_final(text, language, Some(utterance_id))
                    .await;
            }
            other => self.emit(other),
        }
    }

    async fn prepare_identified_draft(&self, source_utterance_id: u64) -> bool {
        if source_utterance_id == 0 {
            return false;
        }
        let mut inner = self.inner.lock().await;
        if inner
            .last_source_utterance_id
            .is_some_and(|id| source_utterance_id <= id)
            || inner
                .current_source_utterance_id
                .is_some_and(|id| source_utterance_id < id)
        {
            pipeline_log!("audio3 asr draft rejected reason=obsolete_sentence");
            return false;
        }
        if inner.current_source_utterance_id == Some(source_utterance_id) {
            return true;
        }
        // Draft revisions within one real sentence may finish their HTTP
        // request. A different server ID is a genuine boundary, including its
        // documented empty sentence_begin; it cannot inherit that old request.
        let had_previous_draft = inner.current_source_utterance_id.is_some()
            || inner.committer.has_pending_text()
            || inner.preview_task.is_some();
        abort_task(&mut inner.draft_stability_task);
        abort_task(&mut inner.draft_maximum_wait_task);
        self.cancel_preview(&mut inner, PreviewCancellationReason::SourceBoundary);
        self.advance_preview_epoch();
        inner.committer.reset();
        inner.latest_draft_language = None;
        inner.current_source_utterance_id = Some(source_utterance_id);
        next_nonzero(&mut inner.draft_revision);
        // Forget only the old candidate identity, never its consumed starts,
        // shared cooldown or 429 suppression. Durable final work is untouched.
        inner.preview_request_pacer.finish_utterance();
        if had_previous_draft {
            self.emit(LiveTranslateServerEvent::SubtitlePreviewCleared);
        }
        // Preserve even an empty begin in the controller's ownership state.
        // Its latest-value slot will be replaced by actual words when present.
        self.emit(source_draft_event(
            Some(source_utterance_id),
            String::new(),
            None,
        ));
        true
    }

    async fn handle_server_final(
        &self,
        text: String,
        language: Option<String>,
        source_utterance_id: Option<u64>,
    ) {
        let text = trim(&text);
        if text.is_empty() {
            return;
        }
        let Some(utterance_revision) = self.prepare_server_final(&text, source_utterance_id).await
        else {
            pipeline_log!("audio3 asr final deduplicated");
            return;
        };
        pipeline_log!(
            "audio3 asr final length={} language={} queuedFinals={}",
            text.chars().count(),
            language
                .as_deref()
                .unwrap_or(self.source_language.raw_value()),
            self.inner.lock().await.final_queue.len()
        );
        self.enqueue_final(
            text,
            language,
            FinalBoundary::ServerFinal,
            utterance_revision,
            source_utterance_id,
        )
        .await;
    }

    async fn prepare_server_final(
        &self,
        text: &str,
        source_utterance_id: Option<u64>,
    ) -> Option<u64> {
        if !crate::core::models::subtitle_text_within_limit(text) {
            return None;
        }
        let mut inner = self.inner.lock().await;
        // Reject a replay before it can cancel the next sentence's preview or
        // consume its draft. Audio3's documented positive IDs increase per task.
        let source_utterance_id = source_utterance_id.filter(|id| *id > 0);
        if source_utterance_id.is_some_and(|id| {
            inner
                .last_source_utterance_id
                .is_some_and(|last| id <= last)
        }) {
            return None;
        }
        abort_task(&mut inner.draft_stability_task);
        abort_task(&mut inner.draft_maximum_wait_task);
        self.cancel_preview(&mut inner, PreviewCancellationReason::SourceFinal);
        self.advance_preview_epoch();

        let utterance_revision = inner.draft_revision;
        let duplicate = source_utterance_id.is_none()
            && inner
                .last_server_final
                .as_ref()
                .is_some_and(|(last, revision)| last == text && *revision == utterance_revision);
        let retains_newer_draft = source_utterance_id.is_some_and(|final_id| {
            inner
                .current_source_utterance_id
                .is_some_and(|draft_id| draft_id > final_id)
        });
        if !retains_newer_draft {
            inner.committer.reset();
            inner.latest_draft_language = None;
            inner.current_source_utterance_id = None;
            inner.preview_request_pacer.finish_utterance();
        }
        if duplicate {
            return None;
        }

        next_nonzero(&mut inner.draft_revision);
        inner.last_server_final = Some((text.to_string(), inner.draft_revision));
        if let Some(id) = source_utterance_id {
            inner.last_source_utterance_id = Some(id);
        }
        Some(next_nonzero(&mut inner.next_confirmation_id))
    }

    // MARK: Replaceable preview lane

    async fn schedule_draft_finalization(&self, revision: u64) {
        let mut inner = self.inner.lock().await;
        if inner.pipeline_failed
            || revision != inner.draft_revision
            || !inner.committer.has_pending_text()
            || inner.final_lane_busy()
        {
            return;
        }

        abort_task(&mut inner.draft_stability_task);
        let stable_id = next_nonzero(&mut inner.next_task_id);
        let stable_self = self.clone();
        let stable_delay = self.stable_draft_delay;
        let stable_task = tokio::spawn(async move {
            tokio::time::sleep(stable_delay).await;
            stable_self
                .handle_draft_timer(DraftTimerKind::Stable, stable_id, revision)
                .await;
        });
        inner.draft_stability_task = Some(TaskSlot {
            id: stable_id,
            handle: stable_task,
        });

        if inner.draft_maximum_wait_task.is_none() {
            let maximum_id = next_nonzero(&mut inner.next_task_id);
            let maximum_self = self.clone();
            let maximum_delay = self.maximum_wait_delay;
            let maximum_task = tokio::spawn(async move {
                tokio::time::sleep(maximum_delay).await;
                maximum_self
                    .handle_draft_timer(DraftTimerKind::Maximum, maximum_id, revision)
                    .await;
            });
            inner.draft_maximum_wait_task = Some(TaskSlot {
                id: maximum_id,
                handle: maximum_task,
            });
        }
    }

    async fn handle_draft_timer(
        &self,
        kind: DraftTimerKind,
        timer_id: u64,
        scheduled_revision: u64,
    ) {
        let preview = {
            let mut inner = self.inner.lock().await;
            let slot_matches = match kind {
                DraftTimerKind::Stable => inner
                    .draft_stability_task
                    .as_ref()
                    .is_some_and(|slot| slot.id == timer_id),
                DraftTimerKind::Maximum => inner
                    .draft_maximum_wait_task
                    .as_ref()
                    .is_some_and(|slot| slot.id == timer_id),
            };
            if !slot_matches {
                return;
            }

            match kind {
                DraftTimerKind::Stable => {
                    drop(inner.draft_stability_task.take());
                    if scheduled_revision != inner.draft_revision {
                        return;
                    }
                }
                DraftTimerKind::Maximum => {
                    drop(inner.draft_maximum_wait_task.take());
                    abort_task(&mut inner.draft_stability_task);
                }
            }

            if inner.pipeline_failed {
                return;
            }
            if inner.final_lane_busy() {
                return;
            }
            // Both timers preview the whole meaningful draft. Preferring its
            // complete-sentence prefix can rewind a previously displayed tail.
            let text = inner.committer.preview_latest_draft(false);
            text.map(|text| {
                (
                    text,
                    inner.latest_draft_language.clone(),
                    inner.draft_revision,
                    match kind {
                        DraftTimerKind::Stable => "stable-draft",
                        DraftTimerKind::Maximum => "maximum-wait",
                    },
                )
            })
        };

        if let Some((text, language, revision, boundary)) = preview {
            pipeline_log!(
                "mt preview scheduled boundary={} length={} language={}",
                boundary,
                text.chars().count(),
                language
                    .as_deref()
                    .unwrap_or(self.source_language.raw_value())
            );
            self.start_preview(text, language, revision).await;
        }
    }

    async fn start_preview(&self, text: String, language: Option<String>, revision: u64) {
        if !crate::core::models::subtitle_text_within_limit(&text) {
            self.emit(LiveTranslateServerEvent::text_limit_error());
            return;
        }
        let mut inner = self.inner.lock().await;
        self.start_preview_locked(&mut inner, text, language, revision);
    }

    // Synchronous under the state lock so completion can hand off to exactly
    // one newest candidate without a recursive async spawn/cleanup chain.
    fn start_preview_locked(
        &self,
        inner: &mut Inner,
        text: String,
        language: Option<String>,
        revision: u64,
    ) {
        if inner.pipeline_failed
            || !self.mt_work_allowed.load(Ordering::SeqCst)
            || revision != inner.draft_revision
            || !inner.committer.has_pending_text()
            || inner.final_lane_busy()
        {
            return;
        }
        let candidate = PreviewCandidate::new(&text, language.as_deref(), 0);
        if inner.preview_task.is_some()
            && inner
                .preview_candidate
                .is_some_and(|active| active.matches_content(candidate))
        {
            // Cosmetic ASR updates must not cancel an identical waiting or
            // in-flight request and restart its deadline.
            if inner.pending_preview_revision.take().is_some() {
                pipeline_log!("mt preview queued discarded reason=reverted");
            }
            return;
        }
        if inner.preview_http_pending.is_some() {
            // A draft revision is not an utterance boundary. Cancelling an
            // already-started request on each revision starves fast speech.
            // Finals/reset/stop still cancel and advance the owner epoch.
            let replaced = inner.pending_preview_revision.replace(revision).is_some();
            pipeline_log!(
                "mt preview queued inFlight=true replaced={} revision={}",
                replaced,
                revision
            );
            return;
        }
        if !self
            .target_language
            .matches_reported_asr(language.as_deref())
            && (!inner.preview_request_pacer.has_changed(candidate)
                || !inner
                    .preview_request_pacer
                    .suppression_remaining(std::time::Instant::now())
                    .is_zero())
        {
            if inner.preview_task.is_some() {
                // A -> B waiting -> A skips a duplicate request for A,
                // but must still cancel B before it can claim a source.
                self.cancel_preview(inner, PreviewCancellationReason::RevertedWaiter);
                self.advance_preview_epoch();
                self.emit(source_draft_event(
                    inner.current_source_utterance_id,
                    text.clone(),
                    language.clone(),
                ));
                self.emit(LiveTranslateServerEvent::TranslationDraft(String::new()));
            }
            return;
        }

        self.cancel_preview(inner, PreviewCancellationReason::ReplacedWaiter);
        let preview_id = self.advance_preview_epoch();
        let preview_self = self.clone();
        let task = tokio::spawn(async move {
            preview_self.run_preview(preview_id, text, language).await;
        });
        inner.preview_task = Some(TaskSlot {
            id: preview_id,
            handle: task,
        });
        inner.preview_candidate = Some(candidate);
    }

    async fn run_preview(&self, preview_id: u64, text: String, language: Option<String>) {
        if !self.preview_is_current(preview_id) {
            return;
        }
        let source_utterance_id = self.inner.lock().await.current_source_utterance_id;
        if self
            .target_language
            .matches_reported_asr(language.as_deref())
        {
            {
                let inner = self.inner.lock().await;
                if !self.preview_is_current(preview_id)
                    || inner.pipeline_failed
                    || inner.final_lane_busy()
                    || inner
                        .preview_task
                        .as_ref()
                        .is_none_or(|task| task.id != preview_id)
                {
                    return;
                }
                *self.translation_latency.lock().unwrap() = None;
                self.emit_preview(
                    preview_id,
                    source_draft_event(source_utterance_id, text.clone(), language),
                );
                self.emit_preview(preview_id, LiveTranslateServerEvent::TranslationDraft(text));
            }
            self.clear_preview_task(preview_id, false).await;
            return;
        }
        let partial_handler = self.preview_partial_handler(preview_id);
        let deadline = tokio::time::Instant::now() + MAX_PREVIEW_REQUEST_AGE;
        let result = self
            .translate_with_retry(
                &text,
                language.as_deref(),
                deadline,
                partial_handler,
                TranslationWorkOwner::Preview(preview_id),
                TranslationEvidenceIdentity {
                    #[cfg(any(test, feature = "development-debugger"))]
                    source_utterance_id,
                    #[cfg(any(test, feature = "development-debugger"))]
                    pair_id: None,
                    #[cfg(any(test, feature = "development-debugger"))]
                    final_boundary: None,
                },
            )
            .await;

        let mut completed = false;
        if self.preview_is_current(preview_id) {
            match result {
                Ok(translation) => {
                    let inner = self.inner.lock().await;
                    if self.translation_work_is_current(
                        &inner,
                        TranslationWorkOwner::Preview(preview_id),
                    ) && !translation.text.trim().is_empty()
                    {
                        completed = true;
                        let translation_length = translation.text.chars().count();
                        *self.translation_latency.lock().unwrap() =
                            translation
                                .request_ms
                                .map(|milliseconds| TranslationLatency {
                                    milliseconds,
                                    kind: TranslationLatencyKind::Request,
                                });
                        self.emit_preview(
                            preview_id,
                            LiveTranslateServerEvent::TranslationDraft(translation.text.clone()),
                        );
                        self.emit_preview(
                            preview_id,
                            LiveTranslateServerEvent::SubtitlePreviewPair {
                                source_utterance_id,
                                source: text.clone(),
                                language: language.clone(),
                                translation: translation.text,
                            },
                        );
                        pipeline_log!(
                            "mt preview completed requestMs={} sourceLength={} translationLength={}",
                            translation.request_ms.unwrap_or_default(),
                            text.chars().count(),
                            translation_length
                        );
                    }
                }
                Err(error) => {
                    pipeline_log!("mt preview failed error={}", error.diagnostic_label());
                    let mut inner = self.inner.lock().await;
                    if self.translation_work_is_current(
                        &inner,
                        TranslationWorkOwner::Preview(preview_id),
                    ) {
                        self.finish_preview_http(&mut inner, preview_id);
                        if error.is_authentication_failure()
                            || matches!(error, QwenMTClientError::UnsupportedSource)
                        {
                            inner.pipeline_failed = true;
                            self.handle_translation_failure(&error);
                        } else {
                            let reason = inner.mt_cooldown.map_or_else(
                                || {
                                    error.recovery_reason().unwrap_or(
                                        TranslationRecoveryReason::TemporarilyUnavailable,
                                    )
                                },
                                |cooldown| cooldown.reason,
                            );
                            self.emit(LiveTranslateServerEvent::TranslationDeferred(
                                TranslationRecovery {
                                    reason,
                                    retry_after_ms: inner
                                        .mt_cooldown
                                        .map_or(Duration::ZERO, |cooldown| {
                                            cooldown.until.saturating_duration_since(
                                                tokio::time::Instant::now(),
                                            )
                                        })
                                        .max(
                                            inner
                                                .preview_request_pacer
                                                .suppression_remaining(std::time::Instant::now()),
                                        )
                                        .as_millis()
                                        as u64,
                                    retry_scheduled: false,
                                },
                            ));
                            // Keep the shared service cooldown, but truthfully
                            // report that this exhausted preview has no timer.
                            drop(inner.preview_task.take());
                            inner.preview_candidate = None;
                            if inner.pending_preview_revision.take().is_some() {
                                pipeline_log!("mt preview queued discarded reason=failed");
                            }
                        }
                    }
                }
            }
        }
        self.clear_preview_task(preview_id, completed).await;
    }

    fn emit_preview(&self, preview_id: u64, event: LiveTranslateServerEvent) {
        let _ = self
            .events
            .send_if(event, || self.preview_is_current(preview_id));
    }

    fn preview_partial_handler(&self, preview_id: u64) -> PartialHandler {
        let client = self.clone();
        Arc::new(move |partial| {
            client.emit_preview(
                preview_id,
                LiveTranslateServerEvent::TranslationDraft(partial),
            );
        })
    }

    fn preview_is_current(&self, preview_id: u64) -> bool {
        self.mt_work_allowed.load(Ordering::SeqCst)
            && self.preview_epoch.load(Ordering::SeqCst) == preview_id
    }

    fn advance_preview_epoch(&self) -> u64 {
        let id = self
            .preview_epoch
            .fetch_add(1, Ordering::SeqCst)
            .wrapping_add(1);
        if id == 0 {
            self.preview_epoch
                .fetch_add(1, Ordering::SeqCst)
                .wrapping_add(1)
        } else {
            id
        }
    }

    async fn clear_preview_task(&self, preview_id: u64, resume_latest: bool) {
        let mut inner = self.inner.lock().await;
        if inner
            .preview_task
            .as_ref()
            .is_some_and(|task| task.id == preview_id)
        {
            self.finish_preview_http(&mut inner, preview_id);
            drop(inner.preview_task.take());
            inner.preview_candidate = None;
            let pending = inner.pending_preview_revision.take();
            if resume_latest && pending.is_some() && self.preview_is_current(preview_id) {
                // Re-read the latest whole draft: ASR may have updated it after
                // the queued timer fired. Do not retain a second text copy or
                // replay an obsolete intermediate candidate.
                if let Some(text) = inner.committer.preview_latest_draft(false) {
                    let language = inner.latest_draft_language.clone();
                    let revision = inner.draft_revision;
                    self.start_preview_locked(&mut inner, text, language, revision);
                }
            }
        }
    }

    fn finish_preview_http(&self, inner: &mut Inner, request_id: u64) {
        if inner.preview_http_pending == Some(request_id) {
            inner.preview_http_pending = None;
            self.emit_preview(
                request_id,
                LiveTranslateServerEvent::PreviewTranslationFinished { request_id },
            );
        }
    }

    fn cancel_preview(&self, inner: &mut Inner, reason: PreviewCancellationReason) {
        if inner.preview_task.is_some() {
            pipeline_log!(
                "mt preview cancelled inFlight={} queued={} reason={}",
                inner.preview_http_pending.is_some(),
                inner.pending_preview_revision.is_some(),
                reason.label()
            );
        }
        if let Some(request_id) = inner.cancel_preview() {
            self.emit_preview(
                request_id,
                LiveTranslateServerEvent::PreviewTranslationFinished { request_id },
            );
        }
    }

    // MARK: Durable final lane

    async fn enqueue_final(
        &self,
        text: String,
        language: Option<String>,
        boundary: FinalBoundary,
        utterance_revision: u64,
        source_utterance_id: Option<u64>,
    ) {
        if !crate::core::models::subtitle_text_within_limit(&text) {
            self.emit(LiveTranslateServerEvent::text_limit_error());
            return;
        }
        if !self.translates_audio {
            self.emit(LiveTranslateServerEvent::SubtitleConfirmedPair {
                utterance_id: utterance_revision,
                source_utterance_id,
                source: text.clone(),
                language,
                translation: text,
            });
            return;
        }

        let request = TranslationRequest {
            text,
            language,
            boundary,
            utterance_revision,
            source_utterance_id,
            content_revision: self.content_revision(),
            enqueued_at: tokio::time::Instant::now(),
        };
        let outcome = {
            let mut inner = self.inner.lock().await;
            if inner.pipeline_failed {
                return;
            }

            if inner
                .active_final
                .as_ref()
                .is_some_and(|active| active.matches(&request))
            {
                return;
            }
            if let Some(queued) = inner
                .final_queue
                .iter_mut()
                .find(|queued| queued.key().matches(&request))
            {
                if boundary == FinalBoundary::ServerFinal {
                    queued.language = request.language;
                    queued.boundary = FinalBoundary::ServerFinal;
                    queued.utterance_revision = request.utterance_revision;
                }
                return;
            }

            if !translation_policy::can_enqueue(inner.final_queue.len(), true) {
                if inner.final_completion_in_progress {
                    inner.deferred_overload = true;
                    EnqueueOutcome::DeferredOverload
                } else {
                    inner.pipeline_failed = true;
                    EnqueueOutcome::Overloaded
                }
            } else {
                pipeline_log!(
                    "mt final enqueued boundary={} depth={}",
                    boundary.label(),
                    inner.final_queue.len() + 1
                );
                inner.final_queue.push_back(request);
                EnqueueOutcome::Queued
            }
        };

        match outcome {
            EnqueueOutcome::Queued => self.start_final_worker_if_needed().await,
            EnqueueOutcome::Overloaded => {
                self.cancel_replaceable_work().await;
                pipeline_log!("mt final overload depth={MAX_FINAL_QUEUE_DEPTH}");
                self.emit_overload_error();
            }
            EnqueueOutcome::DeferredOverload => {}
        }
    }

    async fn start_final_worker_if_needed(&self) {
        let mut inner = self.inner.lock().await;
        if inner.pipeline_failed || inner.final_worker.is_some() || inner.final_queue.is_empty() {
            return;
        }
        let worker_id = next_nonzero(&mut inner.next_task_id);
        let self_arc = self.clone();
        let task = tokio::spawn(async move {
            self_arc.run_final_worker(worker_id).await;
        });
        inner.final_worker = Some(TaskSlot {
            id: worker_id,
            handle: task,
        });
    }

    async fn run_final_worker(&self, worker_id: u64) {
        loop {
            let request = {
                let mut inner = self.inner.lock().await;
                if inner
                    .final_worker
                    .as_ref()
                    .is_none_or(|task| task.id != worker_id)
                {
                    return;
                }
                if inner.pipeline_failed {
                    clear_task_if_id(&mut inner.final_worker, worker_id);
                    inner.active_final = None;
                    return;
                }
                match inner.final_queue.pop_front() {
                    Some(request) => {
                        abort_task(&mut inner.draft_stability_task);
                        abort_task(&mut inner.draft_maximum_wait_task);
                        self.cancel_preview(&mut inner, PreviewCancellationReason::FinalWorker);
                        self.advance_preview_epoch();
                        inner.active_final = Some(request.key());
                        Some(request)
                    }
                    None => {
                        clear_task_if_id(&mut inner.final_worker, worker_id);
                        inner.active_final = None;
                        None
                    }
                }
            };
            let Some(request) = request else {
                self.resume_pending_preview_if_final_lane_idle().await;
                return;
            };

            let started_at = tokio::time::Instant::now();
            let queue_age = started_at.saturating_duration_since(request.enqueued_at);
            if !translation_policy::can_start(queue_age.as_millis().try_into().unwrap_or(u64::MAX))
            {
                self.fail_final_worker_overload(worker_id, queue_age).await;
                return;
            }
            let same_language = self
                .target_language
                .matches_reported_asr(request.language.as_deref());
            if !same_language {
                pipeline_log!(
                    "mt final started boundary={} waitMs={} remaining={}",
                    request.boundary.label(),
                    queue_age.as_millis(),
                    self.inner.lock().await.final_queue.len()
                );
                self.emit_content(
                    request.content_revision,
                    LiveTranslateServerEvent::TranslationStarted,
                );
            }
            {
                let inner = self.inner.lock().await;
                // The latest-value source slot may already contain B, even
                // before its consumer observes it. Starting A's durable work
                // cannot replace that queued newer identity with old A.
                let newer_source = request.source_utterance_id.is_some_and(|id| {
                    inner
                        .current_source_utterance_id
                        .is_some_and(|current| current > id)
                });
                if !newer_source {
                    self.emit_content(
                        request.content_revision,
                        source_draft_event(
                            request.source_utterance_id,
                            request.text.clone(),
                            request.language.clone(),
                        ),
                    );
                }
            }
            // A server final can revise the preview's source. Its prior
            // translation must not remain beside the new request's source
            // while the first streamed partial is still pending.
            self.emit_content(
                request.content_revision,
                LiveTranslateServerEvent::TranslationDraft(String::new()),
            );

            let events = self.events.clone();
            let content_revision = request.content_revision;
            let partial_handler: PartialHandler = Arc::new(move |partial| {
                let _ = events.send_content(
                    content_revision,
                    LiveTranslateServerEvent::TranslationDraft(partial),
                );
            });
            let deadline = request.enqueued_at + MAX_FINAL_REQUEST_AGE;
            let result = if same_language {
                Ok(MeasuredTranslation {
                    text: request.text.clone(),
                    request_ms: None,
                })
            } else {
                self.translate_with_retry(
                    &request.text,
                    request.language.as_deref(),
                    deadline,
                    partial_handler,
                    TranslationWorkOwner::Final(worker_id),
                    TranslationEvidenceIdentity {
                        #[cfg(any(test, feature = "development-debugger"))]
                        source_utterance_id: request.source_utterance_id,
                        #[cfg(any(test, feature = "development-debugger"))]
                        pair_id: Some(request.utterance_revision),
                        #[cfg(any(test, feature = "development-debugger"))]
                        final_boundary: Some(request.boundary.label()),
                    },
                )
                .await
            };
            match result {
                Ok(translation) => {
                    let should_emit = {
                        let mut inner = self.inner.lock().await;
                        let owned = inner
                            .final_worker
                            .as_ref()
                            .is_some_and(|task| task.id == worker_id);
                        if inner.pipeline_failed
                            || !owned
                            || self.content_revision() != request.content_revision
                        {
                            if owned {
                                clear_task_if_id(&mut inner.final_worker, worker_id);
                                inner.active_final = None;
                            }
                            false
                        } else {
                            inner.final_completion_in_progress = true;
                            if same_language {
                                *self.translation_latency.lock().unwrap() = None;
                            } else if !translation.text.trim().is_empty() {
                                *self.translation_latency.lock().unwrap() = translation
                                    .request_ms
                                    .map(|milliseconds| TranslationLatency {
                                        milliseconds,
                                        kind: TranslationLatencyKind::Request,
                                    });
                            }
                            true
                        }
                    };
                    if !should_emit {
                        return;
                    }

                    self.emit_content(
                        request.content_revision,
                        LiveTranslateServerEvent::SubtitleConfirmedPair {
                            utterance_id: request.utterance_revision,
                            source_utterance_id: request.source_utterance_id,
                            source: request.text.clone(),
                            language: request.language.clone(),
                            translation: translation.text.clone(),
                        },
                    );

                    let deferred_overload = {
                        let mut inner = self.inner.lock().await;
                        if self.content_revision() != request.content_revision
                            || inner
                                .final_worker
                                .as_ref()
                                .is_none_or(|task| task.id != worker_id)
                        {
                            return;
                        }
                        inner.final_completion_in_progress = false;
                        inner.active_final = None;
                        if inner.deferred_overload {
                            inner.deferred_overload = false;
                            inner.pipeline_failed = true;
                            clear_task_if_id(&mut inner.final_worker, worker_id);
                            true
                        } else {
                            false
                        }
                    };
                    if deferred_overload {
                        self.cancel_replaceable_work().await;
                        pipeline_log!("mt final overload depth={MAX_FINAL_QUEUE_DEPTH}");
                        self.emit_overload_error();
                        return;
                    }

                    if same_language {
                        pipeline_log!(
                            "mt final skipped label=same_language boundary={} remaining={}",
                            request.boundary.label(),
                            self.inner.lock().await.final_queue.len()
                        );
                    } else {
                        pipeline_log!(
                            "mt final completed boundary={} requestMs={} remaining={} sourceLength={} translationLength={}",
                            request.boundary.label(),
                            translation.request_ms.unwrap_or_default(),
                            self.inner.lock().await.final_queue.len(),
                            request.text.chars().count(),
                            translation.text.chars().count()
                        );
                    }
                }
                Err(error) => {
                    let should_emit = {
                        let mut inner = self.inner.lock().await;
                        let owned = inner
                            .final_worker
                            .as_ref()
                            .is_some_and(|task| task.id == worker_id);
                        if owned {
                            inner.pipeline_failed = true;
                            inner.active_final = None;
                            clear_task_if_id(&mut inner.final_worker, worker_id);
                        }
                        owned
                    };
                    if should_emit {
                        pipeline_log!(
                            "mt final failed requestMs={} error={}",
                            started_at.elapsed().as_millis(),
                            error.diagnostic_label()
                        );
                        self.handle_translation_failure(&error);
                    }
                    return;
                }
            }
        }
    }

    async fn fail_final_worker_overload(&self, worker_id: u64, queue_age: Duration) {
        let should_emit = {
            let mut inner = self.inner.lock().await;
            let owned = inner
                .final_worker
                .as_ref()
                .is_some_and(|task| task.id == worker_id);
            if owned {
                inner.pipeline_failed = true;
                inner.active_final = None;
                clear_task_if_id(&mut inner.final_worker, worker_id);
            }
            owned
        };
        if should_emit {
            self.cancel_replaceable_work().await;
            pipeline_log!("mt final expired waitMs={}", queue_age.as_millis());
            self.emit_overload_error();
        }
    }

    async fn resume_pending_preview_if_final_lane_idle(&self) {
        let revision = {
            let inner = self.inner.lock().await;
            (!inner.pipeline_failed
                && !inner.final_lane_busy()
                && inner.committer.has_pending_text())
            .then_some(inner.draft_revision)
        };
        if let Some(revision) = revision {
            self.schedule_draft_finalization(revision).await;
        }
    }

    fn emit_overload_error(&self) {
        self.emit(LiveTranslateServerEvent::Error {
            code: OVERLOAD_ERROR_CODE.into(),
            message: OVERLOAD_ERROR_MESSAGE.into(),
        });
    }

    fn handle_translation_failure(&self, error: &QwenMTClientError) {
        let code = if error.is_authentication_failure() {
            "translation_authentication_failed"
        } else if let Some(reason) = error.recovery_reason() {
            match reason {
                TranslationRecoveryReason::RateLimited => "translation_rate_limited",
                TranslationRecoveryReason::TemporarilyUnavailable => {
                    "translation_temporarily_unavailable"
                }
            }
        } else if matches!(error, QwenMTClientError::UnsupportedSource) {
            "translation_source_unsupported"
        } else if matches!(error, QwenMTClientError::DeepL(_)) {
            "deepl_translation_failed"
        } else if matches!(error, QwenMTClientError::DeepLX(_)) {
            "deeplx_translation_failed"
        } else if matches!(error, QwenMTClientError::OpenAICompatible(_)) {
            "openai_compatible_translation_failed"
        } else {
            "translation_failed"
        };
        self.emit(LiveTranslateServerEvent::Error {
            code: code.into(),
            message: match code {
                "translation_rate_limited" => "translation_rate_limited".into(),
                "translation_temporarily_unavailable" => {
                    "translation_temporarily_unavailable".into()
                }
                "translation_authentication_failed" => "credential_authentication_failed".into(),
                _ => error.to_string(),
            },
        });
    }

    fn translation_work_is_current(&self, inner: &Inner, owner: TranslationWorkOwner) -> bool {
        if inner.pipeline_failed || !self.mt_work_allowed.load(Ordering::SeqCst) {
            return false;
        }
        match owner {
            TranslationWorkOwner::Preview(id) => {
                self.preview_is_current(id)
                    && !inner.final_lane_busy()
                    && inner
                        .preview_task
                        .as_ref()
                        .is_some_and(|task| task.id == id)
            }
            TranslationWorkOwner::Final(id) => inner
                .final_worker
                .as_ref()
                .is_some_and(|task| task.id == id),
        }
    }

    async fn wait_for_mt_request_slot(
        &self,
        deadline: tokio::time::Instant,
        owner: TranslationWorkOwner,
        text: &str,
        language: Option<&str>,
        first_request: bool,
        candidate: PreviewCandidate,
    ) -> Result<bool, QwenMTClientError> {
        let mut waited = false;
        loop {
            let wait_until = {
                let mut inner = self.inner.lock().await;
                if !self.translation_work_is_current(&inner, owner) {
                    return Err(QwenMTClientError::RequestTimedOut);
                }
                let now = tokio::time::Instant::now();
                if now >= deadline {
                    return Err(QwenMTClientError::RequestTimedOut);
                }
                let cooldown_until = match inner.mt_cooldown {
                    Some(cooldown) if cooldown.until > now => {
                        waited = true;
                        self.emit(LiveTranslateServerEvent::TranslationDeferred(
                            TranslationRecovery {
                                reason: cooldown.reason,
                                retry_after_ms: cooldown
                                    .until
                                    .saturating_duration_since(now)
                                    .as_millis()
                                    as u64,
                                retry_scheduled: true,
                            },
                        ));
                        cooldown.until
                    }
                    _ => {
                        inner.mt_cooldown = None;
                        now
                    }
                };
                let next_preview_at = match owner {
                    TranslationWorkOwner::Preview(_) => {
                        let next = if first_request {
                            inner
                                .preview_request_pacer
                                .next_candidate_start_at(now.into_std(), candidate)
                                .ok_or(QwenMTClientError::RequestTimedOut)?
                        } else {
                            inner.preview_request_pacer.next_start_at(now.into_std())
                        };
                        tokio::time::Instant::from_std(next)
                    }
                    TranslationWorkOwner::Final(_) => tokio::time::Instant::from_std(
                        inner
                            .preview_request_pacer
                            .next_shared_start_at(now.into_std()),
                    ),
                };
                let until = cooldown_until.max(next_preview_at);
                if until > now {
                    Some(until)
                } else {
                    if let TranslationWorkOwner::Preview(id) = owner {
                        inner
                            .preview_request_pacer
                            .record_candidate_start(now.into_std(), candidate);
                        // Preserve the displayed preview while a replacement
                        // waits. Claim its matching source only at HTTP start.
                        if first_request {
                            self.emit_preview(
                                id,
                                source_draft_event(
                                    inner.current_source_utterance_id,
                                    text.to_owned(),
                                    language.map(str::to_owned),
                                ),
                            );
                            self.emit_preview(
                                id,
                                LiveTranslateServerEvent::TranslationDraft(String::new()),
                            );
                        }
                        inner.preview_http_pending = Some(id);
                        self.emit_preview(
                            id,
                            LiveTranslateServerEvent::PreviewTranslationStarted { request_id: id },
                        );
                    } else {
                        inner
                            .preview_request_pacer
                            .record_final_start(now.into_std(), candidate);
                    }
                    if waited && matches!(owner, TranslationWorkOwner::Final(_)) {
                        self.emit(LiveTranslateServerEvent::TranslationStarted);
                    }
                    None
                }
            };
            let Some(until) = wait_until else {
                return Ok(waited);
            };
            if until >= deadline {
                return Err(QwenMTClientError::RequestTimedOut);
            }
            tokio::time::sleep_until(until).await;
        }
    }

    async fn translate_with_retry(
        &self,
        text: &str,
        language: Option<&str>,
        deadline: tokio::time::Instant,
        on_partial: PartialHandler,
        owner: TranslationWorkOwner,
        _identity: TranslationEvidenceIdentity,
    ) -> Result<MeasuredTranslation, QwenMTClientError> {
        if !self.mt.supports_reported_source(language) {
            return Err(QwenMTClientError::UnsupportedSource);
        }
        // tm_list is a set of translation style examples, not conversation
        // history. Feeding a previous full document to a new cumulative
        // prefix can bias the model toward that example's old full output.
        // The live path has no explicitly curated translation memory.
        let candidate = PreviewCandidate::new(text, language, 0);
        let source_override = if self.source_language == SourceLanguage::Automatic {
            language.and_then(|value| SourceLanguage::from_detected(Some(value)))
        } else {
            None
        };
        let mut attempt = 1usize;
        let mut first_request_at: Option<tokio::time::Instant> = None;

        loop {
            self.wait_for_mt_request_slot(
                deadline,
                owner,
                text,
                language,
                first_request_at.is_none(),
                candidate,
            )
            .await?;
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(QwenMTClientError::RequestTimedOut);
            }

            let handler = Arc::clone(&on_partial);
            let first_request_at = *first_request_at.get_or_insert_with(tokio::time::Instant::now);
            pipeline_log!(
                "mt request started lane={} attempt={} sourceLength={} memoryEntries=0",
                match owner {
                    TranslationWorkOwner::Preview(_) => "preview",
                    TranslationWorkOwner::Final(_) => "final",
                },
                attempt,
                text.chars().count()
            );
            #[cfg(any(test, feature = "development-debugger"))]
            let context = self.events.debug_context().map(|(source, generation)| {
                crate::development_content::RequestContext {
                    source,
                    generation,
                    revision: self.content_revision(),
                    owner: match owner {
                        TranslationWorkOwner::Preview(id) | TranslationWorkOwner::Final(id) => id,
                    },
                    preview: matches!(owner, TranslationWorkOwner::Preview(_)),
                    attempt,
                    request_id: 0,
                    source_utterance_id: _identity.source_utterance_id,
                    pair_id: _identity.pair_id,
                    final_boundary: _identity.final_boundary,
                }
            });
            #[cfg(not(any(test, feature = "development-debugger")))]
            let context = None;
            let evidence = crate::development_content::begin_attempt(context);
            let result = tokio::time::timeout(
                remaining,
                crate::development_content::scope_attempt(&evidence, async {
                    if self.streams_finals {
                        self.mt
                            .translate_streaming(text, source_override, move |partial| {
                                (handler)(partial)
                            })
                            .await
                    } else {
                        self.mt.translate(text, source_override).await
                    }
                }),
            )
            .await
            .unwrap_or(Err(QwenMTClientError::RequestTimedOut));
            // Persist the decoded attempt outcome before owner filtering. A
            // successful reply discarded by Clear/replacement is still evidence.
            evidence.complete(&result);

            match result {
                Ok(translation) => {
                    let mut inner = self.inner.lock().await;
                    if !self.translation_work_is_current(&inner, owner) {
                        return Err(QwenMTClientError::RequestTimedOut);
                    }
                    inner.mt_cooldown = None;
                    inner.mt_failure_streak = 0;
                    inner.preview_request_pacer.set_shared_cooldown(None);
                    return Ok(MeasuredTranslation {
                        text: translation,
                        request_ms: Some(first_request_at.elapsed().as_millis() as u64),
                    });
                }
                Err(error) => {
                    let Some(reason) = error.recovery_reason() else {
                        return Err(error);
                    };
                    let class = error.retry_class();
                    let (delay, failure_streak) = {
                        let mut inner = self.inner.lock().await;
                        if !self.translation_work_is_current(&inner, owner) {
                            return Err(error);
                        }
                        if let TranslationWorkOwner::Preview(id) = owner {
                            self.finish_preview_http(&mut inner, id);
                        }
                        inner.mt_failure_streak = inner.mt_failure_streak.saturating_add(1);
                        let failure_streak = inner.mt_failure_streak.max(attempt);
                        let delay = crate::core::protocols::qwen_mt::QwenMTRetryPolicy::delay(
                            &error,
                            failure_streak,
                        )
                        .expect("classified transient error has a retry delay");
                        let until = tokio::time::Instant::now() + delay;
                        inner.mt_cooldown = Some(MTCooldown { until, reason });
                        inner
                            .preview_request_pacer
                            .set_shared_cooldown(Some(until.into_std()));
                        if reason == TranslationRecoveryReason::RateLimited {
                            inner
                                .preview_request_pacer
                                .suppress_after_rate_limit(std::time::Instant::now());
                        }
                        let pauses_preview = matches!(owner, TranslationWorkOwner::Preview(_))
                            && reason == TranslationRecoveryReason::RateLimited;
                        *self.translation_latency.lock().unwrap() = None;
                        self.emit(LiveTranslateServerEvent::TranslationDeferred(
                            TranslationRecovery {
                                reason,
                                retry_after_ms: if pauses_preview {
                                    inner
                                        .preview_request_pacer
                                        .suppression_remaining(std::time::Instant::now())
                                        .as_millis() as u64
                                } else {
                                    delay.as_millis() as u64
                                },
                                retry_scheduled: !pauses_preview,
                            },
                        ));
                        (delay, failure_streak)
                    };
                    if matches!(owner, TranslationWorkOwner::Preview(_))
                        && reason == TranslationRecoveryReason::RateLimited
                    {
                        // Spend recovery requests on durable finals, never on
                        // repeated copies of a replaceable long draft.
                        return Err(error);
                    }
                    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                    // The policy uses milliseconds; round up here and retain the
                    // native-clock check so sub-millisecond budget edges are exact.
                    let remaining_ms = remaining
                        .as_millis()
                        .saturating_add(1)
                        .try_into()
                        .unwrap_or(u64::MAX);
                    let decision = translation_policy::retry_with_remaining(
                        class,
                        attempt,
                        failure_streak,
                        remaining_ms,
                    );
                    if decision.exhausted {
                        return Err(error);
                    }
                    if !decision.retry || tokio::time::Instant::now() + delay >= deadline {
                        return Err(QwenMTClientError::RequestTimedOut);
                    }
                    pipeline_log!(
                        "mt retrying attempt={} delayMs={} error={}",
                        attempt,
                        delay.as_millis(),
                        error.diagnostic_label()
                    );
                    attempt += 1;
                }
            }
        }
    }

    // MARK: Lifecycle helpers

    async fn flush_pending_draft(&self) {
        let pending = {
            let mut inner = self.inner.lock().await;
            abort_task(&mut inner.draft_stability_task);
            abort_task(&mut inner.draft_maximum_wait_task);
            self.cancel_preview(&mut inner, PreviewCancellationReason::SessionFinish);
            self.advance_preview_epoch();
            let text = inner.committer.preview_latest_draft(false);
            let language = inner.latest_draft_language.clone();
            let source_utterance_id = inner.current_source_utterance_id;
            let revision = text
                .as_ref()
                .map(|_| next_nonzero(&mut inner.next_confirmation_id));
            inner.committer.reset();
            inner.latest_draft_language = None;
            inner.current_source_utterance_id = None;
            inner.preview_request_pacer.finish_utterance();
            next_nonzero(&mut inner.draft_revision);
            text.zip(revision)
                .map(|(text, revision)| (text, language, revision, source_utterance_id))
        };
        if let Some((text, language, revision, source_utterance_id)) = pending {
            pipeline_log!("audio3 asr fallback final length={}", text.chars().count());
            self.enqueue_final(
                text,
                language,
                FinalBoundary::SessionFinish,
                revision,
                source_utterance_id,
            )
            .await;
        }
    }

    async fn cancel_replaceable_work(&self) {
        let mut inner = self.inner.lock().await;
        abort_task(&mut inner.draft_stability_task);
        abort_task(&mut inner.draft_maximum_wait_task);
        self.cancel_preview(&mut inner, PreviewCancellationReason::Stop);
        self.advance_preview_epoch();
        next_nonzero(&mut inner.draft_revision);
    }

    async fn reset_draft_state(&self) {
        let mut inner = self.inner.lock().await;
        abort_task(&mut inner.draft_stability_task);
        abort_task(&mut inner.draft_maximum_wait_task);
        self.cancel_preview(&mut inner, PreviewCancellationReason::Reset);
        self.advance_preview_epoch();
        inner.committer.reset();
        inner.latest_draft_language = None;
        inner.preview_request_pacer.finish_utterance();
        next_nonzero(&mut inner.draft_revision);
        inner.last_server_final = None;
        inner.last_source_utterance_id = None;
        inner.current_source_utterance_id = None;
        inner.next_confirmation_id = 0;
        inner.last_draft_log_at = None;
    }

    async fn cancel_final_translations(&self) {
        let mut inner = self.inner.lock().await;
        abort_task(&mut inner.final_worker);
        *self.translation_latency.lock().unwrap() = None;
        inner.final_queue.clear();
        inner.active_final = None;
        inner.mt_cooldown = None;
        inner.mt_failure_streak = 0;
        inner.preview_request_pacer.reset();
        inner.pipeline_failed = false;
        inner.final_completion_in_progress = false;
        inner.deferred_overload = false;
    }

    async fn wait_for_final_translations(&self, timeout: Duration) {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let has_worker = self.inner.lock().await.final_worker.is_some();
            if !has_worker || tokio::time::Instant::now() >= deadline {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn install_asr_bridge(&self, mut receiver: ProviderEventReceiver) {
        let (finish_tx, finish_rx) = oneshot::channel();
        let self_arc = self.clone();
        let task = tokio::spawn(async move {
            let mut finish_tx = Some(finish_tx);
            while let Some(envelope) = receiver.recv_with_revision().await {
                let _content = self_arc.content_operation.lock().await;
                if envelope.is_content()
                    && envelope.content_revision != self_arc.asr_client.content_revision()
                {
                    continue;
                }
                let event = envelope.event;
                let is_session_finished = event == LiveTranslateServerEvent::SessionFinished;
                self_arc.handle_asr_event(event).await;
                if is_session_finished {
                    if let Some(finish_tx) = finish_tx.take() {
                        let _ = finish_tx.send(());
                    }
                }
            }
        });
        let mut bridge = self.asr_bridge.lock().await;
        if let Some(previous_task) = bridge.task.replace(task) {
            previous_task.abort();
        }
        bridge.finish_ack = Some(finish_rx);
    }

    async fn wait_for_asr_bridge_finish(&self, timeout: Duration) -> bool {
        let finish_ack = self.take_asr_bridge_finish_ack().await;
        let Some(finish_ack) = finish_ack else {
            return false;
        };
        matches!(tokio::time::timeout(timeout, finish_ack).await, Ok(Ok(())))
    }

    async fn take_asr_bridge_finish_ack(&self) -> Option<oneshot::Receiver<()>> {
        self.asr_bridge.lock().await.finish_ack.take()
    }

    async fn disconnect_asr_bridge(&self) {
        let mut bridge = self.asr_bridge.lock().await;
        bridge.finish_ack = None;
        if let Some(task) = bridge.task.take() {
            task.abort();
        }
    }

    fn emit_content(&self, revision: u64, event: LiveTranslateServerEvent) {
        let _ = self.events.send_content(revision, event);
    }

    fn emit(&self, event: LiveTranslateServerEvent) {
        let _ = self.events.send(event);
    }
}

fn next_nonzero(counter: &mut u64) -> u64 {
    *counter = counter.wrapping_add(1);
    if *counter == 0 {
        *counter = 1;
    }
    *counter
}

fn source_draft_event(
    source_utterance_id: Option<u64>,
    text: String,
    language: Option<String>,
) -> LiveTranslateServerEvent {
    match source_utterance_id {
        Some(utterance_id) => LiveTranslateServerEvent::SourceUtteranceDraft {
            utterance_id,
            text,
            language,
        },
        None => LiveTranslateServerEvent::SourceDraft { text, language },
    }
}

fn abort_task(slot: &mut Option<TaskSlot>) {
    if let Some(task) = slot.take() {
        task.handle.abort();
    }
}

fn clear_task_if_id(slot: &mut Option<TaskSlot>, task_id: u64) {
    if slot.as_ref().is_some_and(|task| task.id == task_id) {
        drop(slot.take());
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn facade_routes_recognition_and_translation_to_distinct_proxies() {
        use crate::clients::translation_client::TranslationClient;
        use crate::core::network_proxy::{ProxyConfig, ProxyMode};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let speech_proxy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let text_proxy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let destination = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let destination_address = destination.local_addr().unwrap();
        let speech_route = ProxyConfig {
            mode: ProxyMode::Custom,
            url: Some(format!("http://{}", speech_proxy.local_addr().unwrap())),
        };
        let text_route = ProxyConfig {
            mode: ProxyMode::Custom,
            url: Some(format!("http://{}", text_proxy.local_addr().unwrap())),
        };
        let configuration =
            custom_configuration(ProviderKind::CustomDashScopeASR, TargetLanguage::Japanese)
                .with_text_credentials(TextTranslationCredentials::OpenAICompatible {
                    endpoint: format!("http://{destination_address}/v1"),
                    model: "synthetic-model".into(),
                    api_key: String::new(),
                })
                .with_stage_network_proxies(speech_route, text_route);
        let text_server = tokio::spawn(async move {
            let (mut socket, _) = text_proxy.accept().await.unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut buffer = [0; 2048];
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    let length: usize = String::from_utf8_lossy(&bytes[..end])
                        .lines()
                        .find_map(|line| {
                            line.to_lowercase()
                                .strip_prefix("content-length:")
                                .map(|value| value.trim().parse().unwrap())
                        })
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            assert!(String::from_utf8_lossy(&bytes).starts_with(&format!(
                "POST http://{destination_address}/v1/chat/completions HTTP/1.1"
            )));
            let body = r#"{"choices":[{"message":{"content":"Synthetic translation"}}]}"#;
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        let speech_server = tokio::spawn(async move {
            let (mut socket, _) = speech_proxy.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut buffer = [0; 1024];
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                request.extend_from_slice(&buffer[..count]);
                if request.windows(4).any(|part| part == b"\r\n\r\n") {
                    break;
                }
            }
            assert!(
                String::from_utf8_lossy(&request).starts_with("CONNECT example.com:443 HTTP/1.1")
            );
            socket
                .write_all(
                    b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
        });
        let (sender, mut events) = provider_event_channel();
        let TranslationClient::HighQuality(client) =
            TranslationClient::new(&configuration, sender).unwrap()
        else {
            panic!("expected independent pipeline")
        };
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceFinal {
                utterance_id: 1,
                text: "Synthetic source".into(),
                language: None,
            })
            .await;
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if matches!(
                    events.recv().await,
                    Some(LiveTranslateServerEvent::SubtitleConfirmedPair { .. })
                ) {
                    break;
                }
            }
            text_server.await.unwrap();
        })
        .await
        .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(3), client.connect())
                .await
                .unwrap()
                .is_err()
        );
        tokio::time::timeout(Duration::from_secs(3), speech_server)
            .await
            .unwrap()
            .unwrap();
        client.disconnect().await;
        assert!(
            tokio::time::timeout(Duration::from_millis(20), destination.accept())
                .await
                .is_err()
        );
    }

    use super::*;
    use crate::clients::provider_events::ProviderEventReceiver;
    use crate::core::models::TranslationMode;
    use crate::core::provider::ProviderKind;

    fn custom_configuration(
        provider: ProviderKind,
        target: TargetLanguage,
    ) -> LiveTranslationConfiguration {
        LiveTranslationConfiguration::with_credentials(
            provider,
            ProviderCredentials::CustomSpeech {
                endpoint: "wss://example.com/recognition".into(),
                model: "synthetic-recognition-model".into(),
                api_key: "synthetic-recognition-key".into(),
            },
            SourceLanguage::Automatic,
            target,
            TranslationMode::Turbo,
        )
    }

    #[tokio::test]
    async fn custom_recognition_original_mode_commits_without_constructing_or_accounting_for_mt() {
        for provider in [
            ProviderKind::CustomDashScopeASR,
            ProviderKind::CustomOpenAIASR,
        ] {
            let configuration = custom_configuration(provider, TargetLanguage::Original);
            let (sender, mut events) = provider_event_channel();
            let client = HighQualityTranslationClient::new_custom(&configuration, sender).unwrap();
            assert!(matches!(
                client.mt.as_ref(),
                TextTranslationClient::Disabled
            ));
            client
                .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceFinal {
                    utterance_id: 1,
                    text: "Synthetic original sentence".into(),
                    language: Some("en".into()),
                })
                .await;
            assert!(
                matches!(events.recv().await, Some(LiveTranslateServerEvent::SubtitleConfirmedPair {
                source_utterance_id: Some(1), source, translation, ..
            }) if source == "Synthetic original sentence" && translation == source)
            );
            assert!(client.inner.lock().await.final_worker.is_none());
            assert!(client.inner.lock().await.preview_task.is_none());
            assert_eq!(client.translation_latency(), None);
            let now = std::time::Instant::now();
            assert_eq!(
                client
                    .inner
                    .lock()
                    .await
                    .preview_request_pacer
                    .next_shared_start_at(now),
                now
            );
            assert_eq!(
                client
                    .suspend_request_budget()
                    .await
                    .shared_cooldown_until(),
                None
            );
            client.disconnect().await;
        }
    }

    #[test]
    fn custom_recognition_translation_cannot_reuse_the_recognition_key_as_a_qwen_key() {
        let configuration =
            custom_configuration(ProviderKind::CustomDashScopeASR, TargetLanguage::Japanese);
        let (sender, _events) = provider_event_channel();
        assert!(matches!(
            HighQualityTranslationClient::new_custom(&configuration, sender),
            Err(QwenMTClientError::MissingTextTranslation)
        ));
    }

    #[tokio::test]
    async fn custom_recognition_uses_its_independent_chat_key_and_preserves_final_order() {
        assert_independent_chat_final_order(false).await;
        assert_independent_chat_final_order(true).await;
    }

    async fn assert_independent_chat_final_order(chatmock: bool) {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut configuration =
            custom_configuration(ProviderKind::CustomDashScopeASR, TargetLanguage::Japanese);
        configuration.text_credentials = Some(if chatmock {
            TextTranslationCredentials::ChatMock {
                endpoint: format!("http://{}/v1", listener.local_addr().unwrap()),
                model: "synthetic-translation-model".into(),
                api_key: "synthetic-translation-key".into(),
            }
        } else {
            TextTranslationCredentials::OpenAICompatible {
                endpoint: format!("http://{}/v1", listener.local_addr().unwrap()),
                model: "synthetic-translation-model".into(),
                api_key: "synthetic-translation-key".into(),
            }
        });
        let server = tokio::spawn(async move {
            for index in 1..=2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = String::from_utf8(read_synthetic_request(&mut socket).await).unwrap();
                assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1"));
                assert!(request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer synthetic-translation-key"));
                assert!(!request.contains("synthetic-recognition-key"));
                let body_start = request.find("\r\n\r\n").unwrap() + 4;
                let json: serde_json::Value = serde_json::from_str(&request[body_start..]).unwrap();
                assert_eq!(json["model"], "synthetic-translation-model");
                assert_eq!(json["stream"], false);
                assert_eq!(
                    json["messages"][1]["content"],
                    format!("Synthetic sentence {index}")
                );
                let body = format!(
                    r#"{{"choices":[{{"message":{{"content":"Synthetic translation {index}"}}}}]}}"#
                );
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
        });
        let (sender, mut events) = provider_event_channel();
        let mut client = HighQualityTranslationClient::new_custom(&configuration, sender).unwrap();
        client
            .set_network(
                ProviderNetwork::resolve(&crate::core::network_proxy::ProxyConfig {
                    mode: crate::core::network_proxy::ProxyMode::Direct,
                    url: None,
                })
                .unwrap(),
            )
            .unwrap();
        for utterance_id in 1..=2 {
            client
                .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceFinal {
                    utterance_id,
                    text: format!("Synthetic sentence {utterance_id}"),
                    language: Some("en".into()),
                })
                .await;
        }
        let pairs = tokio::time::timeout(Duration::from_secs(5), async {
            let mut pairs = Vec::new();
            while pairs.len() < 2 {
                if let Some(LiveTranslateServerEvent::SubtitleConfirmedPair {
                    source_utterance_id,
                    source,
                    translation,
                    ..
                }) = events.recv().await
                {
                    pairs.push((source_utterance_id, source, translation));
                }
            }
            pairs
        })
        .await
        .unwrap();
        assert_eq!(
            pairs,
            vec![
                (
                    Some(1),
                    "Synthetic sentence 1".into(),
                    "Synthetic translation 1".into()
                ),
                (
                    Some(2),
                    "Synthetic sentence 2".into(),
                    "Synthetic translation 2".into()
                ),
            ]
        );
        server.await.unwrap();
        client.disconnect().await;
    }

    #[tokio::test]
    async fn clear_after_a_real_sse_partial_does_not_restore_late_stream_content() {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint =
            url::Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let (release_tx, release_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let _request = read_synthetic_request(&mut socket).await;
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").await.unwrap();
            let partial =
                "data: {\"choices\":[{\"delta\":{\"content\":\"Synthetic old partial\"}}]}\n\n";
            socket
                .write_all(format!("{:x}\r\n{partial}\r\n", partial.len()).as_bytes())
                .await
                .unwrap();
            release_rx.await.unwrap();
            let late = "data: {\"choices\":[{\"delta\":{\"content\":\" late old suffix\"}}]}\n\ndata: [DONE]\n\n";
            let _ = socket
                .write_all(format!("{:x}\r\n{late}\r\n0\r\n\r\n", late.len()).as_bytes())
                .await;
            assert!(
                tokio::time::timeout(Duration::from_millis(100), listener.accept())
                    .await
                    .is_err()
            );
        });
        let (sender, mut receiver) = provider_event_channel();
        let mut client = HighQualityTranslationClient::new(
            "synthetic-key",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            QwenMTModel::Lite,
            Duration::from_secs(60),
            Duration::from_secs(60),
            12,
            sender,
        )
        .unwrap();
        let TextTranslationClient::Qwen(mt, _) = Arc::make_mut(&mut client.mt) else {
            unreachable!()
        };
        mt.use_synthetic_endpoint(endpoint);
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceFinal {
                utterance_id: 1,
                text: "Synthetic old source".into(),
                language: Some("en".into()),
            })
            .await;
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if matches!(receiver.recv().await, Some(LiveTranslateServerEvent::TranslationDraft(text)) if text == "Synthetic old partial") { break; }
            }
        }).await.unwrap();
        client.clear_content().await;
        assert!(receiver.try_recv().is_err());
        release_tx.send(()).unwrap();
        server.await.unwrap();
        assert!(receiver.try_recv().is_err());
        assert!(client.inner.lock().await.final_worker.is_none());
        client.disconnect().await;
    }

    #[tokio::test]
    async fn clear_keeps_quota_budget_and_rejects_old_preview_and_final_callbacks() {
        let (client, mut receiver) = test_client(TargetLanguage::Japanese, 20);
        let old_revision = client.content_revision();
        let preview_id = client.advance_preview_epoch();
        let late_partial = client.preview_partial_handler(preview_id);
        let cooldown_until = tokio::time::Instant::now() + Duration::from_secs(8);
        let now = std::time::Instant::now();
        let (shared_at, preview_at) = {
            let mut inner = client.inner.lock().await;
            inner.committer.update_draft("Synthetic cleared draft");
            inner.last_source_utterance_id = Some(7);
            inner.mt_failure_streak = 2;
            inner.mt_cooldown = Some(MTCooldown {
                until: cooldown_until,
                reason: TranslationRecoveryReason::RateLimited,
            });
            inner.preview_request_pacer.record_start(now);
            inner.preview_request_pacer.suppress_after_rate_limit(now);
            inner
                .preview_request_pacer
                .set_shared_cooldown(Some(now + Duration::from_secs(8)));
            inner.preview_task = Some(TaskSlot {
                id: preview_id,
                handle: tokio::spawn(std::future::pending()),
            });
            inner.preview_http_pending = Some(preview_id);
            inner.pending_preview_revision = Some(inner.draft_revision);
            (
                inner.preview_request_pacer.next_shared_start_at(now),
                inner.preview_request_pacer.next_start_at(now),
            )
        };
        let revision = client.clear_content().await;
        assert_ne!(revision, old_revision);
        late_partial("Synthetic old partial".into());
        client.emit_content(
            old_revision,
            LiveTranslateServerEvent::SubtitleConfirmedPair {
                source_utterance_id: None,
                utterance_id: 1,
                source: "old".into(),
                language: None,
                translation: "old".into(),
            },
        );
        assert!(receiver.try_recv().is_err());
        let inner = client.inner.lock().await;
        assert_eq!(inner.mt_failure_streak, 2);
        assert_eq!(inner.mt_cooldown.unwrap().until, cooldown_until);
        assert_eq!(
            inner.preview_request_pacer.next_shared_start_at(now),
            shared_at
        );
        assert_eq!(inner.preview_request_pacer.next_start_at(now), preview_at);
        assert_eq!(inner.last_source_utterance_id, Some(7));
        assert!(!inner.committer.has_pending_text());
        assert!(inner.final_queue.is_empty());
        assert!(inner.preview_task.is_none());
        assert!(inner.pending_preview_revision.is_none());
        assert!(client.mt_work_allowed.load(Ordering::SeqCst));
        drop(inner);
        assert!(client
            .prepare_server_final("Synthetic replay", Some(7))
            .await
            .is_none());
        assert!(client
            .prepare_server_final("Synthetic new sentence", Some(8))
            .await
            .is_some());
        client.disconnect().await;
    }

    #[tokio::test]
    async fn clear_cancels_inflight_http_and_queued_finals_then_new_final_recovers() {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (started_tx, started_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut old, _) = listener.accept().await.unwrap();
            let request = String::from_utf8(read_synthetic_request(&mut old).await).unwrap();
            assert!(request.contains("Synthetic old active"));
            started_tx.send(()).unwrap();
            release_rx.await.unwrap();
            let body = r#"{"code":200,"data":"Late old translation"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = old.write_all(response.as_bytes()).await;
            let (mut next, _) = tokio::time::timeout(Duration::from_secs(4), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let request = String::from_utf8(read_synthetic_request(&mut next).await).unwrap();
            assert!(request.contains("Synthetic new sentence"));
            assert!(!request.contains("Synthetic old queued"));
            write_synthetic_response(&mut next, 200, r#"{"code":200,"data":"New translation"}"#)
                .await;
            assert!(
                tokio::time::timeout(Duration::from_millis(100), listener.accept())
                    .await
                    .is_err()
            );
        });
        let (sender, mut receiver) = provider_event_channel();
        let mut client = HighQualityTranslationClient::new_deeplx(
            "synthetic-key",
            &endpoint,
            "",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            sender,
        )
        .unwrap();
        client
            .set_network(
                ProviderNetwork::resolve(&crate::core::network_proxy::ProxyConfig {
                    mode: crate::core::network_proxy::ProxyMode::Direct,
                    url: None,
                })
                .unwrap(),
            )
            .unwrap();
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceFinal {
                utterance_id: 1,
                text: "Synthetic old active".into(),
                language: Some("en".into()),
            })
            .await;
        tokio::time::timeout(Duration::from_secs(2), started_rx)
            .await
            .unwrap()
            .unwrap();
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceFinal {
                utterance_id: 2,
                text: "Synthetic old queued".into(),
                language: Some("en".into()),
            })
            .await;
        assert_eq!(client.inner.lock().await.final_queue.len(), 1);
        let revision = client.clear_content().await;
        assert!(receiver.try_recv().is_err());
        release_tx.send(()).unwrap();
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceFinal {
                utterance_id: 2,
                text: "Synthetic old queued".into(),
                language: Some("en".into()),
            })
            .await;
        assert!(client.inner.lock().await.final_queue.is_empty());
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceFinal {
                utterance_id: 3,
                text: "Synthetic new sentence".into(),
                language: Some("en".into()),
            })
            .await;
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                let event = receiver.recv_with_revision().await.unwrap();
                assert_eq!(event.content_revision, revision);
                if let LiveTranslateServerEvent::SubtitleConfirmedPair {
                    source,
                    translation,
                    ..
                } = event.event
                {
                    assert_eq!(source, "Synthetic new sentence");
                    assert_eq!(translation, "New translation");
                    break;
                }
            }
        })
        .await
        .unwrap();
        client
            .wait_for_final_translations(Duration::from_secs(1))
            .await;
        assert!(receiver.try_recv().is_err());
        client.disconnect().await;
        server.await.unwrap();
    }

    #[tokio::test]
    async fn lite_rejects_known_unsupported_source_before_http_or_request_accounting() {
        let (sender, _events) = provider_event_channel();
        let client = HighQualityTranslationClient::new(
            "synthetic-key",
            SourceLanguage::Automatic,
            TargetLanguage::Japanese,
            crate::core::protocols::qwen_mt::REALTIME_MT_MODEL,
            Duration::from_secs(60),
            Duration::from_secs(60),
            12,
            sender,
        )
        .unwrap();
        assert!(!client.mt.supports_reported_source(Some("no")));
        assert!(client.mt.supports_reported_source(None));
        assert!(client.mt.supports_reported_source(Some("unknown-report")));
        let error = client
            .translate_with_retry(
                "Synthetic unsupported source",
                Some("no"),
                tokio::time::Instant::now() + Duration::from_secs(1),
                Arc::new(|_| {}),
                TranslationWorkOwner::Preview(999),
                TranslationEvidenceIdentity::default(),
            )
            .await
            .err()
            .unwrap();
        assert_eq!(error, QwenMTClientError::UnsupportedSource);
        let now = std::time::Instant::now();
        assert_eq!(
            client
                .inner
                .lock()
                .await
                .preview_request_pacer
                .next_shared_start_at(now),
            now
        );
    }

    #[tokio::test]
    async fn pause_reconnect_budget_keeps_preview_suppressed_and_final_retry_cancellable() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (old_sender, _old_events) = provider_event_channel();
        let old = HighQualityTranslationClient::new_deeplx(
            "synthetic-asr",
            &endpoint,
            "",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            old_sender,
        )
        .unwrap();
        {
            let mut inner = old.inner.lock().await;
            let now = std::time::Instant::now();
            inner.preview_request_pacer.record_start(now);
            inner.preview_request_pacer.suppress_after_rate_limit(now);
            inner
                .preview_request_pacer
                .set_shared_cooldown(Some(now + Duration::from_secs(4)));
        }
        let budget = old.suspend_request_budget().await;
        assert!(!old.mt_work_allowed.load(Ordering::SeqCst));
        old.disconnect().await;
        let (sender, mut events) = provider_event_channel();
        let mut resumed = HighQualityTranslationClient::new_deeplx(
            "synthetic-asr",
            &endpoint,
            "",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            sender,
        )
        .unwrap();
        resumed.stable_draft_delay = Duration::from_secs(60);
        resumed.maximum_wait_delay = Duration::from_secs(60);
        resumed.restore_request_budget(budget).await;
        resumed
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: "Synthetic new speech after resume".into(),
                language: Some("en".into()),
            })
            .await;
        let revision = resumed.inner.lock().await.draft_revision;
        resumed
            .start_preview(
                "Synthetic new speech after resume".into(),
                Some("en".into()),
                revision,
            )
            .await;
        assert!(resumed.inner.lock().await.preview_task.is_none());
        resumed
            .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
                text: "Synthetic durable final after resume".into(),
                language: Some("en".into()),
            })
            .await;
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let LiveTranslateServerEvent::TranslationDeferred(recovery) =
                    events.recv().await.unwrap()
                {
                    assert_eq!(recovery.reason, TranslationRecoveryReason::RateLimited);
                    assert!(recovery.retry_scheduled);
                    assert!(recovery.retry_after_ms > 3_000);
                    break;
                }
            }
        })
        .await
        .unwrap();
        resumed.disconnect().await;
        assert!(
            tokio::time::timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err()
        );
        assert!(resumed.inner.lock().await.final_worker.is_none());
        while let Ok(event) = events.try_recv() {
            assert!(!matches!(
                event,
                LiveTranslateServerEvent::TranslationStarted
                    | LiveTranslateServerEvent::SubtitleConfirmedPair { .. }
            ));
        }
    }

    fn test_client(
        target_language: TargetLanguage,
        threshold: usize,
    ) -> (HighQualityTranslationClient, ProviderEventReceiver) {
        let (events, receiver) = provider_event_channel();
        let client = HighQualityTranslationClient::new(
            "test-key",
            SourceLanguage::Japanese,
            target_language,
            QwenMTModel::Plus,
            Duration::from_secs(60),
            Duration::from_secs(60),
            threshold,
            events,
        )
        .unwrap();
        (client, receiver)
    }

    async fn read_synthetic_request(socket: &mut tokio::net::TcpStream) -> Vec<u8> {
        use tokio::io::AsyncReadExt;
        let mut request = Vec::new();
        loop {
            let mut chunk = [0; 1024];
            let count = socket.read(&mut chunk).await.unwrap();
            assert!(count > 0, "synthetic HTTP request ended before its body");
            request.extend_from_slice(&chunk[..count]);
            assert!(request.len() <= 16 * 1024);
            if let Some(header_end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                let headers = std::str::from_utf8(&request[..header_end]).unwrap();
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if request.len() >= header_end + 4 + content_length {
                    return request;
                }
            }
        }
    }

    async fn reply_to_synthetic_request(
        socket: &mut tokio::net::TcpStream,
        status: u16,
        body: &str,
    ) {
        let _ = read_synthetic_request(socket).await;
        write_synthetic_response(socket, status, body).await;
    }

    async fn write_synthetic_response(socket: &mut tokio::net::TcpStream, status: u16, body: &str) {
        use tokio::io::AsyncWriteExt;
        let response = format!(
            "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    }

    #[tokio::test]
    async fn repeated_document_preview_and_finals_do_not_send_session_history_as_tm_list() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/translate", listener.local_addr().unwrap());
        let document = "Synthetic document with an earlier sentence and a later tail.";
        let prefix = "Synthetic document with an earlier sentence";
        let server = tokio::spawn(async move {
            for (input, output) in [
                (document, "Synthetic first complete document translation"),
                (prefix, "Synthetic shorter prefix translation"),
                (document, "Synthetic repeated complete document translation"),
            ] {
                let (mut socket, _) =
                    tokio::time::timeout(Duration::from_secs(5), listener.accept())
                        .await
                        .unwrap()
                        .unwrap();
                let request = read_synthetic_request(&mut socket).await;
                let body_start = request
                    .windows(4)
                    .position(|bytes| bytes == b"\r\n\r\n")
                    .unwrap()
                    + 4;
                let json: serde_json::Value =
                    serde_json::from_slice(&request[body_start..]).unwrap();
                assert_eq!(json["messages"].as_array().unwrap().len(), 1);
                assert_eq!(json["messages"][0]["content"], input);
                assert_eq!(json["model"], "qwen-mt-lite");
                assert_eq!(json["stream"], true);
                assert!(json["translation_options"].get("tm_list").is_none(),
                    "confirmed history must not be presented as a style example for a new prefix or final");
                assert!(json["translation_options"].get("domains").is_none());
                let event = serde_json::json!({"choices":[{"delta":{"content":output}}]});
                let response = format!("data: {event}\n\ndata: [DONE]\n\n");
                write_synthetic_response(&mut socket, 200, &response).await;
            }
            assert!(
                tokio::time::timeout(Duration::from_millis(100), listener.accept())
                    .await
                    .is_err()
            );
        });
        let (sender, mut events) = provider_event_channel();
        let mut client = HighQualityTranslationClient::new(
            "synthetic-key",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            QwenMTModel::Lite,
            Duration::from_secs(60),
            Duration::from_secs(60),
            12,
            sender,
        )
        .unwrap();
        let TextTranslationClient::Qwen(mt, _) = Arc::make_mut(&mut client.mt) else {
            panic!("fixture uses Qwen")
        };
        mt.use_synthetic_endpoint(endpoint.parse().unwrap());
        let mut controller = crate::core::session::TranslationSessionController::default();
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
                text: document.into(),
                language: Some("en".into()),
            })
            .await;
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let event = events.recv().await.unwrap();
                let confirmed = matches!(
                    event,
                    LiveTranslateServerEvent::SubtitleConfirmedPair { .. }
                );
                controller.handle(event);
                if confirmed {
                    break;
                }
            }
        })
        .await
        .unwrap();
        client
            .wait_for_final_translations(Duration::from_secs(1))
            .await;
        assert_eq!(
            controller.state.subtitles.translation.text,
            "Synthetic first complete document translation"
        );
        controller.clear_subtitles();
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: prefix.into(),
                language: Some("en".into()),
            })
            .await;
        let revision = client.inner.lock().await.draft_revision;
        client
            .start_preview(prefix.into(), Some("en".into()), revision)
            .await;
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let event = events.recv().await.unwrap();
                let completed =
                    matches!(event, LiveTranslateServerEvent::SubtitlePreviewPair { .. });
                controller.handle(event);
                if completed {
                    break;
                }
            }
        })
        .await
        .unwrap();
        assert!(controller.state.subtitles.history.is_empty());
        assert_eq!(
            controller.state.subtitles.preview_pair,
            Some(crate::core::models::PreviewSubtitlePair {
                utterance_id: None,
                source: prefix.into(),
                translation: "Synthetic shorter prefix translation".into(),
            })
        );
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
                text: document.into(),
                language: Some("en".into()),
            })
            .await;
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let event = events.recv().await.unwrap();
                let confirmed = matches!(
                    event,
                    LiveTranslateServerEvent::SubtitleConfirmedPair { .. }
                );
                controller.handle(event);
                if confirmed {
                    break;
                }
            }
        })
        .await
        .unwrap();
        assert!(controller.state.subtitles.preview_pair.is_none());
        assert_eq!(controller.state.subtitles.source.text, document);
        assert_eq!(
            controller.state.subtitles.translation.text,
            "Synthetic repeated complete document translation"
        );
        client
            .wait_for_final_translations(Duration::from_secs(1))
            .await;
        server.await.unwrap();
        client.disconnect().await;
    }

    #[tokio::test]
    async fn fast_drafts_complete_inflight_preview_then_only_the_latest_successor() {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (first_started, first_request) = oneshot::channel();
        let (release_first, release_first_rx) = oneshot::channel();
        let (second_started, second_request) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_synthetic_request(&mut socket).await;
            assert!(String::from_utf8(request)
                .unwrap()
                .contains("First preview"));
            first_started.send(()).unwrap();
            release_first_rx.await.unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(100), listener.accept())
                    .await
                    .is_err(),
                "latest queued previews must not run beside the in-flight HTTP request"
            );
            let body = r#"{"code":200,"data":"First translation"}"#;
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            drop(socket);
            let (mut socket, _) = listener.accept().await.unwrap();
            second_started.send(std::time::Instant::now()).unwrap();
            let request = read_synthetic_request(&mut socket).await;
            let request = String::from_utf8(request).unwrap();
            assert!(request.contains("Newest draft before its timer"));
            assert!(!request.contains("Intermediate replacement"));
            assert!(!request.contains("Latest replacement"));
            tokio::time::sleep(Duration::from_millis(20)).await;
            let body = r#"{"code":200,"data":"Latest translation"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(100), listener.accept())
                    .await
                    .is_err()
            );
        });
        let (sender, mut events) = provider_event_channel();
        let mut client = HighQualityTranslationClient::new_deeplx(
            "synthetic-asr",
            &endpoint,
            "",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            sender,
        )
        .unwrap();
        client.stable_draft_delay = Duration::from_secs(60);
        client.maximum_wait_delay = Duration::from_secs(60);
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: "First preview".into(),
                language: Some("en".into()),
            })
            .await;
        let revision = client.inner.lock().await.draft_revision;
        client
            .start_preview("First preview".into(), Some("en".into()), revision)
            .await;
        tokio::time::timeout(Duration::from_secs(1), first_request)
            .await
            .unwrap()
            .unwrap();
        let allowed_at = client
            .inner
            .lock()
            .await
            .preview_request_pacer
            .next_start_at(std::time::Instant::now());
        let preview_id = client.inner.lock().await.preview_task.as_ref().unwrap().id;
        for text in ["Intermediate replacement", "Latest replacement"] {
            client
                .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                    text: text.into(),
                    language: Some("en".into()),
                })
                .await;
            let revision = client.inner.lock().await.draft_revision;
            client
                .start_preview(text.into(), Some("en".into()), revision)
                .await;
            let inner = client.inner.lock().await;
            assert_eq!(inner.preview_task.as_ref().unwrap().id, preview_id);
            assert_eq!(inner.preview_http_pending, Some(preview_id));
            assert_eq!(inner.pending_preview_revision, Some(revision));
            drop(inner);
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        // This draft has not reached either timer: completion must read the
        // committer's newest bounded text, not the older queued candidate.
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: "Newest draft before its timer".into(),
                language: Some("en".into()),
            })
            .await;
        assert!(client.preview_is_current(preview_id));
        release_first.send(()).unwrap();
        let mut pairs = Vec::new();
        tokio::time::timeout(Duration::from_secs(6), async {
            while pairs.len() < 2 {
                if let LiveTranslateServerEvent::SubtitlePreviewPair {
                    source,
                    translation,
                    ..
                } = events.recv().await.unwrap()
                {
                    pairs.push((source, translation));
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(
            pairs,
            [
                ("First preview".into(), "First translation".into()),
                (
                    "Newest draft before its timer".into(),
                    "Latest translation".into()
                ),
            ]
        );
        assert!(
            second_request.await.unwrap() >= allowed_at,
            "the completed HTTP request must retain its exact pacing boundary"
        );
        assert!(
            client.translation_latency().unwrap().milliseconds < 300,
            "preview spacing is excluded from the actual request measurement"
        );
        client.disconnect().await;
        server.await.unwrap();
    }

    #[tokio::test]
    async fn waiting_preview_replacements_send_only_the_latest_candidate() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = String::from_utf8(read_synthetic_request(&mut socket).await).unwrap();
            assert!(request.contains("Latest waiting candidate"));
            assert!(!request.contains("First waiting candidate"));
            assert!(!request.contains("Intermediate waiting candidate"));
            write_synthetic_response(
                &mut socket,
                200,
                r#"{"code":200,"data":"Latest waiting result"}"#,
            )
            .await;
            assert!(
                tokio::time::timeout(Duration::from_millis(100), listener.accept())
                    .await
                    .is_err()
            );
        });
        let (sender, mut events) = provider_event_channel();
        let mut client = HighQualityTranslationClient::new_deeplx(
            "synthetic-asr",
            &endpoint,
            "",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            sender,
        )
        .unwrap();
        client.stable_draft_delay = Duration::from_secs(60);
        client.maximum_wait_delay = Duration::from_secs(60);
        client
            .inner
            .lock()
            .await
            .preview_request_pacer
            .record_start(std::time::Instant::now());
        let mut previous_id = None;
        for text in [
            "First waiting candidate",
            "Intermediate waiting candidate",
            "Latest waiting candidate",
        ] {
            client
                .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                    text: text.into(),
                    language: Some("en".into()),
                })
                .await;
            let revision = client.inner.lock().await.draft_revision;
            client
                .start_preview(text.into(), Some("en".into()), revision)
                .await;
            let inner = client.inner.lock().await;
            let id = inner.preview_task.as_ref().unwrap().id;
            assert_ne!(Some(id), previous_id);
            assert!(inner.preview_http_pending.is_none());
            assert!(inner.pending_preview_revision.is_none());
            previous_id = Some(id);
            drop(inner);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let LiveTranslateServerEvent::SubtitlePreviewPair {
                    source,
                    translation,
                    ..
                } = events.recv().await.unwrap()
                {
                    assert_eq!(source, "Latest waiting candidate");
                    assert_eq!(translation, "Latest waiting result");
                    break;
                }
            }
        })
        .await
        .unwrap();
        server.await.unwrap();
        client.disconnect().await;
    }

    #[tokio::test]
    async fn final_and_disconnect_cancel_inflight_http_and_its_latest_queued_preview() {
        use tokio::io::AsyncReadExt;
        for final_arrives in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}", listener.local_addr().unwrap());
            let (started, started_rx) = oneshot::channel();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = String::from_utf8(read_synthetic_request(&mut socket).await).unwrap();
                assert!(request.contains("In-flight synthetic source"));
                started.send(()).unwrap();
                let mut tail = Vec::new();
                tokio::time::timeout(Duration::from_secs(2), socket.read_to_end(&mut tail))
                    .await
                    .unwrap()
                    .unwrap();
                assert!(
                    tail.is_empty(),
                    "priority cancellation must close the old HTTP response"
                );
                if final_arrives {
                    let (mut socket, _) =
                        tokio::time::timeout(Duration::from_secs(5), listener.accept())
                            .await
                            .unwrap()
                            .unwrap();
                    let request =
                        String::from_utf8(read_synthetic_request(&mut socket).await).unwrap();
                    assert!(request.contains("Authoritative synthetic final"));
                    assert!(!request.contains("Queued synthetic candidate"));
                    write_synthetic_response(
                        &mut socket,
                        200,
                        r#"{"code":200,"data":"Confirmed synthetic result"}"#,
                    )
                    .await;
                }
                assert!(
                    tokio::time::timeout(Duration::from_millis(200), listener.accept())
                        .await
                        .is_err()
                );
            });
            let (sender, mut events) = provider_event_channel();
            let mut client = HighQualityTranslationClient::new_deeplx(
                "synthetic-asr",
                &endpoint,
                "",
                SourceLanguage::English,
                TargetLanguage::Japanese,
                sender,
            )
            .unwrap();
            client.stable_draft_delay = Duration::from_secs(60);
            client.maximum_wait_delay = Duration::from_secs(60);
            client
                .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                    text: "In-flight synthetic source".into(),
                    language: Some("en".into()),
                })
                .await;
            let revision = client.inner.lock().await.draft_revision;
            client
                .start_preview(
                    "In-flight synthetic source".into(),
                    Some("en".into()),
                    revision,
                )
                .await;
            tokio::time::timeout(Duration::from_secs(2), started_rx)
                .await
                .unwrap()
                .unwrap();
            let preview_id = client.inner.lock().await.preview_task.as_ref().unwrap().id;
            let late_partial = client.preview_partial_handler(preview_id);
            client
                .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                    text: "Queued synthetic candidate".into(),
                    language: Some("en".into()),
                })
                .await;
            let revision = client.inner.lock().await.draft_revision;
            client
                .start_preview(
                    "Queued synthetic candidate".into(),
                    Some("en".into()),
                    revision,
                )
                .await;
            assert_eq!(
                client.inner.lock().await.pending_preview_revision,
                Some(revision)
            );
            while events.try_recv().is_ok() {}
            if final_arrives {
                client
                    .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
                        text: "Authoritative synthetic final".into(),
                        language: Some("en".into()),
                    })
                    .await;
            } else {
                client.disconnect().await;
            }
            assert!(client.inner.lock().await.pending_preview_revision.is_none());
            assert!(!client.preview_is_current(preview_id));
            late_partial("Late synthetic old response".into());
            server.await.unwrap();
            if final_arrives {
                client
                    .wait_for_final_translations(Duration::from_secs(2))
                    .await;
            }
            let mut confirmed = 0;
            while let Ok(event) = events.try_recv() {
                match event {
                    LiveTranslateServerEvent::SubtitlePreviewPair { .. } => {
                        panic!("cancelled preview must not publish a late pair")
                    }
                    LiveTranslateServerEvent::TranslationDraft(text) => assert!(text.is_empty()),
                    LiveTranslateServerEvent::SubtitleConfirmedPair {
                        source,
                        translation,
                        ..
                    } => {
                        assert!(final_arrives);
                        assert_eq!(source, "Authoritative synthetic final");
                        assert_eq!(translation, "Confirmed synthetic result");
                        confirmed += 1;
                    }
                    _ => {}
                }
            }
            assert_eq!(confirmed, usize::from(final_arrives));
            assert!(client.inner.lock().await.preview_task.is_none());
            client.disconnect().await;
        }
    }

    #[tokio::test]
    async fn openai_compatible_final_and_disconnect_cancel_inflight_and_queued_previews() {
        use tokio::io::AsyncReadExt;
        for final_arrives in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}", listener.local_addr().unwrap());
            let (started, started_rx) = oneshot::channel();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = String::from_utf8(read_synthetic_request(&mut socket).await).unwrap();
                assert!(request.contains("In-flight synthetic source"));
                started.send(()).unwrap();
                let mut tail = Vec::new();
                tokio::time::timeout(Duration::from_secs(2), socket.read_to_end(&mut tail))
                    .await
                    .unwrap()
                    .unwrap();
                assert!(
                    tail.is_empty(),
                    "priority cancellation must close the old HTTP response"
                );
                if final_arrives {
                    let (mut socket, _) =
                        tokio::time::timeout(Duration::from_secs(5), listener.accept())
                            .await
                            .unwrap()
                            .unwrap();
                    let request =
                        String::from_utf8(read_synthetic_request(&mut socket).await).unwrap();
                    assert!(request.contains("Authoritative synthetic final"));
                    assert!(!request.contains("Queued synthetic candidate"));
                    write_synthetic_response(
                        &mut socket,
                        200,
                        r#"{"choices":[{"finish_reason":"stop","message":{"content":"Confirmed synthetic result"}}]}"#,
                    )
                    .await;
                }
                assert!(
                    tokio::time::timeout(Duration::from_millis(200), listener.accept())
                        .await
                        .is_err()
                );
            });
            let (sender, mut events) = provider_event_channel();
            let mut client = HighQualityTranslationClient::new_openai_compatible(
                "synthetic-asr",
                &endpoint,
                "synthetic-translation-key",
                "synthetic-model",
                SourceLanguage::English,
                TargetLanguage::Japanese,
                sender,
            )
            .unwrap();
            client
                .set_network(
                    ProviderNetwork::resolve(&crate::core::network_proxy::ProxyConfig {
                        mode: crate::core::network_proxy::ProxyMode::Direct,
                        url: None,
                    })
                    .unwrap(),
                )
                .unwrap();
            client.stable_draft_delay = Duration::from_secs(60);
            client.maximum_wait_delay = Duration::from_secs(60);
            client
                .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                    text: "In-flight synthetic source".into(),
                    language: Some("en".into()),
                })
                .await;
            let revision = client.inner.lock().await.draft_revision;
            client
                .start_preview(
                    "In-flight synthetic source".into(),
                    Some("en".into()),
                    revision,
                )
                .await;
            tokio::time::timeout(Duration::from_secs(2), started_rx)
                .await
                .unwrap()
                .unwrap();
            let preview_id = client.inner.lock().await.preview_task.as_ref().unwrap().id;
            let late_partial = client.preview_partial_handler(preview_id);
            client
                .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                    text: "Queued synthetic candidate".into(),
                    language: Some("en".into()),
                })
                .await;
            let revision = client.inner.lock().await.draft_revision;
            client
                .start_preview(
                    "Queued synthetic candidate".into(),
                    Some("en".into()),
                    revision,
                )
                .await;
            assert_eq!(
                client.inner.lock().await.pending_preview_revision,
                Some(revision)
            );
            while events.try_recv().is_ok() {}
            if final_arrives {
                client
                    .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
                        text: "Authoritative synthetic final".into(),
                        language: Some("en".into()),
                    })
                    .await;
            } else {
                client.disconnect().await;
            }
            assert!(client.inner.lock().await.pending_preview_revision.is_none());
            assert!(!client.preview_is_current(preview_id));
            late_partial("Late synthetic old response".into());
            server.await.unwrap();
            if final_arrives {
                client
                    .wait_for_final_translations(Duration::from_secs(2))
                    .await;
            }
            let mut confirmed = 0;
            while let Ok(event) = events.try_recv() {
                match event {
                    LiveTranslateServerEvent::SubtitlePreviewPair { .. } => {
                        panic!("cancelled preview must not publish a late pair")
                    }
                    LiveTranslateServerEvent::TranslationDraft(text) => assert!(text.is_empty()),
                    LiveTranslateServerEvent::SubtitleConfirmedPair {
                        source,
                        translation,
                        ..
                    } => {
                        assert!(final_arrives);
                        assert_eq!(source, "Authoritative synthetic final");
                        assert_eq!(translation, "Confirmed synthetic result");
                        confirmed += 1;
                    }
                    _ => {}
                }
            }
            assert_eq!(confirmed, usize::from(final_arrives));
            assert!(client.inner.lock().await.preview_task.is_none());
            client.disconnect().await;
        }
    }

    #[tokio::test]
    async fn rejected_inflight_preview_discards_queued_work_without_automatic_retry() {
        for status in [401, 429] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}", listener.local_addr().unwrap());
            let (started, started_rx) = oneshot::channel();
            let (release, release_rx) = oneshot::channel();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let _ = read_synthetic_request(&mut socket).await;
                started.send(()).unwrap();
                release_rx.await.unwrap();
                write_synthetic_response(&mut socket, status, "").await;
                assert!(
                    tokio::time::timeout(Duration::from_millis(300), listener.accept())
                        .await
                        .is_err()
                );
            });
            let (sender, mut events) = provider_event_channel();
            let mut client = HighQualityTranslationClient::new_deeplx(
                "synthetic-asr",
                &endpoint,
                "",
                SourceLanguage::English,
                TargetLanguage::Japanese,
                sender,
            )
            .unwrap();
            client.stable_draft_delay = Duration::from_secs(60);
            client.maximum_wait_delay = Duration::from_secs(60);
            client
                .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                    text: "Rejected synthetic preview".into(),
                    language: Some("en".into()),
                })
                .await;
            let revision = client.inner.lock().await.draft_revision;
            client
                .start_preview(
                    "Rejected synthetic preview".into(),
                    Some("en".into()),
                    revision,
                )
                .await;
            tokio::time::timeout(Duration::from_secs(2), started_rx)
                .await
                .unwrap()
                .unwrap();
            client
                .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                    text: "Queued synthetic successor".into(),
                    language: Some("en".into()),
                })
                .await;
            let revision = client.inner.lock().await.draft_revision;
            client
                .start_preview(
                    "Queued synthetic successor".into(),
                    Some("en".into()),
                    revision,
                )
                .await;
            assert_eq!(
                client.inner.lock().await.pending_preview_revision,
                Some(revision)
            );
            release.send(()).unwrap();
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    match events.recv().await.unwrap() {
                        LiveTranslateServerEvent::Error { code, .. } if status == 401 => {
                            assert_eq!(code, "translation_authentication_failed");
                            break;
                        }
                        LiveTranslateServerEvent::TranslationDeferred(recovery)
                            if status == 429 && !recovery.retry_scheduled =>
                        {
                            assert_eq!(recovery.reason, TranslationRecoveryReason::RateLimited);
                            break;
                        }
                        LiveTranslateServerEvent::SubtitlePreviewPair { .. } => {
                            panic!("rejected request has no completed pair")
                        }
                        _ => {}
                    }
                }
                while client.inner.lock().await.preview_task.is_some() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            {
                let inner = client.inner.lock().await;
                assert!(inner.pending_preview_revision.is_none());
                assert_eq!(inner.pipeline_failed, status == 401);
                if status == 429 {
                    assert!(!inner
                        .preview_request_pacer
                        .suppression_remaining(std::time::Instant::now())
                        .is_zero());
                }
            }
            client
                .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                    text: "New speech after rejected preview".into(),
                    language: Some("en".into()),
                })
                .await;
            let revision = client.inner.lock().await.draft_revision;
            client
                .start_preview(
                    "New speech after rejected preview".into(),
                    Some("en".into()),
                    revision,
                )
                .await;
            assert!(client.inner.lock().await.preview_task.is_none());
            server.await.unwrap();
            client.disconnect().await;
        }
    }

    #[tokio::test]
    async fn returning_to_completed_candidate_cancels_a_different_waiting_preview() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (cancelled, cancelled_rx) = oneshot::channel::<std::time::Instant>();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            reply_to_synthetic_request(
                &mut socket,
                200,
                r#"{"code":200,"data":"Synthetic translation"}"#,
            )
            .await;
            let allowed_at = cancelled_rx.await.unwrap();
            let wait = allowed_at.saturating_duration_since(std::time::Instant::now())
                + Duration::from_millis(100);
            assert!(
                tokio::time::timeout(wait, listener.accept()).await.is_err(),
                "a reverted waiting candidate must never reach HTTP"
            );
        });
        let (sender, mut events) = provider_event_channel();
        let mut client = HighQualityTranslationClient::new_deeplx(
            "synthetic-asr",
            &endpoint,
            "",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            sender,
        )
        .unwrap();
        client.stable_draft_delay = Duration::from_secs(60);
        client.maximum_wait_delay = Duration::from_secs(60);
        let original = "Synthetic original preview";
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: original.into(),
                language: Some("en".into()),
            })
            .await;
        let revision = client.inner.lock().await.draft_revision;
        client
            .start_preview(original.into(), Some("en".into()), revision)
            .await;
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if matches!(
                    events.recv().await.unwrap(),
                    LiveTranslateServerEvent::TranslationDraft(text)
                        if text == "Synthetic translation"
                ) {
                    break;
                }
            }
            while client.inner.lock().await.preview_task.is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let replacement = "Synthetic original preview with substantially more new words";
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: replacement.into(),
                language: Some("en".into()),
            })
            .await;
        while events.try_recv().is_ok() {}
        let (revision, allowed_at) = {
            let inner = client.inner.lock().await;
            (
                inner.draft_revision,
                inner
                    .preview_request_pacer
                    .next_candidate_start_at(
                        std::time::Instant::now(),
                        PreviewCandidate::new(replacement, Some("en"), 0),
                    )
                    .unwrap(),
            )
        };
        client
            .start_preview(replacement.into(), Some("en".into()), revision)
            .await;
        tokio::task::yield_now().await;
        let waiting_id = client.inner.lock().await.preview_task.as_ref().unwrap().id;
        let late_partial = client.preview_partial_handler(waiting_id);

        let cosmetic = format!("{replacement}!");
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: cosmetic.clone(),
                language: Some("en".into()),
            })
            .await;
        let revision = client.inner.lock().await.draft_revision;
        client
            .start_preview(cosmetic, Some("en".into()), revision)
            .await;
        assert_eq!(
            client.inner.lock().await.preview_task.as_ref().unwrap().id,
            waiting_id,
            "same-candidate updates must preserve a waiting owner"
        );

        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: original.into(),
                language: Some("en".into()),
            })
            .await;
        let revision = client.inner.lock().await.draft_revision;
        client
            .start_preview(original.into(), Some("en".into()), revision)
            .await;
        {
            let inner = client.inner.lock().await;
            assert!(inner.preview_task.is_none());
            assert!(inner.preview_candidate.is_none());
            assert!(!client.preview_is_current(waiting_id));
        }
        assert_eq!(
            events.try_recv(),
            Ok(LiveTranslateServerEvent::SourceDraft {
                text: original.into(),
                language: Some("en".into()),
            })
        );
        assert_eq!(
            events.try_recv(),
            Ok(LiveTranslateServerEvent::TranslationDraft(String::new()))
        );
        late_partial("Late synthetic replacement".into());
        assert!(events.try_recv().is_err());
        cancelled.send(allowed_at).unwrap();
        server.await.unwrap();
        client.disconnect().await;
    }

    #[tokio::test]
    async fn cosmetic_drafts_preserve_an_identical_inflight_http_preview() {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (started, started_rx) = oneshot::channel();
        let (release, release_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let _ = read_synthetic_request(&mut socket).await;
            started.send(()).unwrap();
            release_rx.await.unwrap();
            let body = r#"{"code":200,"data":"Synthetic translation"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(100), listener.accept())
                    .await
                    .is_err()
            );
        });
        let (sender, mut events) = provider_event_channel();
        let mut client = HighQualityTranslationClient::new_deeplx(
            "synthetic-asr",
            &endpoint,
            "",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            sender,
        )
        .unwrap();
        client.stable_draft_delay = Duration::from_secs(60);
        client.maximum_wait_delay = Duration::from_secs(60);
        let original = "Synthetic inflight preview";
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: original.into(),
                language: Some("en".into()),
            })
            .await;
        let revision = client.inner.lock().await.draft_revision;
        client
            .start_preview(original.into(), Some("en".into()), revision)
            .await;
        tokio::time::timeout(Duration::from_secs(2), started_rx)
            .await
            .unwrap()
            .unwrap();
        let preview_id = client.inner.lock().await.preview_task.as_ref().unwrap().id;
        let mut controller = crate::core::session::TranslationSessionController::default();
        while let Ok(event) = events.try_recv() {
            controller.handle(event);
        }
        assert!(controller.state.is_translation_preview_pending);
        assert!(!controller.state.is_translation_pending);
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: "Synthetic changed candidate while HTTP remains active".into(),
                language: Some("en".into()),
            })
            .await;
        let revision = client.inner.lock().await.draft_revision;
        client
            .start_preview(
                "Synthetic changed candidate while HTTP remains active".into(),
                Some("en".into()),
                revision,
            )
            .await;
        assert_eq!(
            client.inner.lock().await.pending_preview_revision,
            Some(revision)
        );
        for text in ["Synthetic inflight preview", "Synthetic, inflight preview!"] {
            client
                .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                    text: text.into(),
                    language: Some("en".into()),
                })
                .await;
            let revision = client.inner.lock().await.draft_revision;
            client
                .start_preview(text.into(), Some("en".into()), revision)
                .await;
            assert_eq!(
                client.inner.lock().await.preview_task.as_ref().unwrap().id,
                preview_id
            );
            assert!(client.preview_is_current(preview_id));
            assert!(client.inner.lock().await.pending_preview_revision.is_none());
            assert!(events.try_recv().is_err());
        }
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let event = events.recv().await.unwrap();
                let finished = matches!(
                    event,
                    LiveTranslateServerEvent::PreviewTranslationFinished { request_id }
                        if request_id == preview_id
                );
                controller.handle(event);
                if finished {
                    break;
                }
            }
        })
        .await
        .unwrap();
        while let Ok(event) = events.try_recv() {
            controller.handle(event);
        }
        assert_eq!(
            controller.state.subtitles.translation.text,
            "Synthetic translation"
        );
        assert_eq!(
            controller.state.subtitles.preview_pair,
            Some(crate::core::models::PreviewSubtitlePair {
                utterance_id: None,
                source: original.into(),
                translation: "Synthetic translation".into(),
            })
        );
        assert!(!controller.state.is_translation_preview_pending);
        assert!(!controller.state.is_translation_pending);
        assert!(controller.state.subtitles.history.is_empty());
        server.await.unwrap();
        client.disconnect().await;
    }

    #[tokio::test]
    async fn final_preempts_preview_but_keeps_the_shared_http_budget_and_order() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for expected in ["First final", "Second final"] {
                let (mut socket, _) =
                    tokio::time::timeout(Duration::from_secs(2), listener.accept())
                        .await
                        .unwrap()
                        .unwrap();
                let request = read_synthetic_request(&mut socket).await;
                let request = String::from_utf8(request).unwrap();
                assert!(request.contains(expected));
                assert!(!request.contains("Unstarted preview"));
                use tokio::io::AsyncWriteExt;
                let body = r#"{"code":200,"data":"Confirmed translation"}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
            assert!(
                tokio::time::timeout(Duration::from_millis(800), listener.accept())
                    .await
                    .is_err()
            );
        });
        let (sender, mut events) = provider_event_channel();
        let mut client = HighQualityTranslationClient::new_deeplx(
            "synthetic-asr",
            &endpoint,
            "",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            sender,
        )
        .unwrap();
        client.stable_draft_delay = Duration::from_secs(60);
        client.maximum_wait_delay = Duration::from_secs(60);
        client
            .inner
            .lock()
            .await
            .preview_request_pacer
            .record_start(std::time::Instant::now());
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: "Unstarted preview".into(),
                language: Some("en".into()),
            })
            .await;
        let revision = client.inner.lock().await.draft_revision;
        client
            .start_preview("Unstarted preview".into(), Some("en".into()), revision)
            .await;
        tokio::time::sleep(Duration::from_millis(30)).await;
        let started = tokio::time::Instant::now();
        for text in ["First final", "Second final"] {
            client
                .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
                    text: text.into(),
                    language: Some("en".into()),
                })
                .await;
        }
        let mut sources = Vec::new();
        tokio::time::timeout(Duration::from_secs(3), async {
            while sources.len() < 2 {
                if let LiveTranslateServerEvent::SubtitleConfirmedPair { source, .. } =
                    events.recv().await.unwrap()
                {
                    sources.push(source);
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(sources, ["First final", "Second final"]);
        assert!(started.elapsed() >= Duration::from_millis(1_100));
        assert!(started.elapsed() < Duration::from_secs(3));
        client.disconnect().await;
        server.await.unwrap();
    }

    #[tokio::test]
    async fn disconnect_cancels_a_healthy_preview_wait_without_http_or_stale_events() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (sender, mut events) = provider_event_channel();
        let client = HighQualityTranslationClient::new_deeplx(
            "synthetic-asr",
            &endpoint,
            "",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            sender,
        )
        .unwrap();
        client
            .inner
            .lock()
            .await
            .preview_request_pacer
            .record_start(std::time::Instant::now());
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: "Cancelled before HTTP".into(),
                language: Some("en".into()),
            })
            .await;
        while events.try_recv().is_ok() {}
        let revision = client.inner.lock().await.draft_revision;
        client
            .start_preview("Cancelled before HTTP".into(), Some("en".into()), revision)
            .await;
        tokio::time::sleep(Duration::from_millis(30)).await;
        client.disconnect().await;
        assert!(
            tokio::time::timeout(Duration::from_millis(800), listener.accept())
                .await
                .is_err()
        );
        assert!(events.try_recv().is_err());
        let inner = client.inner.lock().await;
        assert!(inner.preview_task.is_none());
        let now = std::time::Instant::now();
        assert_eq!(inner.preview_request_pacer.next_start_at(now), now);
    }

    #[tokio::test]
    async fn exhausted_preview_reports_no_scheduled_retry_and_new_speech_keeps_cooldown() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for _ in 0..MAX_TRANSLATION_ATTEMPTS {
                let (mut socket, _) = listener.accept().await.unwrap();
                reply_to_synthetic_request(&mut socket, 503, "").await;
            }
            assert!(
                tokio::time::timeout(Duration::from_millis(300), listener.accept())
                    .await
                    .is_err()
            );
        });
        let (sender, mut events) = provider_event_channel();
        let mut client = HighQualityTranslationClient::new_deeplx(
            "synthetic-asr",
            &endpoint,
            "",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            sender,
        )
        .unwrap();
        client.stable_draft_delay = Duration::from_secs(60);
        client.maximum_wait_delay = Duration::from_secs(60);
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: "Bounded failed preview".into(),
                language: Some("en".into()),
            })
            .await;
        let revision = client.inner.lock().await.draft_revision;
        client
            .start_preview("Bounded failed preview".into(), Some("en".into()), revision)
            .await;
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                match events.recv().await.unwrap() {
                    LiveTranslateServerEvent::TranslationDeferred(recovery)
                        if !recovery.retry_scheduled =>
                    {
                        assert_eq!(
                            recovery.reason,
                            TranslationRecoveryReason::TemporarilyUnavailable
                        );
                        assert!(recovery.retry_after_ms > 0);
                        break;
                    }
                    LiveTranslateServerEvent::Error { .. } => {
                        panic!("preview failure must preserve capture")
                    }
                    _ => {}
                }
            }
        })
        .await
        .unwrap();
        {
            let inner = client.inner.lock().await;
            assert!(inner.preview_task.is_none());
            assert!(!inner.pipeline_failed);
            assert!(inner.mt_cooldown.is_some());
            assert_eq!(inner.mt_failure_streak, MAX_TRANSLATION_ATTEMPTS);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            events.try_recv().is_err(),
            "silence must not start infinite retry timers"
        );
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: "New speech during cooldown".into(),
                language: Some("en".into()),
            })
            .await;
        let revision = client.inner.lock().await.draft_revision;
        client
            .start_preview(
                "New speech during cooldown".into(),
                Some("en".into()),
                revision,
            )
            .await;
        tokio::time::timeout(Duration::from_millis(100), async {
            loop {
                if let LiveTranslateServerEvent::TranslationDeferred(recovery) =
                    events.recv().await.unwrap()
                {
                    assert!(recovery.retry_scheduled);
                    assert!(recovery.retry_after_ms > 0);
                    break;
                }
            }
        })
        .await
        .unwrap();
        client.disconnect().await;
        server.await.unwrap();
    }

    #[tokio::test]
    async fn a_preview_rate_limit_survives_cancellation_and_defers_ordered_finals() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            reply_to_synthetic_request(&mut socket, 429, "").await;
            drop(socket);
            let rejected_at = tokio::time::Instant::now();
            for translation in ["First confirmed", "Second confirmed"] {
                let (mut socket, _) = listener.accept().await.unwrap();
                assert!(rejected_at.elapsed() >= Duration::from_secs(4));
                tokio::time::sleep(Duration::from_millis(20)).await;
                let body = serde_json::json!({"code":200,"data":translation}).to_string();
                reply_to_synthetic_request(&mut socket, 200, &body).await;
            }
            assert!(
                tokio::time::timeout(Duration::from_millis(50), listener.accept())
                    .await
                    .is_err()
            );
        });
        let (sender, mut events) = provider_event_channel();
        let mut client = HighQualityTranslationClient::new_deeplx(
            "synthetic-asr",
            &endpoint,
            "",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            sender,
        )
        .unwrap();
        client.stable_draft_delay = Duration::from_secs(60);
        client.maximum_wait_delay = Duration::from_secs(60);
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: "Replaceable preview.".into(),
                language: Some("en".into()),
            })
            .await;
        let (timer_id, revision) = {
            let inner = client.inner.lock().await;
            (
                inner.draft_stability_task.as_ref().unwrap().id,
                inner.draft_revision,
            )
        };
        client
            .handle_draft_timer(DraftTimerKind::Stable, timer_id, revision)
            .await;
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let LiveTranslateServerEvent::TranslationDeferred(recovery) =
                    events.recv().await.unwrap()
                {
                    assert_eq!(recovery.reason, TranslationRecoveryReason::RateLimited);
                    assert!(recovery.retry_after_ms >= 29_000);
                    assert!(!recovery.retry_scheduled);
                    break;
                }
            }
        })
        .await
        .unwrap();
        // Additional ASR changes must not start replacement MT while the
        // thirty-second preview pause reserves capacity for server finals.
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: "New cumulative speech during rate limit".into(),
                language: Some("en".into()),
            })
            .await;
        let revision = client.inner.lock().await.draft_revision;
        client
            .start_preview(
                "New cumulative speech during rate limit".into(),
                Some("en".into()),
                revision,
            )
            .await;
        assert!(client.inner.lock().await.preview_task.is_none());
        for text in ["First authoritative line.", "Second authoritative line."] {
            client
                .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
                    text: text.into(),
                    language: Some("en".into()),
                })
                .await;
        }
        {
            let inner = client.inner.lock().await;
            assert!(inner.mt_cooldown.is_some());
            assert_eq!(inner.mt_failure_streak, 1);
            assert!(inner.preview_task.is_none());
        }
        let mut pairs = Vec::new();
        tokio::time::timeout(Duration::from_secs(8), async {
            while pairs.len() < 2 {
                match events.recv().await.unwrap() {
                    LiveTranslateServerEvent::SubtitleConfirmedPair {
                        source,
                        translation,
                        ..
                    } => pairs.push((source, translation)),
                    LiveTranslateServerEvent::Error { code, .. } => {
                        panic!("temporary cooldown must preserve finals: {code}")
                    }
                    _ => {}
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(
            pairs,
            vec![
                ("First authoritative line.".into(), "First confirmed".into()),
                (
                    "Second authoritative line.".into(),
                    "Second confirmed".into()
                )
            ]
        );
        client
            .wait_for_final_translations(Duration::from_secs(1))
            .await;
        let latency = client.translation_latency().unwrap();
        assert!(latency.milliseconds >= 20);
        assert!(
            latency.milliseconds < 1_000,
            "initial shared cooldown is excluded from HTTP request timing"
        );
        assert!(
            !client
                .inner
                .lock()
                .await
                .preview_request_pacer
                .suppression_remaining(std::time::Instant::now())
                .is_zero(),
            "successful finals must not refund the preview pause"
        );
        client.disconnect().await;
        server.await.unwrap();
    }

    #[tokio::test]
    async fn stopping_during_mt_cooldown_cancels_retry_and_clears_owned_state() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            reply_to_synthetic_request(&mut socket, 429, "").await;
            drop(socket);
            assert!(
                tokio::time::timeout(Duration::from_millis(200), listener.accept())
                    .await
                    .is_err()
            );
        });
        let (sender, mut events) = provider_event_channel();
        let client = HighQualityTranslationClient::new_deeplx(
            "synthetic-asr",
            &endpoint,
            "",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            sender,
        )
        .unwrap();
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
                text: "Synthetic final before stop.".into(),
                language: Some("en".into()),
            })
            .await;
        tokio::time::timeout(Duration::from_secs(1), async {
            while !matches!(
                events.recv().await.unwrap(),
                LiveTranslateServerEvent::TranslationDeferred(_)
            ) {}
        })
        .await
        .unwrap();
        client.finish().await;
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
                text: "A teardown final must not restart text requests.".into(),
                language: Some("en".into()),
            })
            .await;
        assert!(!client.mt_work_allowed.load(Ordering::SeqCst));
        while let Ok(event) = events.try_recv() {
            assert!(!matches!(
                event,
                LiveTranslateServerEvent::Error { .. }
                    | LiveTranslateServerEvent::SubtitleConfirmedPair { .. }
                    | LiveTranslateServerEvent::TranslationStarted
            ));
        }
        let inner = client.inner.lock().await;
        assert!(inner.final_worker.is_none());
        assert!(inner.final_queue.is_empty());
        assert!(inner.mt_cooldown.is_none());
        assert_eq!(inner.mt_failure_streak, 0);
        drop(inner);
        assert_eq!(client.translation_latency(), None);
        client.disconnect().await;
        server.await.unwrap();
    }

    #[tokio::test]
    async fn transient_finals_exhaust_bounded_retries_with_a_recoverable_error() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for _ in 0..MAX_TRANSLATION_ATTEMPTS {
                let (mut socket, _) = listener.accept().await.unwrap();
                reply_to_synthetic_request(&mut socket, 503, "").await;
            }
            assert!(
                tokio::time::timeout(Duration::from_millis(100), listener.accept())
                    .await
                    .is_err()
            );
        });
        let (sender, mut events) = provider_event_channel();
        let client = HighQualityTranslationClient::new_deeplx(
            "synthetic-asr",
            &endpoint,
            "",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            sender,
        )
        .unwrap();
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
                text: "Synthetic transient final.".into(),
                language: Some("en".into()),
            })
            .await;
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                match events.recv().await.unwrap() {
                    LiveTranslateServerEvent::Error { code, message } => {
                        assert_eq!(code, "translation_temporarily_unavailable");
                        assert_eq!(message, "translation_temporarily_unavailable");
                        break;
                    }
                    LiveTranslateServerEvent::SubtitleConfirmedPair { .. } => {
                        panic!("a rejected request cannot commit a final")
                    }
                    _ => {}
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(client.translation_latency(), None);
        client.disconnect().await;
        server.await.unwrap();
    }

    #[tokio::test]
    async fn authentication_failure_is_permanent_and_never_enters_cooldown() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            reply_to_synthetic_request(&mut socket, 401, "").await;
            drop(socket);
            assert!(
                tokio::time::timeout(Duration::from_millis(100), listener.accept())
                    .await
                    .is_err()
            );
        });
        let (sender, mut events) = provider_event_channel();
        let client = HighQualityTranslationClient::new_deeplx(
            "synthetic-asr",
            &endpoint,
            "",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            sender,
        )
        .unwrap();
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
                text: "Synthetic authentication check.".into(),
                language: Some("en".into()),
            })
            .await;
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                match events.recv().await.unwrap() {
                    LiveTranslateServerEvent::Error { code, message } => {
                        assert_eq!(code, "translation_authentication_failed");
                        assert_eq!(message, "credential_authentication_failed");
                        break;
                    }
                    LiveTranslateServerEvent::TranslationDeferred(_) => {
                        panic!("authentication cannot recover by waiting")
                    }
                    _ => {}
                }
            }
        })
        .await
        .unwrap();
        assert!(client.inner.lock().await.mt_cooldown.is_none());
        client.disconnect().await;
        server.await.unwrap();
    }

    #[tokio::test]
    async fn same_language_drafts_and_finals_skip_http_and_clear_request_latency() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        for (target, language) in [
            (TargetLanguage::English, "en"),
            (TargetLanguage::Japanese, "ja"),
            (TargetLanguage::SimplifiedChinese, "zh"),
            (TargetLanguage::SimplifiedChinese, "zh-CN"),
            (TargetLanguage::SimplifiedChinese, "zh-Hans"),
        ] {
            let (sender, mut events) = provider_event_channel();
            let client = HighQualityTranslationClient::new_deeplx(
                "synthetic-asr",
                &endpoint,
                "",
                SourceLanguage::Automatic,
                target,
                sender,
            )
            .unwrap();
            *client.translation_latency.lock().unwrap() = Some(TranslationLatency {
                milliseconds: 123,
                kind: TranslationLatencyKind::Request,
            });
            for text in ["Synthetic", "Synthetic unchanged line."] {
                client
                    .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                        text: text.into(),
                        language: Some(language.into()),
                    })
                    .await;
            }
            assert_eq!(
                events.try_recv(),
                Ok(LiveTranslateServerEvent::SourceDraft {
                    text: "Synthetic unchanged line.".into(),
                    language: Some(language.into()),
                })
            );
            assert_eq!(
                events.try_recv(),
                Ok(LiveTranslateServerEvent::TranslationDraft(
                    "Synthetic unchanged line.".into()
                ))
            );
            assert!(events.try_recv().is_err());
            assert_eq!(client.translation_latency(), None);
            {
                let inner = client.inner.lock().await;
                assert!(inner.preview_task.is_none());
                assert!(inner.draft_stability_task.is_none());
                assert!(inner.draft_maximum_wait_task.is_none());
            }
            let final_event = LiveTranslateServerEvent::SourceFinal {
                text: "Synthetic unchanged line.".into(),
                language: Some(language.into()),
            };
            client.handle_asr_event(final_event.clone()).await;
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(1), events.recv())
                    .await
                    .unwrap(),
                Some(LiveTranslateServerEvent::SubtitleConfirmedPair {
                    source_utterance_id: None,
                    utterance_id: 1,
                    source: "Synthetic unchanged line.".into(),
                    language: Some(language.into()),
                    translation: "Synthetic unchanged line.".into(),
                })
            );
            client.handle_asr_event(final_event).await;
            client
                .wait_for_final_translations(Duration::from_secs(1))
                .await;
            assert!(events.try_recv().is_err());
            assert_eq!(client.translation_latency(), None);
            client.disconnect().await;
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(50), listener.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn same_language_draft_cancels_a_cross_language_preview_and_rejects_its_late_events() {
        let (client, mut events) = test_client(TargetLanguage::Japanese, 20);
        let old_id = client.advance_preview_epoch();
        let old_partial = client.preview_partial_handler(old_id);
        client.inner.lock().await.preview_task = Some(TaskSlot {
            id: old_id,
            handle: tokio::spawn(std::future::pending()),
        });
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: "同じ言語のプレビュー。".into(),
                language: Some("ja".into()),
            })
            .await;
        old_partial("obsolete translation".into());
        client.emit_preview(
            old_id,
            LiveTranslateServerEvent::SourceDraft {
                text: "obsolete source".into(),
                language: Some("en".into()),
            },
        );
        assert_eq!(
            events.try_recv(),
            Ok(LiveTranslateServerEvent::SourceDraft {
                text: "同じ言語のプレビュー。".into(),
                language: Some("ja".into()),
            })
        );
        assert_eq!(
            events.try_recv(),
            Ok(LiveTranslateServerEvent::TranslationDraft(
                "同じ言語のプレビュー。".into()
            ))
        );
        assert!(events.try_recv().is_err());
        assert!(client.inner.lock().await.preview_task.is_none());
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: "An unknown-language preview.".into(),
                language: None,
            })
            .await;
        assert_eq!(
            events.try_recv(),
            Ok(LiveTranslateServerEvent::SourceDraft {
                text: "An unknown-language preview.".into(),
                language: None,
            })
        );
        assert_eq!(
            events.try_recv(),
            Ok(LiveTranslateServerEvent::TranslationDraft(String::new()))
        );
        let inner = client.inner.lock().await;
        assert!(inner.draft_stability_task.is_some());
        assert!(inner.draft_maximum_wait_task.is_some());
        drop(inner);
        client.reset_draft_state().await;
        old_partial("still obsolete".into());
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn mixed_language_finals_keep_order_and_unknown_language_still_requests_translation() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (request_seen, wait_for_request) = oneshot::channel();
        let (release, wait_for_release) = oneshot::channel();
        let server = tokio::spawn(async move {
            let mut request_seen = Some(request_seen);
            let mut wait_for_release = Some(wait_for_release);
            for translation in ["First translated", "Unknown translated"] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                assert!(socket.read(&mut request).await.unwrap() > 0);
                if let Some(request_seen) = request_seen.take() {
                    request_seen.send(()).unwrap();
                    wait_for_release.take().unwrap().await.unwrap();
                }
                let body = serde_json::json!({"code":200,"data":translation}).to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
            assert!(
                tokio::time::timeout(Duration::from_millis(50), listener.accept())
                    .await
                    .is_err()
            );
        });
        let (sender, mut events) = provider_event_channel();
        let client = HighQualityTranslationClient::new_deeplx(
            "synthetic-asr",
            &endpoint,
            "",
            SourceLanguage::Automatic,
            TargetLanguage::Japanese,
            sender,
        )
        .unwrap();
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
                text: "First English line.".into(),
                language: Some("en".into()),
            })
            .await;
        tokio::time::timeout(Duration::from_secs(1), wait_for_request)
            .await
            .unwrap()
            .unwrap();
        for (text, language) in [
            ("日本語はそのまま。", Some("ja")),
            ("Unknown language line.", None),
        ] {
            client
                .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
                    text: text.into(),
                    language: language.map(str::to_string),
                })
                .await;
        }
        let mut started = 0;
        while let Ok(event) = events.try_recv() {
            match event {
                LiveTranslateServerEvent::TranslationStarted => started += 1,
                LiveTranslateServerEvent::SubtitleConfirmedPair { .. } => {
                    panic!("later finals must wait for the active final")
                }
                _ => {}
            }
        }
        release.send(()).unwrap();
        let mut pairs = Vec::new();
        tokio::time::timeout(Duration::from_secs(2), async {
            while pairs.len() < 3 {
                match events.recv().await.unwrap() {
                    LiveTranslateServerEvent::TranslationStarted => started += 1,
                    LiveTranslateServerEvent::SubtitleConfirmedPair {
                        source,
                        translation,
                        ..
                    } => pairs.push((source, translation)),
                    _ => {}
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(
            pairs,
            vec![
                ("First English line.".into(), "First translated".into()),
                ("日本語はそのまま。".into(), "日本語はそのまま。".into()),
                ("Unknown language line.".into(), "Unknown translated".into()),
            ]
        );
        assert_eq!(started, 2);
        client
            .wait_for_final_translations(Duration::from_secs(1))
            .await;
        assert_eq!(
            client.translation_latency().unwrap().kind,
            TranslationLatencyKind::Request
        );
        client.disconnect().await;
        server.await.unwrap();
    }

    #[tokio::test]
    async fn a_deferred_same_language_preview_and_finish_use_no_text_request() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (sender, mut events) = provider_event_channel();
        let client = HighQualityTranslationClient::new_deeplx(
            "synthetic-asr",
            &format!("http://{}", listener.local_addr().unwrap()),
            "",
            SourceLanguage::Automatic,
            TargetLanguage::English,
            sender,
        )
        .unwrap();
        client.inner.lock().await.final_worker = Some(TaskSlot {
            id: 999,
            handle: tokio::spawn(std::future::pending()),
        });
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: "An unfinished synthetic line".into(),
                language: Some("en".into()),
            })
            .await;
        assert!(events.try_recv().is_err());
        abort_task(&mut client.inner.lock().await.final_worker);
        client.resume_pending_preview_if_final_lane_idle().await;
        let (timer_id, revision) = {
            let inner = client.inner.lock().await;
            (
                inner.draft_stability_task.as_ref().unwrap().id,
                inner.draft_revision,
            )
        };
        client
            .handle_draft_timer(DraftTimerKind::Stable, timer_id, revision)
            .await;
        for expected in [
            LiveTranslateServerEvent::SourceDraft {
                text: "An unfinished synthetic line".into(),
                language: Some("en".into()),
            },
            LiveTranslateServerEvent::TranslationDraft("An unfinished synthetic line".into()),
        ] {
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(1), events.recv())
                    .await
                    .unwrap(),
                Some(expected)
            );
        }
        client.flush_pending_draft().await;
        client.flush_pending_draft().await;
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), events.recv())
                .await
                .unwrap(),
            Some(LiveTranslateServerEvent::SubtitleConfirmedPair {
                source_utterance_id: None,
                utterance_id: 1,
                source: "An unfinished synthetic line".into(),
                language: Some("en".into()),
                translation: "An unfinished synthetic line".into(),
            })
        );
        client
            .wait_for_final_translations(Duration::from_secs(1))
            .await;
        assert!(events.try_recv().is_err());
        assert_eq!(client.translation_latency(), None);
        client.disconnect().await;
        assert!(
            tokio::time::timeout(Duration::from_millis(50), listener.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn deeplx_finals_preserve_pairs_and_order_through_real_http() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for translation in ["first translated", "second translated"] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                assert!(socket.read(&mut request).await.unwrap() > 0);
                tokio::time::sleep(Duration::from_millis(40)).await;
                let body = serde_json::json!({"code":200,"data":translation}).to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let (sender, mut receiver) = provider_event_channel();
        let client = HighQualityTranslationClient::new_deeplx(
            "synthetic-asr",
            &format!("http://{address}"),
            "",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            sender,
        )
        .unwrap();
        for text in ["first synthetic sentence.", "second synthetic sentence."] {
            client
                .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
                    text: text.into(),
                    language: Some("en".into()),
                })
                .await;
        }
        let mut pairs = Vec::new();
        tokio::time::timeout(Duration::from_secs(3), async {
            while pairs.len() < 2 {
                if let Some(LiveTranslateServerEvent::SubtitleConfirmedPair {
                    source,
                    translation,
                    ..
                }) = receiver.recv().await
                {
                    pairs.push((source, translation));
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(
            pairs,
            vec![
                (
                    "first synthetic sentence.".into(),
                    "first translated".into()
                ),
                (
                    "second synthetic sentence.".into(),
                    "second translated".into()
                )
            ]
        );
        let latency = client.translation_latency().unwrap();
        assert_eq!(latency.kind, TranslationLatencyKind::Request);
        assert!(latency.milliseconds >= 40);
        client.disconnect().await;
        assert_eq!(client.translation_latency(), None);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn openai_compatible_finals_preserve_pairs_and_order_through_real_http() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for translation in ["first translated", "second translated"] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                assert!(socket.read(&mut request).await.unwrap() > 0);
                tokio::time::sleep(Duration::from_millis(40)).await;
                let body = serde_json::json!({"choices":[{"finish_reason":"stop","message":{"content":translation}}]}).to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let (sender, mut receiver) = provider_event_channel();
        let mut client = HighQualityTranslationClient::new_openai_compatible(
            "synthetic-asr",
            &format!("http://{address}"),
            "synthetic-translation-key",
            "synthetic-model",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            sender,
        )
        .unwrap();
        client
            .set_network(
                ProviderNetwork::resolve(&crate::core::network_proxy::ProxyConfig {
                    mode: crate::core::network_proxy::ProxyMode::Direct,
                    url: None,
                })
                .unwrap(),
            )
            .unwrap();
        for text in ["first synthetic sentence.", "second synthetic sentence."] {
            client
                .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
                    text: text.into(),
                    language: Some("en".into()),
                })
                .await;
        }
        let mut pairs = Vec::new();
        tokio::time::timeout(Duration::from_secs(3), async {
            while pairs.len() < 2 {
                if let Some(LiveTranslateServerEvent::SubtitleConfirmedPair {
                    source,
                    translation,
                    ..
                }) = receiver.recv().await
                {
                    pairs.push((source, translation));
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(
            pairs,
            vec![
                (
                    "first synthetic sentence.".into(),
                    "first translated".into()
                ),
                (
                    "second synthetic sentence.".into(),
                    "second translated".into()
                )
            ]
        );
        let latency = client.translation_latency().unwrap();
        assert_eq!(latency.kind, TranslationLatencyKind::Request);
        assert!(latency.milliseconds >= 40);
        client.disconnect().await;
        assert_eq!(client.translation_latency(), None);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn original_mode_and_cancelled_final_workers_do_not_report_request_latency() {
        for target in [TargetLanguage::Original, TargetLanguage::SimplifiedChinese] {
            let (client, _events) = test_client(target, 20);
            *client.translation_latency.lock().unwrap() = Some(TranslationLatency {
                milliseconds: 150,
                kind: TranslationLatencyKind::Request,
            });
            if target == TargetLanguage::Original {
                assert_eq!(client.translation_latency(), None);
            }
            client.cancel_final_translations().await;
            assert_eq!(client.translation_latency(), None);
        }
    }

    #[tokio::test]
    async fn a_revised_final_clears_the_displayed_preview_before_its_translation_arrives() {
        use crate::core::session::TranslationSessionController;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (release_translation, wait_for_release) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            wait_for_release.await.unwrap();
            let body = r#"{"code":200,"data":"Confirmed translation"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        let (sender, mut events) = provider_event_channel();
        let client = HighQualityTranslationClient::new_deeplx(
            "synthetic-asr",
            &format!("http://{address}"),
            "",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            sender,
        )
        .unwrap();
        let mut controller = TranslationSessionController::default();
        controller.handle(LiveTranslateServerEvent::SourceDraft {
            text: "Provisional recognition".into(),
            language: Some("en".into()),
        });
        controller.handle(LiveTranslateServerEvent::TranslationDraft(
            "Provisional translation".into(),
        ));
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
                text: "Corrected recognition.".into(),
                language: Some("en".into()),
            })
            .await;

        // The HTTP response is gated, so a final or a partial cannot hide a
        // stale draft. The new source must arrive with an empty translation.
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let event = events.recv().await.unwrap();
                assert!(!matches!(
                    event,
                    LiveTranslateServerEvent::SubtitleConfirmedPair { .. }
                ));
                controller.handle(event);
                if controller.state.subtitles.source.text == "Corrected recognition."
                    && controller.state.subtitles.translation.text.is_empty()
                {
                    break;
                }
            }
        })
        .await
        .unwrap();
        assert!(controller.state.is_translation_pending);
        assert!(controller.state.subtitles.history.is_empty());

        release_translation.send(()).unwrap();
        let final_event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            final_event,
            LiveTranslateServerEvent::SubtitleConfirmedPair { .. }
        ));
        controller.handle(final_event);
        assert_eq!(controller.state.subtitles.history.len(), 1);
        let pair = &controller.state.subtitles.history[0];
        assert_eq!(pair.source, "Corrected recognition.");
        assert_eq!(pair.translation, "Confirmed translation");
        client.disconnect().await;
        server.await.unwrap();
    }

    #[tokio::test]
    async fn recognition_drafts_show_immediately_on_the_bounded_latest_value_lane() {
        let (client, mut events) = test_client(TargetLanguage::SimplifiedChinese, 100);
        for text in ["hi", "hello", " ... ", "  "] {
            client
                .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                    text: text.into(),
                    language: Some("en".into()),
                })
                .await;
        }
        assert_eq!(
            events.try_recv(),
            Ok(LiveTranslateServerEvent::SourceDraft {
                text: "hello".into(),
                language: Some("en".into()),
            })
        );
        assert_eq!(
            events.try_recv(),
            Ok(LiveTranslateServerEvent::TranslationDraft(String::new()))
        );
        assert!(events.try_recv().is_err());
        client.reset_draft_state().await;
    }

    #[tokio::test]
    async fn completed_preview_survives_duplicate_and_cosmetic_drafts_without_another_request() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (checked, checked_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            reply_to_synthetic_request(
                &mut socket,
                200,
                r#"{"code":200,"data":"Synthetic translation"}"#,
            )
            .await;
            checked_rx.await.unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(100), listener.accept())
                    .await
                    .is_err(),
                "duplicate and cosmetic drafts must not request translation"
            );
        });
        let (sender, mut events) = provider_event_channel();
        let mut client = HighQualityTranslationClient::new_deeplx(
            "synthetic-asr",
            &endpoint,
            "",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            sender,
        )
        .unwrap();
        client.stable_draft_delay = Duration::from_secs(60);
        client.maximum_wait_delay = Duration::from_secs(60);
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: "Synthetic preview".into(),
                language: Some("en".into()),
            })
            .await;
        let revision = client.inner.lock().await.draft_revision;
        client
            .start_preview("Synthetic preview".into(), Some("en".into()), revision)
            .await;

        let mut controller = crate::core::session::TranslationSessionController::default();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let event = events.recv().await.unwrap();
                let completed = matches!(
                    &event,
                    LiveTranslateServerEvent::TranslationDraft(text)
                        if text == "Synthetic translation"
                );
                controller.handle(event);
                if completed {
                    break;
                }
            }
            while client.inner.lock().await.preview_task.is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        while let Ok(event) = events.try_recv() {
            controller.handle(event);
        }
        assert!(!controller.state.is_translation_preview_pending);
        assert_eq!(controller.state.subtitles.source.text, "Synthetic preview");
        assert_eq!(
            controller.state.subtitles.translation.text,
            "Synthetic translation"
        );

        for text in [
            "Synthetic preview",
            "Synthetic, preview.",
            "Synthetic   preview!",
        ] {
            client
                .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                    text: text.into(),
                    language: Some("en".into()),
                })
                .await;
            for kind in [DraftTimerKind::Stable, DraftTimerKind::Maximum] {
                let (timer_id, revision) = {
                    let inner = client.inner.lock().await;
                    let slot = match kind {
                        DraftTimerKind::Stable => &inner.draft_stability_task,
                        DraftTimerKind::Maximum => &inner.draft_maximum_wait_task,
                    };
                    (slot.as_ref().unwrap().id, inner.draft_revision)
                };
                client.handle_draft_timer(kind, timer_id, revision).await;
                assert!(client.inner.lock().await.preview_task.is_none());
                assert!(
                    events.try_recv().is_err(),
                    "a skipped preview must not clear its displayed translation"
                );
            }
        }

        // Actual new words reach raw recognition immediately. The last
        // complete pair remains independently readable while MT waits.
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: "Synthetic preview grows".into(),
                language: Some("en".into()),
            })
            .await;
        controller.handle(events.try_recv().unwrap());
        controller.handle(events.try_recv().unwrap());
        assert_eq!(
            controller.state.subtitles.source.text,
            "Synthetic preview grows"
        );
        assert!(controller.state.subtitles.translation.text.is_empty());
        assert_eq!(
            controller.state.subtitles.preview_pair,
            Some(crate::core::models::PreviewSubtitlePair {
                utterance_id: None,
                source: "Synthetic preview".into(),
                translation: "Synthetic translation".into(),
            })
        );
        assert!(events.try_recv().is_err());
        checked.send(()).unwrap();
        server.await.unwrap();
        client.disconnect().await;
    }

    #[tokio::test]
    async fn short_unpunctuated_drafts_can_preview_on_both_timers_without_becoming_final() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for kind in [DraftTimerKind::Stable, DraftTimerKind::Maximum] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                assert!(socket.read(&mut request).await.unwrap() > 0);
                let body = r#"{"code":200,"data":"Synthetic preview"}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            });
            let (sender, mut events) = provider_event_channel();
            let mut client = HighQualityTranslationClient::new_deeplx(
                "synthetic-asr",
                &format!("http://{address}"),
                "",
                SourceLanguage::English,
                TargetLanguage::Japanese,
                sender,
            )
            .unwrap();
            client.stable_draft_delay = Duration::from_secs(60);
            client.maximum_wait_delay = Duration::from_secs(60);
            for text in ["hi", "hello"] {
                client
                    .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                        text: text.into(),
                        language: Some("en".into()),
                    })
                    .await;
            }
            let (timer_id, revision) = {
                let inner = client.inner.lock().await;
                let slot = match kind {
                    DraftTimerKind::Stable => &inner.draft_stability_task,
                    DraftTimerKind::Maximum => &inner.draft_maximum_wait_task,
                };
                (slot.as_ref().unwrap().id, inner.draft_revision)
            };
            client.handle_draft_timer(kind, timer_id, revision).await;
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    match events.recv().await.unwrap() {
                        LiveTranslateServerEvent::SourceDraft { text, .. } => {
                            assert_eq!(text, "hello");
                        }
                        LiveTranslateServerEvent::TranslationDraft(text) => {
                            if text.is_empty() {
                                continue;
                            }
                            assert_eq!(text, "Synthetic preview");
                            break;
                        }
                        LiveTranslateServerEvent::PreviewTranslationStarted { .. }
                        | LiveTranslateServerEvent::PreviewTranslationFinished { .. } => {}
                        LiveTranslateServerEvent::SubtitlePreviewPair {
                            source,
                            translation,
                            ..
                        } => {
                            assert_eq!(source, "hello");
                            assert_eq!(translation, "Synthetic preview");
                        }
                        event => panic!("preview must not publish a durable event: {event:?}"),
                    }
                }
            })
            .await
            .unwrap();
            assert_eq!(
                client.translation_latency().unwrap().kind,
                TranslationLatencyKind::Request
            );
            while let Ok(event) = events.try_recv() {
                assert!(matches!(
                    event,
                    LiveTranslateServerEvent::PreviewTranslationFinished { .. }
                ));
            }
            client.disconnect().await;
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn stable_timer_previews_the_whole_tail_and_replaces_only_a_complete_pair() {
        use tokio::io::AsyncWriteExt;
        let draft = "First synthetic sentence. More synthetic words";
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (started, started_rx) = oneshot::channel();
        let (release, release_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_synthetic_request(&mut socket).await;
            assert!(String::from_utf8(request).unwrap().contains(draft));
            started.send(()).unwrap();
            release_rx.await.unwrap();
            let body = r#"{"code":200,"data":"Complete synthetic translation"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        let (sender, mut events) = provider_event_channel();
        let mut client = HighQualityTranslationClient::new_deeplx(
            "synthetic-asr",
            &endpoint,
            "",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            sender,
        )
        .unwrap();
        client.stable_draft_delay = Duration::from_secs(60);
        client.maximum_wait_delay = Duration::from_secs(60);
        let previous = crate::core::models::PreviewSubtitlePair {
            utterance_id: None,
            source: "Previous synthetic source".into(),
            translation: "Previous synthetic translation".into(),
        };
        let mut controller = crate::core::session::TranslationSessionController::default();
        controller.handle(LiveTranslateServerEvent::SubtitlePreviewPair {
            source_utterance_id: None,
            source: previous.source.clone(),
            language: Some("en".into()),
            translation: previous.translation.clone(),
        });
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: draft.into(),
                language: Some("en".into()),
            })
            .await;
        let (timer_id, revision) = {
            let inner = client.inner.lock().await;
            (
                inner.draft_stability_task.as_ref().unwrap().id,
                inner.draft_revision,
            )
        };
        client
            .handle_draft_timer(DraftTimerKind::Stable, timer_id, revision)
            .await;
        tokio::time::timeout(Duration::from_secs(2), started_rx)
            .await
            .unwrap()
            .unwrap();
        while let Ok(event) = events.try_recv() {
            controller.handle(event);
        }
        assert!(controller.state.is_translation_preview_pending);
        assert!(!controller.state.is_translation_pending);
        assert_eq!(controller.state.subtitles.preview_pair, Some(previous));
        assert_eq!(controller.state.subtitles.source.text, draft);
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let event = events.recv().await.unwrap();
                let finished = matches!(
                    event,
                    LiveTranslateServerEvent::PreviewTranslationFinished { .. }
                );
                controller.handle(event);
                if finished {
                    break;
                }
            }
        })
        .await
        .unwrap();
        while let Ok(event) = events.try_recv() {
            controller.handle(event);
        }
        assert_eq!(
            controller.state.subtitles.preview_pair,
            Some(crate::core::models::PreviewSubtitlePair {
                utterance_id: None,
                source: draft.into(),
                translation: "Complete synthetic translation".into(),
            })
        );
        assert!(!controller.state.is_translation_preview_pending);
        assert!(!controller.state.is_translation_pending);
        assert!(controller.state.subtitles.history.is_empty());
        server.await.unwrap();
        client.disconnect().await;
    }

    #[tokio::test]
    async fn stable_timer_resets_while_maximum_timer_keeps_its_first_token() {
        let (client, _events) = test_client(TargetLanguage::SimplifiedChinese, 100);
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: "まだ話しています".into(),
                language: Some("ja".into()),
            })
            .await;
        let (first_stable, first_maximum) = {
            let inner = client.inner.lock().await;
            (
                inner.draft_stability_task.as_ref().unwrap().id,
                inner.draft_maximum_wait_task.as_ref().unwrap().id,
            )
        };

        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: "まだ話し続けています".into(),
                language: Some("ja".into()),
            })
            .await;
        let (second_stable, second_maximum) = {
            let inner = client.inner.lock().await;
            (
                inner.draft_stability_task.as_ref().unwrap().id,
                inner.draft_maximum_wait_task.as_ref().unwrap().id,
            )
        };

        assert_ne!(first_stable, second_stable);
        assert_eq!(first_maximum, second_maximum);
        client.reset_draft_state().await;
    }

    #[tokio::test]
    async fn stable_callback_clears_only_itself_and_preserves_maximum_wait() {
        let (mut client, _events) = test_client(TargetLanguage::SimplifiedChinese, 100);
        // This timer now starts a short-draft preview. Keep its request local
        // and pending until reset cancels it; no provider connection is needed.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        client.mt = Arc::new(TextTranslationClient::DeepLX(
            crate::clients::deeplx_client::DeepLXClient::new(
                &format!("http://{}", listener.local_addr().unwrap()),
                "",
                SourceLanguage::Japanese,
                TargetLanguage::SimplifiedChinese,
            )
            .unwrap(),
        ));
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: "短い未完の文".into(),
                language: Some("ja".into()),
            })
            .await;
        let (stable_id, maximum_id, revision) = {
            let inner = client.inner.lock().await;
            (
                inner.draft_stability_task.as_ref().unwrap().id,
                inner.draft_maximum_wait_task.as_ref().unwrap().id,
                inner.draft_revision,
            )
        };

        client
            .handle_draft_timer(DraftTimerKind::Stable, stable_id, revision)
            .await;
        let inner = client.inner.lock().await;
        assert!(inner.draft_stability_task.is_none());
        assert_eq!(
            inner.draft_maximum_wait_task.as_ref().map(|task| task.id),
            Some(maximum_id)
        );
        drop(inner);
        client.reset_draft_state().await;
    }

    #[tokio::test]
    async fn a_running_preview_keeps_its_source_until_replaced_instead_of_mixing_new_text() {
        let (client, mut events) = test_client(TargetLanguage::SimplifiedChinese, 20);
        let preview_id = client.advance_preview_epoch();
        client.inner.lock().await.preview_task = Some(TaskSlot {
            id: preview_id,
            handle: tokio::spawn(std::future::pending()),
        });
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: "new synthetic source".into(),
                language: Some("en".into()),
            })
            .await;
        assert!(events.try_recv().is_err());
        assert!(client.inner.lock().await.committer.has_pending_text());
        client.reset_draft_state().await;
    }

    #[tokio::test]
    async fn original_mode_commits_server_final_as_one_atomic_pair() {
        let (client, mut events) = test_client(TargetLanguage::Original, 20);
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
                text: "今日は晴れです。".into(),
                language: Some("ja".into()),
            })
            .await;

        assert_eq!(
            events.recv().await,
            Some(LiveTranslateServerEvent::SubtitleConfirmedPair {
                source_utterance_id: None,
                utterance_id: 1,
                source: "今日は晴れです。".into(),
                language: Some("ja".into()),
                translation: "今日は晴れです。".into(),
            })
        );
    }

    #[tokio::test]
    async fn server_final_cancels_timers_and_invalidates_an_inflight_preview() {
        let (client, _events) = test_client(TargetLanguage::SimplifiedChinese, 20);
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: "今日は晴れです。".into(),
                language: Some("ja".into()),
            })
            .await;
        let preview_id = client.advance_preview_epoch();
        {
            let mut inner = client.inner.lock().await;
            inner.preview_task = Some(TaskSlot {
                id: preview_id,
                handle: tokio::spawn(std::future::pending()),
            });
        }

        assert!(client
            .prepare_server_final("今日は晴れです。", None)
            .await
            .is_some());

        let inner = client.inner.lock().await;
        assert!(inner.draft_stability_task.is_none());
        assert!(inner.draft_maximum_wait_task.is_none());
        assert!(inner.preview_task.is_none());
        assert!(!inner.committer.has_pending_text());
        assert_ne!(client.preview_epoch.load(Ordering::SeqCst), preview_id);
    }

    #[tokio::test]
    async fn server_final_discards_late_source_partial_and_completed_previews() {
        let (client, mut events) = test_client(TargetLanguage::SimplifiedChinese, 20);
        let preview_id = client.advance_preview_epoch();
        let partial = client.preview_partial_handler(preview_id);
        assert!(client
            .prepare_server_final("confirmed", None)
            .await
            .is_some());
        let final_pair = LiveTranslateServerEvent::SubtitleConfirmedPair {
            source_utterance_id: None,
            utterance_id: 0,
            source: "confirmed".into(),
            language: Some("en".into()),
            translation: "已确认".into(),
        };
        client.emit(final_pair.clone());

        client.emit_preview(
            preview_id,
            LiveTranslateServerEvent::SourceDraft {
                text: "obsolete source".into(),
                language: Some("en".into()),
            },
        );
        partial("obsolete partial".into());
        client.emit_preview(
            preview_id,
            LiveTranslateServerEvent::TranslationDraft("obsolete completion".into()),
        );
        client.emit_preview(
            preview_id,
            LiveTranslateServerEvent::SubtitlePreviewPair {
                source_utterance_id: None,
                source: "obsolete source".into(),
                language: Some("en".into()),
                translation: "obsolete completion".into(),
            },
        );
        client.emit_preview(
            preview_id,
            LiveTranslateServerEvent::PreviewTranslationStarted {
                request_id: preview_id,
            },
        );
        client.emit_preview(
            preview_id,
            LiveTranslateServerEvent::PreviewTranslationFinished {
                request_id: preview_id,
            },
        );

        assert_eq!(events.try_recv(), Ok(final_pair));
        assert_eq!(
            events.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        );
    }

    #[tokio::test]
    async fn reset_preview_cannot_overwrite_or_clear_a_new_preview() {
        let (client, mut events) = test_client(TargetLanguage::SimplifiedChinese, 20);
        let old_id = client.advance_preview_epoch();
        let old_partial = client.preview_partial_handler(old_id);
        client.reset_draft_state().await;
        let current_id = client.advance_preview_epoch();
        let current_partial = client.preview_partial_handler(current_id);
        let current_source = LiveTranslateServerEvent::SourceDraft {
            text: "new source".into(),
            language: Some("en".into()),
        };
        client.emit_preview(current_id, current_source.clone());
        current_partial("新的预览".into());

        client.emit_preview(
            old_id,
            LiveTranslateServerEvent::SourceDraft {
                text: "obsolete source".into(),
                language: Some("en".into()),
            },
        );
        old_partial("obsolete partial".into());
        old_partial(String::new());
        client.emit_preview(
            old_id,
            LiveTranslateServerEvent::TranslationDraft(String::new()),
        );

        assert_eq!(events.try_recv(), Ok(current_source));
        assert_eq!(
            events.try_recv(),
            Ok(LiveTranslateServerEvent::TranslationDraft(
                "新的预览".into()
            ))
        );
        assert_eq!(
            events.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        );
    }

    #[tokio::test]
    async fn same_text_finish_tail_keeps_its_own_source_identity_in_active_and_queued_lanes() {
        for (earlier_id, tail_id) in [(Some(7), Some(8)), (None, None)] {
            for active in [false, true] {
                let (client, _events) = test_client(TargetLanguage::SimplifiedChinese, 20);
                let earlier = TranslationRequest {
                    text: "Synthetic repeated sentence".into(),
                    language: Some("en".into()),
                    boundary: FinalBoundary::ServerFinal,
                    utterance_revision: 1,
                    source_utterance_id: earlier_id,
                    content_revision: 0,
                    enqueued_at: tokio::time::Instant::now(),
                };
                {
                    let mut inner = client.inner.lock().await;
                    inner.final_worker = Some(TaskSlot {
                        id: 999,
                        handle: tokio::spawn(std::future::pending()),
                    });
                    if active {
                        inner.active_final = Some(earlier.key());
                    } else {
                        inner.final_queue.push_back(earlier);
                    }
                    inner.committer.update_draft("Synthetic repeated sentence");
                    inner.current_source_utterance_id = tail_id;
                    inner.latest_draft_language = Some("en".into());
                    inner.next_confirmation_id = 1;
                }
                client.flush_pending_draft().await;
                let inner = client.inner.lock().await;
                let identities: Vec<_> = inner
                    .final_queue
                    .iter()
                    .map(|request| {
                        (
                            request.source_utterance_id,
                            request.utterance_revision,
                            request.boundary,
                        )
                    })
                    .collect();
                let expected = if active {
                    vec![(tail_id, 2, FinalBoundary::SessionFinish)]
                } else {
                    vec![
                        (earlier_id, 1, FinalBoundary::ServerFinal),
                        (tail_id, 2, FinalBoundary::SessionFinish),
                    ]
                };
                assert_eq!(identities, expected);
                drop(inner);
                client.disconnect().await;
            }
        }
    }

    #[test]
    fn final_request_alias_requires_matching_source_identity_and_content_revision() {
        let request = |boundary, confirmation, source, revision| TranslationRequest {
            text: "Synthetic repeated sentence".into(),
            language: Some("en".into()),
            boundary,
            utterance_revision: confirmation,
            source_utterance_id: source,
            content_revision: revision,
            enqueued_at: tokio::time::Instant::now(),
        };
        let server = request(FinalBoundary::ServerFinal, 1, Some(7), 0);
        assert!(server
            .key()
            .matches(&request(FinalBoundary::SessionFinish, 2, Some(7), 0)));
        assert!(!server
            .key()
            .matches(&request(FinalBoundary::SessionFinish, 2, Some(8), 0)));
        assert!(!server
            .key()
            .matches(&request(FinalBoundary::SessionFinish, 2, None, 0)));
        assert!(!server
            .key()
            .matches(&request(FinalBoundary::SessionFinish, 2, Some(7), 1)));
        assert!(!server
            .key()
            .matches(&request(FinalBoundary::ServerFinal, 2, Some(8), 0)));
        let unknown = request(FinalBoundary::ServerFinal, 1, None, 0);
        assert!(!unknown
            .key()
            .matches(&request(FinalBoundary::SessionFinish, 2, None, 0)));
        assert!(unknown
            .key()
            .matches(&request(FinalBoundary::ServerFinal, 1, None, 0)));
    }

    #[tokio::test]
    async fn active_final_defers_preview_until_the_final_lane_is_idle() {
        let (client, mut events) = test_client(TargetLanguage::SimplifiedChinese, 20);
        {
            let mut inner = client.inner.lock().await;
            inner.final_worker = Some(TaskSlot {
                id: 999,
                handle: tokio::spawn(std::future::pending()),
            });
            inner.active_final = Some(FinalRequestKey {
                text: "earlier final".into(),
                boundary: FinalBoundary::ServerFinal,
                utterance_revision: 1,
                source_utterance_id: None,
                content_revision: 0,
            });
        }

        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: "まだ話し続けています".into(),
                language: Some("ja".into()),
            })
            .await;

        let pending_revision = {
            let inner = client.inner.lock().await;
            assert!(inner.committer.has_pending_text());
            assert!(inner.draft_stability_task.is_none());
            assert!(inner.draft_maximum_wait_task.is_none());
            assert!(inner.preview_task.is_none());
            inner.draft_revision
        };
        assert!(events.try_recv().is_err());

        {
            let mut inner = client.inner.lock().await;
            abort_task(&mut inner.final_worker);
            inner.active_final = None;
        }
        client.resume_pending_preview_if_final_lane_idle().await;

        let inner = client.inner.lock().await;
        assert_eq!(inner.draft_revision, pending_revision);
        assert!(inner.draft_stability_task.is_some());
        assert!(inner.draft_maximum_wait_task.is_some());
        drop(inner);
        client.reset_draft_state().await;
    }

    #[tokio::test]
    async fn identical_server_final_is_only_deduplicated_without_a_new_draft() {
        let (client, mut events) = test_client(TargetLanguage::Original, 20);
        let final_event = LiveTranslateServerEvent::SourceFinal {
            text: "はい。".into(),
            language: Some("ja".into()),
        };
        client.handle_asr_event(final_event.clone()).await;
        client.handle_asr_event(final_event.clone()).await;
        assert!(matches!(
            events.recv().await,
            Some(LiveTranslateServerEvent::SubtitleConfirmedPair { .. })
        ));
        assert!(events.try_recv().is_err());

        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: "はい".into(),
                language: Some("ja".into()),
            })
            .await;
        let _ = events.recv().await;
        let _ = events.recv().await;
        client.handle_asr_event(final_event).await;
        assert!(matches!(
            events.recv().await,
            Some(LiveTranslateServerEvent::SubtitleConfirmedPair { .. })
        ));
    }

    #[tokio::test]
    async fn repeated_accepted_finals_keep_identity_through_coalesced_drafts_and_duplicate_replays()
    {
        // Exercise both direct original confirmation and the serial final
        // worker's same-language branch, without issuing a provider request.
        for target in [TargetLanguage::Original, TargetLanguage::Japanese] {
            let (client, mut events) = test_client(target, 20);
            for _ in 0..2 {
                client
                    .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                        text: "Synthetic repeated lyric".into(),
                        language: Some("ja".into()),
                    })
                    .await;
                client
                    .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
                        text: "Synthetic repeated lyric".into(),
                        language: Some("ja".into()),
                    })
                    .await;
            }
            client
                .wait_for_final_translations(Duration::from_secs(1))
                .await;
            let mut controller = crate::core::session::TranslationSessionController::default();
            controller.archive_mut().begin(true, 0);
            let mut ids = Vec::new();
            let mut replay = None;
            while let Ok(event) = events.try_recv() {
                let LiveTranslateServerEvent::SubtitleConfirmedPair { utterance_id, .. } = &event
                else {
                    panic!(
                        "the reliable final barrier must have coalesced every raw draft: {event:?}"
                    );
                };
                ids.push(*utterance_id);
                replay = Some(event.clone());
                controller.handle(event);
            }
            assert_eq!(ids, [1, 2]);
            assert_eq!(controller.state.subtitles.history.len(), 2);
            assert_eq!(controller.archive().count(), 2);
            assert!(
                controller.state.subtitles.history[0].created_at_ms
                    < controller.state.subtitles.history[1].created_at_ms
            );
            client.emit(replay.unwrap());
            controller.handle(events.try_recv().unwrap());
            assert_eq!(controller.state.subtitles.history.len(), 2);
            assert_eq!(controller.archive().count(), 2);
            assert!(events.try_recv().is_err());
            client.disconnect().await;
        }
    }

    #[tokio::test]
    async fn audio3_sentence_ids_keep_repeated_finals_when_both_draft_lanes_are_coalesced() {
        use crate::core::protocols::audio3::Audio3ASRServerEventDecoder;

        for target in [TargetLanguage::Original, TargetLanguage::Japanese] {
            let (client, mut confirmed_events) = test_client(target, 20);
            let (source_events, mut source_receiver) = provider_event_channel();
            // Queue everything before either consumer runs. The reliable source
            // finals suppress every draft, including the only new-sentence hint.
            for sentence_id in [1, 2, 2] {
                for is_final in [false, true] {
                    let message = serde_json::json!({
                        "header": {"event": "result-generated"},
                        "payload": {"output": {"sentence": {
                            "text": "Synthetic repeated lyric",
                            "sentence_id": sentence_id,
                            "sentence_end": is_final
                        }}}
                    });
                    let event = Audio3ASRServerEventDecoder::decode(&message.to_string())
                        .unwrap()
                        .subtitle_event(SourceLanguage::Japanese);
                    source_events.send(event).unwrap();
                }
            }
            let mut received_source_ids = Vec::new();
            while let Ok(event) = source_receiver.try_recv() {
                let LiveTranslateServerEvent::SourceUtteranceFinal { utterance_id, .. } = &event
                else {
                    panic!("a reliable source boundary must supersede the draft: {event:?}");
                };
                received_source_ids.push(*utterance_id);
                client.handle_asr_event(event).await;
            }
            assert_eq!(received_source_ids, [1, 2, 2]);
            client
                .wait_for_final_translations(Duration::from_secs(1))
                .await;
            let mut controller = crate::core::session::TranslationSessionController::default();
            controller.archive_mut().begin(true, 0);
            let mut received_confirmation_ids = Vec::new();
            let mut confirmed_source_ids = Vec::new();
            while let Ok(event) = confirmed_events.try_recv() {
                let LiveTranslateServerEvent::SubtitleConfirmedPair {
                    utterance_id,
                    source_utterance_id,
                    ..
                } = &event
                else {
                    panic!("expected only completed confirmations: {event:?}");
                };
                received_confirmation_ids.push(*utterance_id);
                confirmed_source_ids.push(*source_utterance_id);
                controller.handle(event);
            }
            assert_eq!(received_confirmation_ids, [1, 2]);
            assert_eq!(confirmed_source_ids, [Some(1), Some(2)]);
            assert_eq!(controller.state.subtitles.history.len(), 2);
            assert_eq!(controller.archive().count(), 2);
            client.disconnect().await;
        }
    }

    #[tokio::test]
    async fn replayed_source_identity_cannot_cancel_new_draft_and_resets_per_task() {
        let (client, _events) = test_client(TargetLanguage::SimplifiedChinese, 20);
        assert_eq!(
            client
                .prepare_server_final("Synthetic final", Some(7))
                .await,
            Some(1)
        );
        let epoch = client.advance_preview_epoch();
        {
            let mut inner = client.inner.lock().await;
            inner.committer.update_draft("Synthetic next sentence");
            inner.latest_draft_language = Some("en".into());
            inner.preview_task = Some(TaskSlot {
                id: epoch,
                handle: tokio::spawn(std::future::pending()),
            });
        }
        for id in [7, 6] {
            assert!(client
                .prepare_server_final("Synthetic final", Some(id))
                .await
                .is_none());
            let inner = client.inner.lock().await;
            assert!(inner.committer.has_pending_text());
            assert_eq!(inner.draft_revision, 1);
            assert_eq!(inner.latest_draft_language.as_deref(), Some("en"));
            assert_eq!(inner.preview_task.as_ref().map(|task| task.id), Some(epoch));
            assert_eq!(client.preview_epoch.load(Ordering::SeqCst), epoch);
        }
        client.reset_draft_state().await;
        assert_eq!(
            client
                .prepare_server_final("Synthetic final", Some(1))
                .await,
            Some(1)
        );
        client.disconnect().await;
    }

    #[tokio::test]
    async fn identified_drafts_cannot_resurrect_a_confirmed_sentence_or_replace_a_newer_one() {
        use crate::core::protocols::audio3::Audio3ASRServerEventDecoder;

        let draft = |id, text| {
            let frame = serde_json::json!({
                "header": {"event": "result-generated"},
                "payload": {"output": {"sentence": {
                    "sentence_id": id, "text": text, "sentence_end": false
                }}}
            });
            Audio3ASRServerEventDecoder::decode(&frame.to_string())
                .unwrap()
                .subtitle_event(SourceLanguage::English)
        };
        let (client, _events) = test_client(TargetLanguage::Original, 20);
        client
            .prepare_server_final("Synthetic repeated lyric", Some(7))
            .await;
        client
            .handle_asr_event(draft(7, "Synthetic repeated lyric"))
            .await;
        assert!(!client.inner.lock().await.committer.has_pending_text());

        client
            .handle_asr_event(draft(8, "Synthetic repeated lyric"))
            .await;
        assert_eq!(
            client
                .inner
                .lock()
                .await
                .committer
                .preview_latest_draft(false)
                .as_deref(),
            Some("Synthetic repeated lyric")
        );
        client
            .handle_asr_event(draft(7, "Synthetic stale revision"))
            .await;
        assert_eq!(
            client
                .inner
                .lock()
                .await
                .committer
                .preview_latest_draft(false)
                .as_deref(),
            Some("Synthetic repeated lyric")
        );
        client.disconnect().await;
    }

    #[tokio::test]
    async fn an_older_final_keeps_an_already_received_new_sentence_candidate() {
        use crate::core::protocols::audio3::Audio3ASRServerEventDecoder;

        let (client, _events) = test_client(TargetLanguage::Original, 20);
        let frame = serde_json::json!({
            "header": {"event": "result-generated"},
            "payload": {"output": {"sentence": {
                "sentence_id": 8, "text": "Synthetic next sentence", "sentence_end": false
            }}}
        });
        client
            .handle_asr_event(
                Audio3ASRServerEventDecoder::decode(&frame.to_string())
                    .unwrap()
                    .subtitle_event(SourceLanguage::English),
            )
            .await;
        assert!(client
            .prepare_server_final("Synthetic previous sentence", Some(7))
            .await
            .is_some());
        assert_eq!(
            client
                .inner
                .lock()
                .await
                .committer
                .preview_latest_draft(false)
                .as_deref(),
            Some("Synthetic next sentence")
        );
        client.disconnect().await;
    }

    #[tokio::test]
    async fn a_real_new_begin_cancels_only_the_old_preview_and_keeps_quota_and_final_order() {
        use crate::core::protocols::audio3::Audio3ASRServerEventDecoder;

        let (client, mut events) = test_client(TargetLanguage::Japanese, 20);
        client.prepare_identified_draft(7).await;
        while events.try_recv().is_ok() {}
        let preview_id = client.advance_preview_epoch();
        let late_partial = client.preview_partial_handler(preview_id);
        let now = std::time::Instant::now();
        let cooldown_until = tokio::time::Instant::now() + Duration::from_secs(8);
        let (shared_at, preview_at) = {
            let mut inner = client.inner.lock().await;
            inner.committer.update_draft("Synthetic old draft");
            inner.preview_task = Some(TaskSlot {
                id: preview_id,
                handle: tokio::spawn(std::future::pending()),
            });
            inner.preview_http_pending = Some(preview_id);
            inner.pending_preview_revision = Some(inner.draft_revision);
            inner.mt_failure_streak = 2;
            inner.mt_cooldown = Some(MTCooldown {
                until: cooldown_until,
                reason: TranslationRecoveryReason::RateLimited,
            });
            inner.preview_request_pacer.record_start(now);
            inner.preview_request_pacer.suppress_after_rate_limit(now);
            for id in [5, 6] {
                inner.final_queue.push_back(TranslationRequest {
                    text: "Synthetic durable source".into(),
                    language: Some("en".into()),
                    boundary: FinalBoundary::ServerFinal,
                    utterance_revision: id,
                    source_utterance_id: Some(id),
                    content_revision: client.content_revision(),
                    enqueued_at: tokio::time::Instant::now(),
                });
            }
            (
                inner.preview_request_pacer.next_shared_start_at(now),
                inner.preview_request_pacer.next_start_at(now),
            )
        };
        let frame = serde_json::json!({
            "header": {"event": "result-generated"},
            "payload": {"output": {"sentence": {
                "sentence_id": 8, "text": "", "sentence_begin": true, "sentence_end": false
            }}}
        });
        client
            .handle_asr_event(
                Audio3ASRServerEventDecoder::decode(&frame.to_string())
                    .unwrap()
                    .subtitle_event(SourceLanguage::English),
            )
            .await;
        late_partial("Synthetic obsolete callback".into());
        assert_eq!(
            events.try_recv().unwrap(),
            LiveTranslateServerEvent::PreviewTranslationFinished {
                request_id: preview_id
            }
        );
        assert_eq!(
            events.try_recv().unwrap(),
            LiveTranslateServerEvent::SubtitlePreviewCleared
        );
        assert!(
            matches!(events.try_recv().unwrap(), LiveTranslateServerEvent::SourceUtteranceDraft { utterance_id: 8, text, .. } if text.is_empty())
        );
        assert!(events.try_recv().is_err());
        let inner = client.inner.lock().await;
        assert_eq!(inner.current_source_utterance_id, Some(8));
        assert!(inner.preview_task.is_none());
        assert!(inner.pending_preview_revision.is_none());
        assert!(!inner.committer.has_pending_text());
        assert_eq!(inner.mt_failure_streak, 2);
        assert_eq!(inner.mt_cooldown.unwrap().until, cooldown_until);
        assert_eq!(
            inner.preview_request_pacer.next_shared_start_at(now),
            shared_at
        );
        assert_eq!(inner.preview_request_pacer.next_start_at(now), preview_at);
        assert_eq!(
            inner
                .final_queue
                .iter()
                .map(|request| request.source_utterance_id)
                .collect::<Vec<_>>(),
            [Some(5), Some(6)]
        );
        drop(inner);
        client.disconnect().await;
    }

    #[tokio::test]
    async fn same_identified_sentence_revisions_keep_the_active_preview_owner() {
        let (client, mut events) = test_client(TargetLanguage::SimplifiedChinese, 20);
        client.prepare_identified_draft(8).await;
        while events.try_recv().is_ok() {}
        let preview_id = client.advance_preview_epoch();
        {
            let mut inner = client.inner.lock().await;
            inner.preview_task = Some(TaskSlot {
                id: preview_id,
                handle: tokio::spawn(std::future::pending()),
            });
            inner.preview_http_pending = Some(preview_id);
        }
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceDraft {
                utterance_id: 8,
                text: "Synthetic revised sentence".into(),
                language: Some("en".into()),
            })
            .await;
        assert_eq!(client.preview_epoch.load(Ordering::SeqCst), preview_id);
        assert_eq!(
            client.inner.lock().await.preview_task.as_ref().unwrap().id,
            preview_id
        );
        while let Ok(event) = events.try_recv() {
            assert!(!matches!(
                event,
                LiveTranslateServerEvent::SubtitlePreviewCleared
            ));
        }
        client.disconnect().await;
    }

    #[tokio::test]
    async fn starting_an_older_final_does_not_replace_a_newer_source_still_queued_in_the_outer_lane(
    ) {
        let (client, mut events) = test_client(TargetLanguage::Japanese, 20);
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceDraft {
                utterance_id: 8,
                text: "Synthetic next source".into(),
                language: Some("ja".into()),
            })
            .await;
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceFinal {
                utterance_id: 7,
                text: "Synthetic previous source".into(),
                language: Some("ja".into()),
            })
            .await;
        client
            .wait_for_final_translations(Duration::from_secs(1))
            .await;
        let mut controller = crate::core::session::TranslationSessionController::default();
        while let Ok(event) = events.try_recv() {
            controller.handle(event);
        }
        assert_eq!(controller.state.subtitles.history.len(), 1);
        assert_eq!(
            controller.state.subtitles.history[0].source,
            "Synthetic previous source"
        );
        assert_eq!(
            controller.state.subtitles.source.text,
            "Synthetic next source"
        );
        assert!(!controller.state.subtitles.source.is_final);
        assert_eq!(
            client.inner.lock().await.current_source_utterance_id,
            Some(8)
        );
        assert_eq!(
            client
                .inner
                .lock()
                .await
                .committer
                .preview_latest_draft(false)
                .as_deref(),
            Some("Synthetic next source")
        );
        client.disconnect().await;
    }

    #[tokio::test]
    async fn oversized_source_cannot_claim_identity_replace_a_candidate_or_bypass_into_finals() {
        let oversized = "x".repeat(crate::core::models::MAX_SUBTITLE_TEXT_BYTES + 1);
        for target in [TargetLanguage::Original, TargetLanguage::Japanese] {
            let (client, mut events) = test_client(target, 20);
            {
                let mut inner = client.inner.lock().await;
                inner.committer.update_draft("Synthetic prior candidate");
                inner.draft_revision = 5;
                inner.last_source_utterance_id = Some(7);
            }
            for event in [
                LiveTranslateServerEvent::SourceDraft {
                    text: oversized.clone(),
                    language: Some("ja".into()),
                },
                LiveTranslateServerEvent::SourceUtteranceFinal {
                    utterance_id: 8,
                    text: oversized.clone(),
                    language: Some("ja".into()),
                },
            ] {
                client.handle_asr_event(event).await;
                assert_eq!(
                    events.try_recv(),
                    Ok(LiveTranslateServerEvent::text_limit_error())
                );
            }
            client
                .start_preview(oversized.clone(), Some("ja".into()), 5)
                .await;
            assert_eq!(
                events.try_recv(),
                Ok(LiveTranslateServerEvent::text_limit_error())
            );
            client
                .enqueue_final(
                    oversized.clone(),
                    Some("ja".into()),
                    FinalBoundary::ServerFinal,
                    5,
                    None,
                )
                .await;
            assert_eq!(
                events.try_recv(),
                Ok(LiveTranslateServerEvent::text_limit_error())
            );
            assert!(client
                .prepare_server_final(&oversized, Some(8))
                .await
                .is_none());
            let inner = client.inner.lock().await;
            assert_eq!(inner.draft_revision, 5);
            assert_eq!(inner.last_source_utterance_id, Some(7));
            assert_eq!(
                inner.committer.preview_latest_draft(false).as_deref(),
                Some("Synthetic prior candidate")
            );
            assert!(inner.preview_task.is_none());
            assert!(inner.final_worker.is_none());
            assert!(inner.final_queue.is_empty());
            assert!(events.try_recv().is_err());
            drop(inner);
            client.disconnect().await;
        }
    }

    #[tokio::test]
    async fn final_queue_has_a_hard_limit_and_reports_overload_once() {
        let (client, mut events) = test_client(TargetLanguage::SimplifiedChinese, 20);
        {
            let mut inner = client.inner.lock().await;
            inner.final_worker = Some(TaskSlot {
                id: 999,
                handle: tokio::spawn(std::future::pending()),
            });
        }

        for revision in 1..=MAX_FINAL_QUEUE_DEPTH as u64 {
            client
                .enqueue_final(
                    format!("server final {revision}"),
                    Some("ja".into()),
                    FinalBoundary::ServerFinal,
                    revision,
                    None,
                )
                .await;
        }
        client
            .enqueue_final(
                "overflow".into(),
                Some("ja".into()),
                FinalBoundary::ServerFinal,
                99,
                None,
            )
            .await;
        client
            .enqueue_final(
                "second overflow".into(),
                Some("ja".into()),
                FinalBoundary::ServerFinal,
                100,
                None,
            )
            .await;

        let inner = client.inner.lock().await;
        assert_eq!(inner.final_queue.len(), MAX_FINAL_QUEUE_DEPTH);
        assert!(inner.pipeline_failed);
        drop(inner);
        assert_eq!(
            events.recv().await,
            Some(LiveTranslateServerEvent::Error {
                code: OVERLOAD_ERROR_CODE.into(),
                message: OVERLOAD_ERROR_MESSAGE.into(),
            })
        );
        assert!(events.try_recv().is_err());
        client.cancel_final_translations().await;
    }

    #[tokio::test]
    async fn finish_ack_waits_for_the_bridge_to_consume_the_queued_final() {
        let (client, mut events) = test_client(TargetLanguage::Original, 20);
        let (asr_events, asr_receiver) = provider_event_channel();
        client.install_asr_bridge(asr_receiver).await;
        let mut finish_ack = client.take_asr_bridge_finish_ack().await.unwrap();

        // Hold the draft/final state so the bridge cannot finish consuming the
        // authoritative final. The per-connection acknowledgement uses separate
        // bridge state, so observing it never shares this lock.
        let inner = client.inner.lock().await;
        asr_events
            .send(LiveTranslateServerEvent::SourceFinal {
                text: "最後の字幕。".into(),
                language: Some("ja".into()),
            })
            .unwrap();
        asr_events
            .send(LiveTranslateServerEvent::SessionFinished)
            .unwrap();
        tokio::task::yield_now().await;
        assert!(matches!(
            finish_ack.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));

        drop(inner);
        assert!(tokio::time::timeout(Duration::from_secs(1), finish_ack)
            .await
            .unwrap()
            .is_ok());
        assert_eq!(
            events.recv().await,
            Some(LiveTranslateServerEvent::SubtitleConfirmedPair {
                source_utterance_id: None,
                utterance_id: 1,
                source: "最後の字幕。".into(),
                language: Some("ja".into()),
                translation: "最後の字幕。".into(),
            })
        );
        assert_eq!(
            events.recv().await,
            Some(LiveTranslateServerEvent::SessionFinished)
        );
        client.disconnect_asr_bridge().await;
    }

    #[tokio::test]
    async fn finish_ack_is_scoped_to_one_bridge_connection() {
        let (client, _events) = test_client(TargetLanguage::Original, 20);

        let (first_events, first_receiver) = provider_event_channel();
        client.install_asr_bridge(first_receiver).await;
        first_events
            .send(LiveTranslateServerEvent::SessionFinished)
            .unwrap();
        assert!(
            client
                .wait_for_asr_bridge_finish(Duration::from_secs(1))
                .await
        );

        let (_second_events, second_receiver) = provider_event_channel();
        client.install_asr_bridge(second_receiver).await;
        assert!(
            !client
                .wait_for_asr_bridge_finish(Duration::from_millis(25))
                .await
        );
        client.disconnect_asr_bridge().await;
    }
}
