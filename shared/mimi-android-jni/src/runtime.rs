//! Thin native lifecycle adapter. Provider transport, queues, recovery schedule
//! and subtitle state are the same Rust implementations as the desktop shell.
//!
//! This entry owns session handles, PCM admission and connection lifetime.
//! Configuration metadata, offline contract views and diagnostic-probe handles
//! have separate owners; every provider/subtitle decision still runs in shared Rust.

mod configuration;
mod contracts;
mod probes;
#[cfg(test)]
mod tests;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use mimi_runtime::{
    audio::send_pipeline::{AudioIngress, AudioIngressError, AudioSendPipeline},
    clients::{
        provider_events::{
            provider_event_channel, ProviderEvent, ProviderEventReceiver, ProviderEventSender,
        },
        provider_network::ProviderNetwork,
        translation_client::TranslationClient,
    },
    core::{
        audio_input::{AudioInput, AudioSource},
        configuration::LiveTranslationConfiguration,
        credentials::{ProviderCredentials, ProviderCredentialsError},
        diagnostics::TranslationRecovery,
        models::{
            SessionStatus, SourceLanguage, SubtitleSnapshot, TargetLanguage, TranslationMode,
        },
        network_proxy::ProxyConfig,
        protocols::{
            live_translate::LiveTranslateServerEvent,
            volcano_engine::{VolcanoEngineEndpoint, VolcanoEngineServerEvent},
        },
        provider::{
            AlibabaRealtimeMode, ProviderKind, ProviderPreferences, ServiceProfile,
            SpeechTranslationMode, TextTranslation, XAISpeechMode,
        },
        recovery::{
            advance_lifecycle_sequence_if_current, provider_error_is_retryable,
            provider_recovery_minimum_delay, MTBudgetContinuity, MTBudgetScope, RecoveryContinuity,
            RecoverySchedule,
        },
        session::TranslationSessionController,
    },
    resource_cleanup::{CleanupOwners, CleanupTicket},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
};
use tokio::{
    runtime::Runtime,
    sync::{oneshot, watch, Notify},
    task::JoinHandle,
};

const MAX_SESSIONS: usize = 2;
const MAX_REQUEST: usize = 8 * 1024 * 1024;
const MAX_PCM: usize = 192_000;
pub(crate) fn executor() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("native_runtime_unavailable")
    })
}
fn sessions() -> &'static Mutex<HashMap<u64, Arc<Session>>> {
    static SESSIONS: OnceLock<Mutex<HashMap<u64, Arc<Session>>>> = OnceLock::new();
    SESSIONS.get_or_init(Default::default)
}
fn session(id: u64) -> Result<Arc<Session>, &'static str> {
    sessions()
        .lock()
        .map_err(|_| "native_runtime_failed")?
        .get(&id)
        .cloned()
        .ok_or("native_runtime_stale_handle")
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StageNetworkInput {
    speech_proxy: ProxyConfig,
    text_proxy: ProxyConfig,
    trust_roots: Vec<String>,
}

impl StageNetworkInput {
    /// Resolve independent immutable speech/text routes from one native snapshot.
    fn resolve(self) -> Result<(ProviderNetwork, ProviderNetwork), &'static str> {
        let roots: Vec<Vec<u8>> = self
            .trust_roots
            .iter()
            .map(|root| STANDARD.decode(root).map_err(|_| "native_network_invalid"))
            .collect::<Result<_, _>>()?;
        let speech = ProviderNetwork::resolve(&self.speech_proxy)
            .and_then(|network| network.with_trust_roots(roots.clone()))
            .map_err(|_| "native_network_invalid")?;
        let text = ProviderNetwork::resolve(&self.text_proxy)
            .and_then(|network| network.with_trust_roots(roots))
            .map_err(|_| "native_network_invalid")?;
        Ok((speech, text))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotMetadata {
    version: u64,
    finished: bool,
    error_code: Option<String>,
    history_save_error: bool,
    history_limited: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FullSnapshot {
    #[serde(flatten)]
    metadata: SnapshotMetadata,
    connection_status: &'static str,
    snapshot: SubtitleSnapshot,
    atomic_preview: Option<bool>,
    detected_language: Option<String>,
    is_translation_pending: bool,
    is_translation_preview_pending: bool,
    is_translation_timed_out: bool,
    translation_recovery: Option<TranslationRecovery>,
}

#[derive(Serialize)]
struct UnchangedSnapshot {
    unchanged: bool,
    #[serde(flatten)]
    metadata: SnapshotMetadata,
}

struct Connection {
    cleanup_id: u64,
    client: TranslationClient,
    pipeline: AudioSendPipeline,
    ready: watch::Sender<bool>,
    events: ProviderEventSender,
    budget: Option<mimi_runtime::core::preview_pacing::MTRequestBudget>,
}

struct ConnectionIngress {
    audio: AudioIngress,
    events: ProviderEventSender,
}

impl ConnectionIngress {
    fn try_send(&self, data: Vec<u8>) -> Result<(), AudioIngressError> {
        let result = self.audio.try_send(data);
        if result == Err(AudioIngressError::Backpressure) {
            // The shared ingress fails closed and reports Backpressure once.
            // Keep its error on the same connection's recovery lane, even if
            // this native submission outlives a connection replacement.
            let _ = self.events.send(LiveTranslateServerEvent::Error {
                code: "transport_error".into(),
                message: "transport_error".into(),
            });
        }
        result
    }
}

/// Drop the connection/pipeline before releasing its final-disconnect owner.
/// The ticket then starts that owner even if the lease was never awaited.
struct ConnectionLease {
    connection: Connection,
    release: oneshot::Sender<()>,
    cleanup: CleanupTicket,
}

impl ConnectionLease {
    async fn disconnected(self) {
        let Self {
            connection,
            release,
            cleanup,
        } = self;
        drop(connection);
        drop(release);
        cleanup.await;
    }
}
struct AbortTask(JoinHandle<()>);
impl AbortTask {
    fn abort(&self) {
        self.0.abort();
    }
}
impl Drop for AbortTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}
struct Session {
    audio_source: AudioSource,
    history: Mutex<Option<crate::history::Lease>>,
    history_status: Mutex<mimi_runtime::session_journal::JournalState>,
    configuration: LiveTranslationConfiguration,
    speech_network: ProviderNetwork,
    text_network: ProviderNetwork,
    controller: Mutex<TranslationSessionController>,
    connection: Mutex<Option<Connection>>,
    cleanup_owners: CleanupOwners,
    cleanup_sequence: AtomicU64,
    task: Mutex<Option<JoinHandle<()>>>,
    epoch: AtomicU64,
    version: AtomicU64,
    started: AtomicBool,
    stopping: AtomicBool,
    finished: AtomicBool,
    signal: Notify,
    error_code: Mutex<Option<String>>,
    budget: Mutex<MTBudgetContinuity>,
    recovery_continuity: Mutex<RecoveryContinuity>,
}
impl Session {
    #[cfg(test)]
    fn snapshot(&self) -> Value {
        serde_json::from_str(&self.snapshot_json(None).unwrap()).unwrap()
    }

    fn snapshot_json(&self, since_version: Option<u64>) -> Result<String, &'static str> {
        let controller = self.controller.lock().unwrap();
        // Terminal observations are not part of the controller revision.
        // Capture them with the view before releasing its guard, including
        // when a late finish does not require another full snapshot.
        let finished = self.finished.load(Ordering::SeqCst);
        let error_code = self.error_code.lock().unwrap().clone();
        let version = self.version.load(Ordering::SeqCst);
        let history_status = *self.history_status.lock().unwrap();
        let metadata = SnapshotMetadata {
            version,
            finished,
            error_code,
            history_save_error: history_status.save_error,
            history_limited: history_status.limited,
        };
        if since_version == Some(version) {
            drop(controller);
            return serde_json::to_string(&UnchangedSnapshot {
                unchanged: true,
                metadata,
            })
            .map_err(|_| "native_runtime_failed");
        }
        let state = &controller.state;
        let connection_status = match state.status {
            SessionStatus::Idle => "idle",
            SessionStatus::Connecting => "connecting",
            SessionStatus::Listening => "listening",
            SessionStatus::Stopping => "stopping",
            SessionStatus::Error(_) => "error",
        };
        let snapshot = FullSnapshot {
            metadata,
            connection_status,
            snapshot: state.subtitles.clone(),
            atomic_preview: state.atomic_preview,
            detected_language: state
                .detected_language
                .as_ref()
                .map(|language| language.code.clone()),
            is_translation_pending: state.is_translation_pending,
            is_translation_preview_pending: state.is_translation_preview_pending,
            is_translation_timed_out: state.is_translation_timed_out,
            translation_recovery: state.translation_recovery,
        };
        drop(controller);
        // Serialize the captured owned view directly, avoiding a second full
        // JSON tree and keeping serialization outside the controller lock.
        serde_json::to_string(&snapshot).map_err(|_| "native_runtime_failed")
    }

    fn publish(&self, update: impl FnOnce(&mut TranslationSessionController)) {
        let mut controller = self.controller.lock().unwrap();
        update(&mut controller);
        self.version.fetch_add(1, Ordering::SeqCst);
    }

    fn apply_event(&self, event: ProviderEvent) {
        self.publish(|controller| {
            let application = event.apply_to(controller, self.audio_source);
            if let Some(pair) = application.confirmed_pair {
                if let Some(history) = self.history.lock().unwrap().as_ref() {
                    history.append(&pair);
                    *self.history_status.lock().unwrap() = history.state();
                }
            }
        });
    }

    fn release_history(&self) {
        if let Some(history) = self.history.lock().unwrap().take() {
            *self.history_status.lock().unwrap() = history.release();
        }
    }

    fn mark_finished(&self) {
        self.release_history();
        self.finished.store(true, Ordering::SeqCst);
    }

    fn tick(&self, now: std::time::Instant) {
        let mut controller = self.controller.lock().unwrap();
        if controller.tick(now) {
            self.version.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn connection(&self, generation: u64) -> Result<ProviderEventReceiver, &'static str> {
        let (events, receiver) = provider_event_channel();
        let client = TranslationClient::new_with_stage_networks(
            &self.configuration,
            events.clone(),
            self.speech_network.clone(),
            self.text_network.clone(),
        )
        .map_err(|_| "invalid_configuration")?;
        self.publish(|controller| controller.set_atomic_preview(client.uses_atomic_preview()));
        let (ready, ready_rx) = watch::channel(false);
        let sender = client.clone();
        let failures = events.clone();
        let pipeline = AudioSendPipeline::spawn_with_error_handler(
            move |data| {
                let sender = sender.clone();
                let mut ready = ready_rx.clone();
                async move {
                    while !*ready.borrow_and_update() {
                        ready.changed().await.map_err(|_| None)?;
                    }
                    sender.send_audio(&data).await.map_err(Some)
                }
            },
            move |error| {
                let label = error
                    .as_ref()
                    .and_then(|error| error.recognition_failure_label())
                    .unwrap_or("transport_error");
                let _ = failures.send(LiveTranslateServerEvent::Error {
                    code: label.into(),
                    message: label.into(),
                });
            },
        );
        client.set_audio_pending_gate(pipeline.pending_pcm_gate());
        let scope = MTBudgetScope::for_configuration("android-active".into(), &self.configuration);
        let budget = self.budget.lock().unwrap().prepare(generation, scope);
        // Cleanup identity belongs to this actual connection, independently
        // of a lifecycle generation or another source's connection.
        let cleanup_id = self
            .cleanup_sequence
            .fetch_add(1, Ordering::SeqCst)
            .wrapping_add(1);
        let cleanup_id = if cleanup_id == 0 {
            self.cleanup_sequence.fetch_add(1, Ordering::SeqCst) + 1
        } else {
            cleanup_id
        };
        *self.connection.lock().unwrap() = Some(Connection {
            cleanup_id,
            client,
            pipeline,
            ready,
            events,
            budget,
        });
        Ok(receiver)
    }
    fn ingress(&self) -> Option<ConnectionIngress> {
        let slot = self.connection.lock().unwrap();
        let connection = slot.as_ref()?;
        Some(ConnectionIngress {
            audio: connection.pipeline.ingress()?,
            events: connection.events.clone(),
        })
    }

    fn take_connection_owned(&self) -> Option<ConnectionLease> {
        let mut slot = self.connection.lock().unwrap();
        let connection = slot.as_ref()?;
        let client = connection.client.clone();
        let (release, released) = oneshot::channel();
        // Register while the slot is still occupied. Every path taking this
        // client must leave an awaitable owner before cancellation can occur.
        let cleanup = self.cleanup_owners.register(
            connection.cleanup_id,
            (),
            move || async move {
                let _ = released.await;
                let _ = tokio::time::timeout(
                    mimi_runtime::core::health::DISCONNECT_TIMEOUT,
                    client.disconnect(),
                )
                .await;
            },
            None,
        );
        Some(ConnectionLease {
            connection: slot.take().expect("the connection slot is locked"),
            release,
            cleanup,
        })
    }

    async fn retire(&self, generation: u64) {
        let connection = self.take_connection_owned();
        if let Some(connection) = connection {
            connection.connection.pipeline.stop();
            let lease = self.budget.lock().unwrap().take_lease(generation);
            if let Some((token, scope)) = lease {
                if let Some(budget) = connection
                    .connection
                    .client
                    .suspend_mt_request_budget()
                    .await
                {
                    self.budget.lock().unwrap().remember(token, scope, budget);
                }
            }
            connection.disconnected().await;
        }
    }

    fn begin_start(self: &Arc<Self>) -> Result<(), &'static str> {
        // A caller can retain this Session just before Stop removes its handle.
        // Publish the drive atomically with Stop's task snapshot, so neither a
        // late connection install nor an unregistered drive can escape cleanup.
        let mut task = self.task.lock().unwrap();
        if self.started.swap(true, Ordering::SeqCst) {
            return Err("native_runtime_already_started");
        }
        if self.stopping.load(Ordering::SeqCst) {
            return Err("native_runtime_stale_handle");
        }
        self.recovery_continuity.lock().unwrap().clear();
        let _runtime = executor().enter();
        let receiver = self.connection(1)?;
        *task = Some(executor().spawn(self.clone().drive(receiver)));
        Ok(())
    }

    fn begin_stop(self: &Arc<Self>) -> CleanupTicket {
        self.stopping.store(true, Ordering::SeqCst);
        self.epoch.fetch_add(1, Ordering::SeqCst);
        self.recovery_continuity.lock().unwrap().clear();
        let task = self.task.lock().unwrap().take();
        if let Some(task) = &task {
            task.abort();
        }
        let native = Arc::clone(self);
        // JNI is also called from JVM threads outside Tokio. Capture the same
        // executor as Start without blocking that thread on network teardown.
        let _runtime = executor().enter();
        CleanupOwners::default().register(
            1,
            (),
            move || async move {
                if let Some(task) = task {
                    // A cancelled connect can still be executing synchronous
                    // installation. Join it before sweeping the final slot.
                    let _ = task.await;
                }
                native.retire(0).await;
                native.cleanup_owners.wait_registered().await;
                native.release_history();
            },
            None,
        )
    }
    fn fail(&self, code: &str) {
        *self.error_code.lock().unwrap() = Some(code.into());
        self.publish(|controller| controller.did_fail(code));
        self.mark_finished();
    }
    fn record_health_success(&self, generation: u64, now: std::time::Instant) -> bool {
        let mut continuity = self.recovery_continuity.lock().unwrap();
        if self.epoch.load(Ordering::SeqCst) != generation || self.stopping.load(Ordering::SeqCst) {
            return false;
        }
        continuity.health_succeeded(generation, now)
    }
    fn claim_recovery(
        &self,
        generation: u64,
        recovery: &mut Option<RecoverySchedule>,
        minimum_delay: std::time::Duration,
    ) -> bool {
        // Health success and failure ownership must linearize under the same
        // lock. A retired probe may finish its synchronous tail after abort.
        let mut continuity = self.recovery_continuity.lock().unwrap();
        let Some(next) = advance_lifecycle_sequence_if_current(&self.epoch, generation) else {
            return false;
        };
        if let Some(schedule) = recovery.as_mut() {
            schedule.accept_failure(next)
        } else {
            *recovery = Some(continuity.take_for_failure(generation, next, minimum_delay));
            true
        }
    }
    async fn drive(self: Arc<Self>, mut receiver: ProviderEventReceiver) {
        let mut generation = 1;
        let mut recovery: Option<RecoverySchedule> = None;
        loop {
            if self.stopping.load(Ordering::SeqCst) {
                self.finish_connection(&mut receiver).await;
                return;
            }
            let client = self
                .connection
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .client
                .clone();
            let budget = self.connection.lock().unwrap().as_ref().unwrap().budget;
            client.restore_mt_request_budget(budget).await;
            let result = tokio::select! {
                result = client.connect() => result,
                _ = self.signal.notified() => { self.finish_connection(&mut receiver).await; return; }
            };
            let mut failure = result.err().map(|error| error.safe_code().to_string());
            if failure.is_none() {
                if let Some(connection) = self.connection.lock().unwrap().as_ref() {
                    connection.ready.send_replace(true);
                }
                self.publish(TranslationSessionController::did_connect);
                if let Some(schedule) = recovery.take() {
                    let mut continuity = self.recovery_continuity.lock().unwrap();
                    if self.epoch.load(Ordering::SeqCst) == generation
                        && !self.stopping.load(Ordering::SeqCst)
                    {
                        continuity.record_connected(schedule, std::time::Instant::now());
                    }
                }
                let health_client = client.clone();
                let health_session = Arc::clone(&self);
                let health_events = self
                    .connection
                    .lock()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .events
                    .clone();
                let health = AbortTask(tokio::spawn(async move {
                    loop {
                        tokio::time::sleep(mimi_runtime::core::health::HEALTH_CHECK_INTERVAL).await;
                        if let Err(error) =
                            mimi_runtime::core::health::probe_connection_health(&health_client)
                                .await
                        {
                            let code = error
                                .recognition_failure_label()
                                .unwrap_or_else(|| error.safe_code());
                            let _ = health_events.send(LiveTranslateServerEvent::Error {
                                code: code.into(),
                                message: code.into(),
                            });
                            return;
                        }
                        health_session.record_health_success(generation, std::time::Instant::now());
                    }
                }));
                let mut ticks =
                    tokio::time::interval(mimi_runtime::core::health::SNAPSHOT_PUBLISH_INTERVAL);
                loop {
                    tokio::select! {
                        event = receiver.recv_with_revision() => {
                            let Some(event) = event else { failure = Some("transport_error".into()); break; };
                            if event.is_content() && event.content_revision != client.content_revision() {continue;}
                            match &event.event {
                                LiveTranslateServerEvent::Error {code,..} => { failure = Some(mimi_runtime::core::recovery::normalize_provider_error_code(code).into()); break; }
                                LiveTranslateServerEvent::SessionFinished => {
                                    health.abort();
                                    self.publish(TranslationSessionController::did_stop);
                                    self.retire(generation).await;
                                    self.mark_finished();
                                    return;
                                }
                                _ => self.apply_event(event),
                            }
                        }
                        _ = ticks.tick() => {
                            self.tick(std::time::Instant::now());
                        }
                        _ = self.signal.notified() => {health.abort();self.finish_connection(&mut receiver).await;return;}
                    }
                }
                health.abort();
            }
            let code = failure.as_deref().unwrap_or("transport_error");
            self.retire(generation).await;
            if self.stopping.load(Ordering::SeqCst) {
                self.publish(TranslationSessionController::did_stop);
                self.mark_finished();
                return;
            }
            if !provider_error_is_retryable(code) {
                self.fail(code);
                return;
            }
            if !self.claim_recovery(
                generation,
                &mut recovery,
                provider_recovery_minimum_delay(code),
            ) {
                return;
            }
            let schedule = recovery.as_mut().unwrap();
            let Some(attempt) = schedule.next_attempt() else {
                self.fail(code);
                return;
            };
            self.publish(TranslationSessionController::begin_connecting);
            tokio::select! { _ = tokio::time::sleep(attempt.delay) => {}, _ = self.signal.notified() => {self.publish(TranslationSessionController::did_stop);self.mark_finished();return;} }
            generation = if attempt.index == 0 {
                schedule.generation()
            } else {
                match schedule.advance_for_retry(&self.epoch) {
                    Some(value) => value,
                    None => return,
                }
            };
            if !schedule.owns_generation(self.epoch.load(Ordering::SeqCst)) {
                return;
            }
            match self.connection(generation) {
                Ok(value) => receiver = value,
                Err(code) => {
                    self.fail(code);
                    return;
                }
            }
        }
    }
    async fn finish_connection(&self, receiver: &mut ProviderEventReceiver) {
        self.recovery_continuity.lock().unwrap().clear();
        self.publish(TranslationSessionController::begin_stopping);
        let connection = self.take_connection_owned();
        if let Some(connection) = connection {
            {
                let resources = &connection.connection;
                let finish = async {
                    resources
                        .pipeline
                        .finish(mimi_runtime::core::health::AUDIO_DRAIN_TIMEOUT)
                        .await;
                    let _ = tokio::time::timeout(
                        resources.client.finish_timeout(),
                        resources.client.finish(),
                    )
                    .await;
                };
                tokio::pin!(finish);
                loop {
                    tokio::select! {
                        _ = &mut finish => break,
                        event = receiver.recv_with_revision() => {if let Some(event)=event {self.apply_event(event);}else{break;}}
                    }
                }
            }
            while let Ok(event) = receiver.try_recv_with_revision() {
                self.apply_event(event);
            }
            connection.disconnected().await;
        }
        self.publish(TranslationSessionController::did_stop);
        self.mark_finished();
    }
}

pub fn exchange(request: &str) -> Result<String, &'static str> {
    if request.len() > MAX_REQUEST {
        return Err("native_runtime_invalid_request");
    }
    let value: Value =
        serde_json::from_str(request).map_err(|_| "native_runtime_invalid_request")?;
    let output = match value["operation"]
        .as_str()
        .ok_or("native_runtime_invalid_request")?
    {
        "app_update_endpoint" | "app_update_start" | "app_update_poll" | "app_update_cancel" => {
            crate::app_update::exchange(&value)?
        }
        "merge_snapshots" => crate::snapshot_merge::merge(&value)?,
        "resolve_credentials_draft" => configuration::resolve_credentials_draft(&value)?,
        "policy" => {
            json!({"snapshotPublishIntervalMs":mimi_runtime::core::health::SNAPSHOT_PUBLISH_INTERVAL.as_millis() as u64,
                "sessionFinishTimeoutMs":mimi_runtime::core::health::SESSION_FINISH_TIMEOUT.as_millis() as u64})
        }
        "normalize_text_endpoint" => configuration::normalize_text_endpoint(&value)?,
        "text_endpoints_equivalent" => configuration::text_endpoints_equivalent(&value)?,
        "volcano_decode" => contracts::volcano_decode(&value)?,
        "qwen_streaming_text" => contracts::qwen_streaming_text(&value)?,
        "gemini_streaming_transcripts" => contracts::gemini_streaming_transcripts(&value)?,
        "xai_streaming_transcripts" => contracts::xai_streaming_transcripts(&value)?,
        "preview_content_changed" => json!(contracts::preview_content_changed(&value)?),
        "preview_candidate" => contracts::preview_candidate(&value)?,
        "text_probe_endpoint" => probes::endpoint(&value)?,
        "create_text_probe" | "create_speech_probe" => probes::create(&value)?,
        "text_probe_poll" | "speech_probe_poll" => probes::poll(&value)?,
        "text_probe_cancel" | "speech_probe_cancel" => probes::cancel(&value)?,
        "capabilities" | "normalize" => configuration::capabilities_or_normalize(&value)?,
        "endpoints" => configuration::endpoints(&value)?,
        "history_begin" | "history_finish" | "history_disable" | "history_state"
        | "history_list" | "history_page" | "history_export" | "history_delete" => {
            crate::history::exchange(&value)?
        }
        "create" => {
            let audio_source: AudioSource = value
                .get("audioSource")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|_| "invalid_configuration")?
                .unwrap_or(AudioSource::System);
            let configuration: LiveTranslationConfiguration =
                serde_json::from_value(value["configuration"].clone())
                    .map_err(|_| "invalid_configuration")?;
            let configuration = configuration
                .validated()
                .map_err(|_| "invalid_configuration")?;
            let networks: StageNetworkInput = serde_json::from_value(value["network"].clone())
                .map_err(|_| "native_network_invalid")?;
            let (speech_network, text_network) = networks.resolve()?;
            let mut registry = sessions().lock().map_err(|_| "native_runtime_failed")?;
            if registry.len() >= MAX_SESSIONS {
                return Err("native_runtime_capacity");
            }
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let id = NEXT.fetch_add(1, Ordering::SeqCst);
            let history = value
                .get("historyHandle")
                .map(|handle| {
                    crate::history::attach(handle.as_u64().ok_or("session_history_unavailable")?)
                })
                .transpose()?;
            let native = Arc::new(Session {
                audio_source,
                history: Mutex::new(history),
                history_status: Mutex::new(Default::default()),
                configuration,
                speech_network,
                text_network,
                controller: Mutex::new(Default::default()),
                connection: Mutex::new(None),
                cleanup_owners: Default::default(),
                cleanup_sequence: AtomicU64::new(0),
                task: Mutex::new(None),
                epoch: AtomicU64::new(1),
                version: AtomicU64::new(1),
                started: AtomicBool::new(false),
                stopping: AtomicBool::new(false),
                finished: AtomicBool::new(false),
                signal: Notify::new(),
                error_code: Mutex::new(None),
                budget: Mutex::new(Default::default()),
                recovery_continuity: Mutex::new(Default::default()),
            });
            native.publish(|controller| {
                controller.set_audio_input(match audio_source {
                    AudioSource::System => AudioInput::System,
                    AudioSource::Microphone => AudioInput::Microphone,
                });
                controller.begin_session();
            });
            registry.insert(id, native);
            json!({"handle":id})
        }
        "start" => {
            let native = session(
                value["handle"]
                    .as_u64()
                    .ok_or("native_runtime_invalid_request")?,
            )?;
            native.begin_start()?;
            json!({})
        }
        "poll" => {
            let native = session(
                value["handle"]
                    .as_u64()
                    .ok_or("native_runtime_invalid_request")?,
            )?;
            let since_version = value
                .get("sinceVersion")
                .map(|version| version.as_u64().ok_or("native_runtime_invalid_request"))
                .transpose()?;
            return native.snapshot_json(since_version);
        }
        "finish" => {
            let native = session(
                value["handle"]
                    .as_u64()
                    .ok_or("native_runtime_invalid_request")?,
            )?;
            native.stopping.store(true, Ordering::SeqCst);
            native.signal.notify_one();
            json!({})
        }
        "stop" => {
            let id = value["handle"]
                .as_u64()
                .ok_or("native_runtime_invalid_request")?;
            if let Some(native) = sessions()
                .lock()
                .map_err(|_| "native_runtime_failed")?
                .remove(&id)
            {
                drop(native.begin_stop());
            }
            json!({})
        }
        _ => return Err("native_runtime_invalid_request"),
    };
    serde_json::to_string(&output).map_err(|_| "native_runtime_failed")
}

pub fn pcm(handle: u64, data: Vec<u8>) -> Result<bool, &'static str> {
    if data.len() > MAX_PCM || !data.len().is_multiple_of(2) {
        return Err("native_runtime_invalid_pcm");
    }
    let native = session(handle)?;
    if native.stopping.load(Ordering::SeqCst) {
        return Ok(false);
    }
    Ok(native
        .ingress()
        .is_some_and(|ingress| ingress.try_send(data).is_ok()))
}
