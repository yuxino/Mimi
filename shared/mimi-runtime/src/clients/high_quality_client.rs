//! Bounded recognition + independently configured text-translation pipeline.
//!
//! Replaceable ASR drafts use a latest-only preview lane. Only authoritative
//! server finals (plus a bounded session-finish fallback) enter the durable,
//! serial final-translation queue.
//!
//! This entry owns construction, connection/finish/Clear order and shared state.
//! `recognition` accepts source events, `preview` owns replaceable work,
//! `final_translation` owns durable ordering, and `request_budget` arbitrates
//! both lanes. `text_translation` adapts the selected independent destination.

mod final_translation;
mod preview;
mod recognition;
mod request_budget;
#[cfg(test)]
mod tests;
mod text_translation;

use text_translation::TextTranslationClient;

use crate::clients::provider_events::{
    provider_event_channel, ProviderEventReceiver, ProviderEventSender,
};
use crate::clients::provider_network::{ProviderNetwork, ProviderNetworkError};
use crate::clients::qwen_mt_client::QwenMTClient;
use crate::clients::recognition_client::{RecognitionClient, RecognitionClientError};
use crate::core::committer::ASRDraftCommitter;
use crate::core::configuration::LiveTranslationConfiguration;
#[cfg(test)]
use crate::core::credentials::ProviderCredentials;
use crate::core::credentials::TextTranslationCredentials;
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
        status_code: if error.is_authentication_failure() || error.is_missing_credentials() {
            401
        } else if error.is_invalid_configuration() {
            400
        } else if error.is_timeout() {
            408
        } else if error.is_unreachable() {
            503
        } else {
            0
        },
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

/// Recognition identity, replaceable preview ownership and durable final
/// admission share one lock. A final must invalidate preview work atomically;
/// separating source files must not create independently mutable lane state.
struct PipelineState {
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

impl PipelineState {
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
pub struct HighQualityTranslationClient {
    asr_client: RecognitionClient,
    mt: Arc<TextTranslationClient>,
    source_language: SourceLanguage,
    target_language: TargetLanguage,
    translates_audio: bool,
    events: ProviderEventSender,
    state: Arc<Mutex<PipelineState>>,
    asr_bridge: Arc<Mutex<ASRBridgeState>>,
    /// Serializes incoming recognition content with an explicit Clear.
    content_operation: Arc<Mutex<()>>,
    preview_epoch: Arc<AtomicU64>,
    mt_work_allowed: Arc<AtomicBool>,
    translation_latency: Arc<std::sync::Mutex<Option<TranslationLatency>>>,
    stable_draft_delay: Duration,
    maximum_wait_delay: Duration,
    streams_finals: bool,
}

impl HighQualityTranslationClient {
    pub(crate) fn network_endpoints(&self) -> (Option<url::Url>, Option<url::Url>) {
        (
            self.asr_client.network_endpoint(),
            self.mt.network_endpoint(),
        )
    }

    #[cfg(test)]
    pub(super) fn qwen_model_for_test(&self) -> Option<(QwenMTModel, bool)> {
        match self.mt.as_ref() {
            TextTranslationClient::Qwen(_, model) => Some((*model, self.streams_finals)),
            _ => None,
        }
    }

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
            state: Arc::new(Mutex::new(PipelineState {
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

    pub fn new_apple(
        asr_key: &str,
        source: SourceLanguage,
        target: TargetLanguage,
        events: ProviderEventSender,
    ) -> Result<Self, QwenMTClientError> {
        let asr = RecognitionClient::alibaba(asr_key, source)
            .map_err(|_| QwenMTClientError::MissingAPIKey)?;
        let mt = if target.translates_audio() {
            TextTranslationClient::Apple(
                crate::clients::apple_translation_client::AppleTranslationClient::new(
                    source, target,
                )?,
            )
        } else {
            TextTranslationClient::Disabled
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

    /// All standalone recognizers consume the same independently typed text
    /// destination. The speech credential never supplies a missing text key.
    fn with_independent_text(
        asr: RecognitionClient,
        configuration: &LiveTranslationConfiguration,
        events: ProviderEventSender,
    ) -> Result<Self, QwenMTClientError> {
        let mt = TextTranslationClient::independent(configuration)?;
        Ok(Self::from_components(
            asr,
            mt,
            configuration.source_language,
            configuration.target_language,
            Duration::from_millis(250),
            Duration::from_millis(1_000),
            12,
            false,
            events,
        ))
    }

    pub fn new_xai(
        configuration: &LiveTranslationConfiguration,
        events: ProviderEventSender,
    ) -> Result<Self, QwenMTClientError> {
        let key = configuration
            .credentials
            .direct_api_key()
            .ok_or(QwenMTClientError::MissingAPIKey)?;
        let asr = RecognitionClient::xai(key, configuration.source_language)
            .map_err(recognition_error)?;
        Self::with_independent_text(asr, configuration, events)
    }

    pub fn new_alibaba_independent(
        configuration: &LiveTranslationConfiguration,
        events: ProviderEventSender,
    ) -> Result<Self, QwenMTClientError> {
        let key = configuration
            .credentials
            .alibaba_key()
            .ok_or(QwenMTClientError::MissingAPIKey)?;
        let asr = RecognitionClient::alibaba(key, configuration.source_language)
            .map_err(recognition_error)?;
        Self::with_independent_text(asr, configuration, events)
    }

    pub fn new_custom(
        configuration: &LiveTranslationConfiguration,
        events: ProviderEventSender,
    ) -> Result<Self, QwenMTClientError> {
        let asr = RecognitionClient::standalone(configuration).map_err(recognition_error)?;
        Self::with_independent_text(asr, configuration, events)
    }

    /// Connects the recognizer and resets all draft/final workers.
    pub async fn connect(&self) -> Result<(), QwenMTClientError> {
        self.mt_work_allowed.store(false, Ordering::SeqCst);
        let budget = self
            .state
            .lock()
            .await
            .preview_request_pacer
            .export_budget();
        self.reset_draft_state().await;
        self.cancel_final_translations().await;
        self.restore_request_budget(budget).await;
        self.disconnect_asr_bridge().await;

        // Fail before opening recognition or capture. Readiness never downloads.
        if let TextTranslationClient::Apple(client) = self.mt.as_ref() {
            client.check_ready().await?;
        }
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

    pub async fn finish(&self) {
        // A user stop during service backoff cancels MT immediately. New ASR
        // teardown finals must not restart requests while the bridge drains.
        let has_mt_cooldown = { self.state.lock().await.mt_cooldown.is_some() };
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

    pub fn content_revision(&self) -> u64 {
        self.events.content_revision()
    }

    /// Discards local work without disconnecting ASR or refunding an already
    /// started request's spacing, quota suppression or shared cooldown.
    pub async fn clear_content(&self) -> u64 {
        let _content = self.content_operation.lock().await;
        let revision = {
            let mut inner = self.state.lock().await;
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
