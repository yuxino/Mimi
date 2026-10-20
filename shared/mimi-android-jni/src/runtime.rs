//! Thin native lifecycle adapter. Provider transport, queues, recovery schedule
//! and subtitle state are the same Rust implementations as the desktop shell.
use base64::{engine::general_purpose::STANDARD, Engine as _};
use mimi_runtime::{
    audio::send_pipeline::{AudioIngress, AudioIngressError, AudioSendPipeline},
    clients::{
        provider_events::{provider_event_channel, ProviderEventReceiver, ProviderEventSender},
        provider_network::ProviderNetwork,
        translation_client::TranslationClient,
    },
    core::{
        audio_input::AudioSource,
        configuration::LiveTranslationConfiguration,
        diagnostics::TranslationRecovery,
        models::{
            SessionStatus, SourceLanguage, SubtitleSnapshot, TargetLanguage, TranslationMode,
        },
        network_proxy::ProxyConfig,
        protocols::{
            live_translate::LiveTranslateServerEvent,
            volcano_engine::{VolcanoEngineEndpoint, VolcanoEngineServerEvent},
        },
        provider::{AlibabaRealtimeMode, ProviderKind, ProviderPreferences, TextTranslation},
        recovery::{
            advance_lifecycle_sequence_if_current, provider_error_is_retryable,
            provider_recovery_minimum_delay, MTBudgetContinuity, MTBudgetScope, RecoverySchedule,
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
fn executor() -> &'static Runtime {
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
struct Networks {
    speech_proxy: ProxyConfig,
    text_proxy: ProxyConfig,
    trust_roots: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotMetadata {
    version: u64,
    finished: bool,
    error_code: Option<String>,
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
        let metadata = SnapshotMetadata {
            version,
            finished,
            error_code,
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
        let pipeline = AudioSendPipeline::spawn(
            move |data| {
                let sender = sender.clone();
                let mut ready = ready_rx.clone();
                async move {
                    while !*ready.borrow_and_update() {
                        ready.changed().await.map_err(|_| "transport_error")?;
                    }
                    sender
                        .send_audio(&data)
                        .await
                        .map_err(|_| "transport_error")
                }
            },
            move |_| {
                let _ = failures.send(LiveTranslateServerEvent::Error {
                    code: "transport_error".into(),
                    message: "transport_error".into(),
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
        let _runtime = executor().enter();
        let receiver = self.connection(1)?;
        *task = Some(executor().spawn(self.clone().drive(receiver)));
        Ok(())
    }

    fn begin_stop(self: &Arc<Self>) -> CleanupTicket {
        self.stopping.store(true, Ordering::SeqCst);
        self.epoch.fetch_add(1, Ordering::SeqCst);
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
            },
            None,
        )
    }
    fn fail(&self, code: &str) {
        *self.error_code.lock().unwrap() = Some(code.into());
        self.publish(|controller| controller.did_fail(code));
        self.finished.store(true, Ordering::SeqCst);
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
                recovery = None;
                let health_client = client.clone();
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
                            let code = error.safe_code();
                            let _ = health_events.send(LiveTranslateServerEvent::Error {
                                code: code.into(),
                                message: code.into(),
                            });
                            return;
                        }
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
                                    self.finished.store(true,Ordering::SeqCst);
                                    return;
                                }
                                _ => self.publish(|controller| { event.apply_to(controller, AudioSource::System); }),
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
                self.finished.store(true, Ordering::SeqCst);
                return;
            }
            if !provider_error_is_retryable(code) {
                self.fail(code);
                return;
            }
            let Some(next) = advance_lifecycle_sequence_if_current(&self.epoch, generation) else {
                return;
            };
            if let Some(schedule) = recovery.as_mut() {
                if !schedule.accept_failure(next) {
                    return;
                }
            } else {
                recovery = Some(RecoverySchedule::new(
                    generation,
                    next,
                    provider_recovery_minimum_delay(code),
                ));
            }
            let schedule = recovery.as_mut().unwrap();
            let Some(attempt) = schedule.next_attempt() else {
                self.fail(code);
                return;
            };
            self.publish(TranslationSessionController::begin_connecting);
            tokio::select! { _ = tokio::time::sleep(attempt.delay) => {}, _ = self.signal.notified() => {self.publish(TranslationSessionController::did_stop);self.finished.store(true,Ordering::SeqCst);return;} }
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
                        mimi_runtime::core::health::PROVIDER_FINISH_TIMEOUT,
                        resources.client.finish(),
                    )
                    .await;
                };
                tokio::pin!(finish);
                loop {
                    tokio::select! {
                        _ = &mut finish => break,
                        event = receiver.recv_with_revision() => {if let Some(event)=event {self.publish(|controller|{event.apply_to(controller,AudioSource::System);});}else{break;}}
                    }
                }
            }
            while let Ok(event) = receiver.try_recv_with_revision() {
                self.publish(|controller| {
                    event.apply_to(controller, AudioSource::System);
                });
            }
            connection.disconnected().await;
        }
        self.publish(TranslationSessionController::did_stop);
        self.finished.store(true, Ordering::SeqCst);
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TextProbeInput {
    credentials: mimi_runtime::core::credentials::TextTranslationCredentials,
    source_language: SourceLanguage,
    target_language: TargetLanguage,
    qwen_mt_model: mimi_runtime::core::protocols::qwen_mt::QwenMTModel,
}
fn text_probe_configuration(
    value: &Value,
) -> Result<mimi_runtime::core::configuration::TextTranslationProbeConfiguration, &'static str> {
    let input: TextProbeInput =
        serde_json::from_value(value.clone()).map_err(|_| "invalid_configuration")?;
    Ok(
        mimi_runtime::core::configuration::TextTranslationProbeConfiguration {
            credentials:
                mimi_runtime::core::configuration::TextTranslationProbeCredentials::Independent(
                    input.credentials,
                ),
            source_language: input.source_language,
            target_language: input.target_language,
            qwen_mt_model: input.qwen_mt_model,
            network_proxy: ProxyConfig {
                mode: mimi_runtime::core::network_proxy::ProxyMode::Direct,
                url: None,
            },
        },
    )
}
struct TextProbe {
    result: Mutex<Option<mimi_runtime::clients::connection_diagnostics::ConnectionDiagnostic>>,
    task: Mutex<Option<JoinHandle<()>>>,
}
fn probes() -> &'static Mutex<HashMap<u64, Arc<TextProbe>>> {
    static PROBES: OnceLock<Mutex<HashMap<u64, Arc<TextProbe>>>> = OnceLock::new();
    PROBES.get_or_init(Default::default)
}

// An offline contract view of the shared wire decoder. This never creates a
// provider, session or audio pipeline; protocol rejection is represented by null.
fn volcano_decode(value: &Value) -> Result<Value, &'static str> {
    let hex = value["hex"]
        .as_str()
        .filter(|hex| {
            hex.len() <= VolcanoEngineEndpoint::MAXIMUM_SERVER_FRAME_BYTES * 2
                && hex.len().is_multiple_of(2)
        })
        .ok_or("native_runtime_invalid_request")?;
    let frame = hex
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|bytes| {
            let pair = std::str::from_utf8(bytes).map_err(|_| "native_runtime_invalid_request")?;
            u8::from_str_radix(pair, 16).map_err(|_| "native_runtime_invalid_request")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let event = VolcanoEngineServerEvent::decode(&frame).ok().map(|event| match event {
        VolcanoEngineServerEvent::SourceSubtitleStarted { start_time_ms } =>
            json!({"type":"sourceStart","startTimeMs":start_time_ms}),
        VolcanoEngineServerEvent::SourceSubtitleDraft(text) =>
            json!({"type":"sourceDraft","text":text}),
        VolcanoEngineServerEvent::SourceSubtitleFinal { text, start_time_ms, end_time_ms } =>
            json!({"type":"sourceFinal","text":text,"startTimeMs":start_time_ms,"endTimeMs":end_time_ms}),
        VolcanoEngineServerEvent::TranslationSubtitleStarted { start_time_ms } =>
            json!({"type":"translationStart","startTimeMs":start_time_ms}),
        VolcanoEngineServerEvent::TranslationSubtitleDraft(text) =>
            json!({"type":"translationDraft","text":text}),
        VolcanoEngineServerEvent::TranslationSubtitleFinal { text, start_time_ms, end_time_ms } =>
            json!({"type":"translationFinal","text":text,"startTimeMs":start_time_ms,"endTimeMs":end_time_ms}),
        VolcanoEngineServerEvent::SessionStarted => json!({"type":"sessionStarted"}),
        VolcanoEngineServerEvent::SessionFinished => json!({"type":"sessionFinished"}),
        VolcanoEngineServerEvent::SessionFailed { status_code } =>
            json!({"type":"sessionFailed","statusCode":status_code}),
        VolcanoEngineServerEvent::Ignored { event } => json!({"type":"ignored","event":event}),
    });
    Ok(json!({"event":event}))
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
        "policy" => {
            json!({"snapshotPublishIntervalMs":mimi_runtime::core::health::SNAPSHOT_PUBLISH_INTERVAL.as_millis() as u64,
                "sessionFinishTimeoutMs":mimi_runtime::core::health::SESSION_FINISH_TIMEOUT.as_millis() as u64})
        }
        "volcano_decode" => volcano_decode(&value)?,
        "preview_candidate" => {
            let minimum = value["minimumCharacters"]
                .as_u64()
                .filter(|n| (1..=65536).contains(n))
                .ok_or("native_runtime_invalid_request")?;
            let draft = value["draft"]
                .as_str()
                .filter(|text| mimi_runtime::core::models::subtitle_text_within_limit(text))
                .ok_or("native_runtime_invalid_request")?;
            let opening = value["opening"]
                .as_bool()
                .ok_or("native_runtime_invalid_request")?;
            let mut committer =
                mimi_runtime::core::committer::ASRDraftCommitter::new(minimum as usize);
            committer.update_draft(draft);
            json!({"candidate":committer.preview_latest_draft(opening)})
        }
        "text_probe_endpoint" => {
            let configuration = text_probe_configuration(&value["configuration"])?;
            let endpoint =
                mimi_runtime::clients::connection_diagnostics::text_probe_network_endpoint(
                    &configuration,
                )
                .map_err(|_| "invalid_configuration")?;
            json!({"textEndpoint":endpoint})
        }
        "create_text_probe" => {
            let configuration = text_probe_configuration(&value["configuration"])?;
            let networks: Networks = serde_json::from_value(value["network"].clone())
                .map_err(|_| "native_network_invalid")?;
            let roots = networks
                .trust_roots
                .iter()
                .map(|root| STANDARD.decode(root).map_err(|_| "native_network_invalid"))
                .collect::<Result<Vec<_>, _>>()?;
            let network = ProviderNetwork::resolve(&networks.text_proxy)
                .and_then(|network| network.with_trust_roots(roots))
                .map_err(|_| "native_network_invalid")?;
            let mut registry = probes().lock().map_err(|_| "native_runtime_failed")?;
            if registry.len() >= 2 {
                return Err("native_runtime_capacity");
            }
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let id = NEXT.fetch_add(1, Ordering::SeqCst);
            let probe = Arc::new(TextProbe {
                result: Mutex::new(None),
                task: Mutex::new(None),
            });
            let worker = probe.clone();
            *probe.task.lock().unwrap() = Some(executor().spawn(async move {
                let result =
                    mimi_runtime::clients::connection_diagnostics::check_text_service_with_network(
                        &configuration,
                        network,
                    )
                    .await;
                *worker.result.lock().unwrap() = Some(result);
            }));
            registry.insert(id, probe);
            json!({"handle":id})
        }
        "text_probe_poll" => {
            let id = value["handle"]
                .as_u64()
                .ok_or("native_runtime_invalid_request")?;
            let registry = probes().lock().map_err(|_| "native_runtime_failed")?;
            let probe = registry.get(&id).ok_or("native_runtime_stale_handle")?;
            let result = probe.result.lock().unwrap();
            json!({"finished":result.is_some(),"result":*result})
        }
        "text_probe_cancel" => {
            let id = value["handle"]
                .as_u64()
                .ok_or("native_runtime_invalid_request")?;
            if let Some(probe) = probes()
                .lock()
                .map_err(|_| "native_runtime_failed")?
                .remove(&id)
            {
                if let Some(task) = probe.task.lock().unwrap().take() {
                    task.abort();
                }
            }
            json!({})
        }
        "capabilities" | "normalize" => {
            let provider: ProviderKind = serde_json::from_value(value["provider"].clone())
                .map_err(|_| "invalid_configuration")?;
            let route: TextTranslation = serde_json::from_value(value["route"].clone())
                .map_err(|_| "invalid_configuration")?;
            let target: TargetLanguage = match serde_json::from_value(value["target"].clone()) {
                Ok(language) => language,
                Err(_) if value["operation"] == "normalize" && value["target"].is_string() => {
                    TargetLanguage::SimplifiedChinese
                }
                Err(_) => return Err("invalid_configuration"),
            };
            let mode: AlibabaRealtimeMode = value
                .get("alibabaRealtimeMode")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|_| "invalid_configuration")?
                .unwrap_or_default();
            if mode != AlibabaRealtimeMode::Disabled && provider != ProviderKind::AlibabaCloud {
                return Err("invalid_configuration");
            }
            let capabilities_for_target = |target| {
                if mode != AlibabaRealtimeMode::Disabled {
                    mode.capabilities()
                } else {
                    provider.capabilities_for_route(route, target)
                }
            };
            let capabilities = capabilities_for_target(target);
            if value["operation"] == "normalize" {
                let source: SourceLanguage = match serde_json::from_value(value["source"].clone()) {
                    Ok(language) => language,
                    Err(_) if value["source"].is_string() => SourceLanguage::Automatic,
                    Err(_) => return Err("invalid_configuration"),
                };
                let first = capabilities.normalize(ProviderPreferences {
                    source_language: source,
                    target_language: target,
                    translation_mode: TranslationMode::Turbo,
                });
                let normalized = capabilities_for_target(first.target_language).normalize(first);
                json!({"sourceLanguage":normalized.source_language,"targetLanguage":normalized.target_language})
            } else {
                let pairs: serde_json::Map<String, Value> = capabilities
                    .source_languages
                    .iter()
                    .map(|source| {
                        (
                            source.raw_value().into(),
                            json!(capabilities.clone().for_source(*source).target_languages),
                        )
                    })
                    .collect();
                json!({"sourceLanguages":capabilities.source_languages,"targetLanguages":capabilities.target_languages,"sampleRateHz":capabilities.input_sample_rate_hz,"targetPairs":pairs})
            }
        }
        "endpoints" => {
            let configuration: LiveTranslationConfiguration =
                serde_json::from_value(value["configuration"].clone())
                    .map_err(|_| "invalid_configuration")?;
            let (speech, text) = configuration
                .network_endpoints()
                .map_err(|_| "invalid_configuration")?;
            json!({"speechEndpoint":speech,"textEndpoint":text,"modelNames":configuration.model_names()})
        }
        "create" => {
            let configuration: LiveTranslationConfiguration =
                serde_json::from_value(value["configuration"].clone())
                    .map_err(|_| "invalid_configuration")?;
            let configuration = configuration
                .validated()
                .map_err(|_| "invalid_configuration")?;
            let networks: Networks = serde_json::from_value(value["network"].clone())
                .map_err(|_| "native_network_invalid")?;
            let roots: Vec<Vec<u8>> = networks
                .trust_roots
                .iter()
                .map(|root| STANDARD.decode(root).map_err(|_| "native_network_invalid"))
                .collect::<Result<_, _>>()?;
            let speech_network = ProviderNetwork::resolve(&networks.speech_proxy)
                .and_then(|network| network.with_trust_roots(roots.clone()))
                .map_err(|_| "native_network_invalid")?;
            let text_network = ProviderNetwork::resolve(&networks.text_proxy)
                .and_then(|network| network.with_trust_roots(roots))
                .map_err(|_| "native_network_invalid")?;
            let mut registry = sessions().lock().map_err(|_| "native_runtime_failed")?;
            if registry.len() >= MAX_SESSIONS {
                return Err("native_runtime_capacity");
            }
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let id = NEXT.fetch_add(1, Ordering::SeqCst);
            let native = Arc::new(Session {
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
            });
            native.publish(TranslationSessionController::begin_session);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    fn fixture(configuration: LiveTranslationConfiguration) -> Arc<Session> {
        let direct = ProxyConfig {
            mode: mimi_runtime::core::network_proxy::ProxyMode::Direct,
            url: None,
        };
        Arc::new(Session {
            configuration,
            speech_network: ProviderNetwork::resolve(&direct).unwrap(),
            text_network: ProviderNetwork::resolve(&direct).unwrap(),
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
        })
    }
    fn configuration() -> LiveTranslationConfiguration {
        serde_json::from_value(json!({"provider":"alibabaCloud","qwenMtModel":"lite","credentials":{"kind":"apiKey","apiKey":"synthetic-key"},"textCredentials":null,"sourceLanguage":"en","targetLanguage":"zh","translationMode":"turbo","networkProxy":{"mode":"direct"},"textNetworkProxy":{"mode":"direct"}})).unwrap()
    }

    struct PollSession {
        handle: u64,
        native: Arc<Session>,
    }
    impl PollSession {
        fn new() -> Self {
            static HANDLE: AtomicU64 = AtomicU64::new(u64::MAX - 100);
            let handle = HANDLE.fetch_sub(1, Ordering::SeqCst);
            let native = fixture(configuration());
            sessions().lock().unwrap().insert(handle, native.clone());
            Self { handle, native }
        }
        fn poll(&self, since: Option<Value>) -> Result<Value, &'static str> {
            let mut request = json!({"operation":"poll","handle":self.handle});
            if let Some(since) = since {
                request["sinceVersion"] = since;
            }
            exchange(&request.to_string()).map(|response| serde_json::from_str(&response).unwrap())
        }
    }
    impl Drop for PollSession {
        fn drop(&mut self) {
            sessions().lock().unwrap().remove(&self.handle);
        }
    }

    // Compatibility oracle for the complete pre-conditional-poll wire shape.
    fn legacy_full_response(native: &Session) -> Value {
        let controller = native.controller.lock().unwrap();
        let state = &controller.state;
        let connection_status = match state.status {
            SessionStatus::Idle => "idle",
            SessionStatus::Connecting => "connecting",
            SessionStatus::Listening => "listening",
            SessionStatus::Stopping => "stopping",
            SessionStatus::Error(_) => "error",
        };
        json!({"version":native.version.load(Ordering::SeqCst),"connectionStatus":connection_status,"snapshot":state.subtitles,"atomicPreview":state.atomic_preview,"detectedLanguage":state.detected_language.as_ref().map(|language|language.code.as_str()),"isTranslationPending":state.is_translation_pending,"isTranslationPreviewPending":state.is_translation_preview_pending,"isTranslationTimedOut":state.is_translation_timed_out,"translationRecovery":state.translation_recovery,"finished":native.finished.load(Ordering::SeqCst),"errorCode":*native.error_code.lock().unwrap()})
    }

    #[test]
    fn conditional_poll_preserves_legacy_and_first_full_response() {
        let fixture = PollSession::new();
        let expected = legacy_full_response(&fixture.native);
        assert_eq!(fixture.poll(None).unwrap(), expected);
        assert_eq!(fixture.poll(Some(json!(0))).unwrap(), expected);
        assert_eq!(fixture.poll(Some(json!(u64::MAX))).unwrap(), expected);
        assert_eq!(
            fixture.poll(Some(json!(1))).unwrap(),
            json!({"unchanged":true,"version":1,"finished":false,"errorCode":null})
        );
    }

    #[test]
    fn conditional_poll_refreshes_terminal_fields_without_a_controller_revision() {
        let fixture = PollSession::new();
        let first = fixture.poll(None).unwrap();
        let version = first["version"].clone();
        assert_eq!(
            fixture.poll(Some(version.clone())).unwrap()["finished"],
            false
        );
        *fixture.native.error_code.lock().unwrap() = Some("transport_error".into());
        let pending = fixture.poll(Some(version.clone())).unwrap();
        assert_eq!(pending["errorCode"], "transport_error");
        assert_eq!(pending["finished"], false);
        fixture.native.finished.store(true, Ordering::SeqCst);
        assert_eq!(
            fixture.poll(Some(version.clone())).unwrap(),
            json!({"unchanged":true,"version":version,"finished":true,"errorCode":"transport_error"})
        );
    }

    #[test]
    fn conditional_poll_returns_complete_current_state_after_a_change() {
        let fixture = PollSession::new();
        let first = fixture.poll(None).unwrap();
        fixture.native.publish(|controller| {
            controller.did_connect();
            controller.handle_from(
                AudioSource::System,
                LiveTranslateServerEvent::SubtitleConfirmedPair {
                    source_utterance_id: Some(1),
                    utterance_id: 1,
                    source: "Synthetic conditional source".into(),
                    translation: "Synthetic conditional translation".into(),
                    language: Some("en".into()),
                },
            );
            controller.state.atomic_preview = Some(true);
            controller.state.is_translation_pending = true;
            controller.state.is_translation_preview_pending = true;
            controller.state.is_translation_timed_out = true;
            controller.state.translation_recovery = Some(TranslationRecovery {
                reason: mimi_runtime::core::diagnostics::TranslationRecoveryReason::RateLimited,
                retry_after_ms: 4000,
                retry_scheduled: true,
            });
        });
        let changed = fixture.poll(Some(first["version"].clone())).unwrap();
        assert_eq!(changed, legacy_full_response(&fixture.native));
        assert!(changed.get("unchanged").is_none());
        assert_eq!(changed["snapshot"]["history"].as_array().unwrap().len(), 1);
        assert_eq!(
            fixture.poll(Some(changed["version"].clone())).unwrap()["unchanged"],
            true
        );
    }

    #[test]
    fn conditional_poll_observes_tick_changes_without_republishing_unchanged_ticks() {
        let fixture = PollSession::new();
        fixture.native.publish(|controller| {
            controller.handle_from(
                AudioSource::System,
                LiveTranslateServerEvent::TranslationStarted,
            );
        });
        let before = fixture.poll(None).unwrap();
        assert_eq!(before["isTranslationPending"], true);
        let now = std::time::Instant::now();
        fixture.native.tick(now + Duration::from_secs(7));
        let expired = fixture.poll(Some(before["version"].clone())).unwrap();
        assert_eq!(expired["isTranslationPending"], false);
        assert_eq!(expired["isTranslationTimedOut"], true);
        assert_eq!(
            expired["version"].as_u64().unwrap(),
            before["version"].as_u64().unwrap() + 1
        );
        fixture.native.tick(now + Duration::from_secs(8));
        assert_eq!(
            fixture.poll(Some(expired["version"].clone())).unwrap()["unchanged"],
            true
        );
    }

    #[test]
    fn conditional_poll_rejects_invalid_versions_and_stale_handles() {
        let fixture = PollSession::new();
        for since in [json!(-1), Value::Null, json!(true), json!("1"), json!(1.5)] {
            assert_eq!(
                fixture.poll(Some(since)),
                Err("native_runtime_invalid_request")
            );
        }
        sessions().lock().unwrap().remove(&fixture.handle);
        assert_eq!(
            fixture.poll(Some(json!(1))),
            Err("native_runtime_stale_handle")
        );
    }

    #[test]
    fn published_controller_state_and_version_stay_together_under_contention() {
        let native = fixture(configuration());
        native
            .controller
            .lock()
            .unwrap()
            .state
            .subtitles
            .source
            .text = "1".into();
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let writers: Vec<_> = (0..2)
            .map(|_| {
                let native = native.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    for _ in 0..10_000 {
                        native.publish(|controller| {
                            let next = native.version.load(Ordering::SeqCst) + 1;
                            controller.state.subtitles.source.text = next.to_string();
                            std::thread::yield_now();
                        });
                    }
                })
            })
            .collect();
        barrier.wait();
        for _ in 0..2_000 {
            let snapshot = native.snapshot();
            assert_eq!(
                snapshot["snapshot"]["source"]["text"]
                    .as_str()
                    .unwrap()
                    .parse::<u64>()
                    .unwrap(),
                snapshot["version"].as_u64().unwrap(),
                "a snapshot must carry the revision of its controller state"
            );
        }
        for writer in writers {
            writer.join().unwrap();
        }
        let final_snapshot = native.snapshot();
        assert_eq!(final_snapshot["snapshot"]["source"]["text"], "20001");
        assert_eq!(final_snapshot["version"], 20_001);
    }

    #[tokio::test]
    async fn pcm_backpressure_reports_one_retryable_error_for_its_connection() {
        use mimi_runtime::audio::send_pipeline::AudioIngressError;
        let native = fixture(configuration());
        let mut receiver = native.connection(1).unwrap();
        let ingress = native.ingress().unwrap();
        let mut backpressure = 0;
        // The actual pipeline waits for provider readiness. Twenty-two chunks
        // exceed the bounded queue even if its worker has taken one already.
        for _ in 0..22 {
            if ingress.try_send(vec![0, 0]) == Err(AudioIngressError::Backpressure) {
                backpressure += 1;
            }
        }
        assert_eq!(backpressure, 1);
        let event = receiver
            .try_recv_with_revision()
            .expect("a closed PCM ingress must notify the drive instead of silently losing audio");
        let LiveTranslateServerEvent::Error { code, .. } = event.event else {
            panic!("PCM backpressure must use the provider recovery lane");
        };
        assert_eq!(code, "transport_error");
        assert!(provider_error_is_retryable(&code));
        for _ in 0..10 {
            assert_eq!(ingress.try_send(vec![0, 0]), Err(AudioIngressError::Closed));
        }
        assert!(receiver.try_recv_with_revision().is_err());
        native.begin_stop().await;
    }

    #[tokio::test]
    async fn pcm_backpressure_from_an_old_ingress_cannot_fail_a_new_connection() {
        use mimi_runtime::audio::send_pipeline::AudioIngressError;
        let native = fixture(configuration());
        let mut old_receiver = native.connection(1).unwrap();
        let old_ingress = native.ingress().unwrap();
        // Keep the old resources alive across replacement to model a native
        // submission that acquired its endpoint before the generation changed.
        let old_connection = native.take_connection_owned().unwrap();
        let mut current_receiver = native.connection(2).unwrap();
        for _ in 0..22 {
            let _ = old_ingress.try_send(vec![0, 0]);
        }
        assert!(matches!(
            old_receiver.try_recv_with_revision().unwrap().event,
            LiveTranslateServerEvent::Error { .. }
        ));
        assert!(current_receiver.try_recv_with_revision().is_err());
        assert_eq!(native.ingress().unwrap().try_send(vec![0, 0]), Ok(()));
        for _ in 0..10 {
            assert_eq!(
                old_ingress.try_send(vec![0, 0]),
                Err(AudioIngressError::Closed)
            );
        }
        assert!(current_receiver.try_recv_with_revision().is_err());
        drop(old_connection);
        native.begin_stop().await;
    }

    #[tokio::test]
    async fn closed_pcm_ingress_after_hard_stop_does_not_request_recovery() {
        use mimi_runtime::audio::send_pipeline::AudioIngressError;
        let native = fixture(configuration());
        let mut receiver = native.connection(1).unwrap();
        let ingress = native.ingress().unwrap();
        native.begin_stop().await;
        let epoch = native.epoch.load(Ordering::SeqCst);
        assert_eq!(ingress.try_send(vec![0, 0]), Err(AudioIngressError::Closed));
        assert!(receiver.try_recv_with_revision().is_err());
        assert!(native.connection.lock().unwrap().is_none());
        assert_eq!(native.epoch.load(Ordering::SeqCst), epoch);
    }

    #[tokio::test]
    async fn dropping_an_unpolled_connection_lease_stops_its_pipeline_and_finishes_cleanup() {
        let native = fixture(configuration());
        let _events = native.connection(1).unwrap();
        let ingress = native.ingress().unwrap();
        let lease = native.take_connection_owned().unwrap();
        assert!(native.connection.lock().unwrap().is_none());
        let owners = native.cleanup_owners.wait_registered();
        tokio::pin!(owners);
        assert!(tokio::time::timeout(Duration::from_millis(10), &mut owners)
            .await
            .is_err());

        drop(lease);

        tokio::time::timeout(Duration::from_secs(1), owners)
            .await
            .expect("a dropped lease must release its registered disconnect owner");
        assert!(ingress.try_send(vec![0, 0]).is_err());
    }

    #[tokio::test]
    async fn same_generation_connections_keep_independent_cleanup_owners() {
        let native = fixture(configuration());
        let _first_events = native.connection(7).unwrap();
        let first = native.take_connection_owned().unwrap();
        let first_key = first.connection.cleanup_id;
        // A rebuild or another lane must not reuse the first client's factory,
        // even when a caller supplies the same lifecycle generation.
        let _second_events = native.connection(7).unwrap();
        let second = native.take_connection_owned().unwrap();
        assert_ne!(first_key, second.connection.cleanup_id);
        let owners = native.cleanup_owners.wait_registered();
        tokio::pin!(owners);
        drop(first);
        assert!(tokio::time::timeout(Duration::from_millis(10), &mut owners)
            .await
            .is_err());

        drop(second);

        tokio::time::timeout(Duration::from_secs(1), owners)
            .await
            .expect("each actual client must release its own owner");
    }

    #[tokio::test]
    async fn hard_stop_joins_a_cancelled_drive_with_an_already_taken_connection() {
        let native = fixture(configuration());
        let _events = native.connection(1).unwrap();
        let ingress = native.ingress().unwrap();
        let (taken, was_taken) = oneshot::channel();
        let task = tokio::spawn({
            let native = native.clone();
            async move {
                let lease = native.take_connection_owned().unwrap();
                taken.send(()).unwrap();
                std::future::pending::<()>().await;
                drop(lease);
            }
        });
        *native.task.lock().unwrap() = Some(task);
        was_taken.await.unwrap();

        // Leave the stop ticket unpolled: aborting the drive must still drop
        // its lease and start final disconnect before another waiter joins.
        let stop = native.begin_stop();
        tokio::time::timeout(
            Duration::from_secs(1),
            native.cleanup_owners.wait_registered(),
        )
        .await
        .expect("drive cancellation must not cancel final disconnect");
        tokio::time::timeout(Duration::from_secs(1), stop)
            .await
            .expect("the stop completion must join the cancelled drive");
        assert!(native.connection.lock().unwrap().is_none());
        assert!(ingress.try_send(vec![0, 0]).is_err());
    }

    #[tokio::test]
    async fn hard_stop_waits_for_late_installation_before_retiring_the_slot() {
        struct InstallOnDrop {
            native: Arc<Session>,
            entered: Option<oneshot::Sender<()>>,
            release: std::sync::mpsc::Receiver<()>,
            installed: Option<oneshot::Sender<ConnectionIngress>>,
        }
        impl Drop for InstallOnDrop {
            fn drop(&mut self) {
                let _ = self.entered.take().unwrap().send(());
                self.release.recv_timeout(Duration::from_secs(2)).unwrap();
                let _events = self.native.connection(1).unwrap();
                let _ = self
                    .installed
                    .take()
                    .unwrap()
                    .send(self.native.ingress().unwrap());
            }
        }
        let native = fixture(configuration());
        let (started, was_started) = oneshot::channel();
        let (entered, was_entered) = oneshot::channel();
        let (release, released) = std::sync::mpsc::channel();
        let (installed, was_installed) = oneshot::channel();
        let task = executor().spawn({
            let native = native.clone();
            async move {
                let installation = InstallOnDrop {
                    native,
                    entered: Some(entered),
                    release: released,
                    installed: Some(installed),
                };
                started.send(()).unwrap();
                std::future::pending::<()>().await;
                drop(installation);
            }
        });
        *native.task.lock().unwrap() = Some(task);
        was_started.await.unwrap();
        let stop = native.begin_stop();
        was_entered.await.unwrap();
        tokio::pin!(stop);
        let still_waiting = tokio::time::timeout(Duration::from_millis(20), &mut stop)
            .await
            .is_err();
        // Always release the worker before asserting, including a regression
        // where Stop incorrectly finishes while installation is still gated.
        release.send(()).unwrap();
        let ingress = was_installed.await.unwrap();
        assert!(still_waiting, "Stop must join installation before sweeping");
        tokio::time::timeout(Duration::from_secs(1), stop)
            .await
            .expect("the installed client must be retired after joining");
        assert!(native.connection.lock().unwrap().is_none());
        assert!(ingress.try_send(vec![0, 0]).is_err());
    }

    #[tokio::test]
    async fn retiring_a_connection_preserves_request_budget_for_the_retry() {
        use mimi_runtime::core::preview_pacing::PreviewRequestPacer;
        let native = fixture(configuration());
        let _events = native.connection(7).unwrap();
        let client = native
            .connection
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .client
            .clone();
        let now = std::time::Instant::now();
        let mut pacer = PreviewRequestPacer::default();
        pacer.record_start(now);
        pacer.suppress_after_rate_limit(now);
        pacer.set_shared_cooldown(Some(now + Duration::from_secs(4)));
        let budget = pacer.export_budget();
        client.restore_mt_request_budget(Some(budget)).await;
        drop(client);

        native.retire(7).await;

        let _retry_events = native.connection(8).unwrap();
        let retained = native
            .connection
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .budget
            .expect("normal retirement must preserve request accounting");
        assert_eq!(
            retained.shared_cooldown_until(),
            budget.shared_cooldown_until()
        );
        assert!(retained.is_preview_suppressed(now));
        native.begin_stop().await;
    }

    #[tokio::test]
    async fn a_retained_stopped_session_rejects_start_before_installing_resources() {
        // A failed regression must still be confined to loopback if it
        // mistakenly starts the drive instead of rejecting this owner.
        let mut configuration = configuration();
        configuration.provider = ProviderKind::CustomOpenAIASR;
        configuration.target_language = TargetLanguage::Original;
        configuration.credentials =
            mimi_runtime::core::credentials::ProviderCredentials::CustomSpeech {
                endpoint: "ws://127.0.0.1:9/v1/realtime".into(),
                model: "synthetic-stop-owner".into(),
                api_key: "synthetic-key".into(),
            };
        let native = fixture(configuration);
        native.begin_stop().await;

        assert_eq!(native.begin_start(), Err("native_runtime_stale_handle"));
        assert_eq!(native.cleanup_sequence.load(Ordering::SeqCst), 0);
        assert!(native.connection.lock().unwrap().is_none());
        assert!(native.task.lock().unwrap().is_none());
    }

    #[test]
    fn native_snapshot_preserves_shared_connection_and_translation_recovery() {
        use mimi_runtime::core::diagnostics::{TranslationRecovery, TranslationRecoveryReason};
        let native = fixture(configuration());
        native.publish(TranslationSessionController::begin_session);
        assert_eq!(native.snapshot()["connectionStatus"], "connecting");
        native.publish(TranslationSessionController::did_connect);
        native.publish(|controller| {
            controller.handle_from(
                AudioSource::System,
                LiveTranslateServerEvent::TranslationDeferred(TranslationRecovery {
                    reason: TranslationRecoveryReason::RateLimited,
                    retry_after_ms: 8000,
                    retry_scheduled: false,
                }),
            );
        });
        let snapshot = native.snapshot();
        assert_eq!(snapshot["connectionStatus"], "listening");
        assert_eq!(snapshot["translationRecovery"]["reason"], "rateLimited");
        assert_eq!(snapshot["translationRecovery"]["retryAfterMs"], 8000);
        assert_eq!(snapshot["translationRecovery"]["retryScheduled"], false);
        native.publish(|controller| {
            controller.handle_from(
                AudioSource::System,
                LiveTranslateServerEvent::TranslationStarted,
            )
        });
        assert!(native.snapshot()["translationRecovery"].is_null());
        native.publish(TranslationSessionController::begin_stopping);
        assert_eq!(native.snapshot()["connectionStatus"], "stopping");
        native.publish(TranslationSessionController::did_stop);
        assert_eq!(native.snapshot()["connectionStatus"], "idle");
    }

    #[test]
    fn native_snapshot_uses_the_desktop_controller_for_identified_drafts() {
        let native = fixture(configuration());
        let pair = |id: &str, source: &str, translation: &str| {
            LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                utterance_id: id.into(),
                source: source.into(),
                translation: translation.into(),
                language: Some("en".into()),
            }
        };
        native.publish(TranslationSessionController::begin_session);
        native.publish(TranslationSessionController::did_connect);
        native.publish(|controller| {
            controller.handle_from(AudioSource::System, pair("A", "Complete A", "完整 A"))
        });
        native.publish(|controller| {
            controller.handle_from(
                AudioSource::System,
                LiveTranslateServerEvent::UtteranceText {
                    utterance_id: "B".into(),
                    role: mimi_runtime::core::models::UtteranceRole::Source,
                    text: "Draft B".into(),
                    is_final: false,
                    language: Some("en".into()),
                },
            )
        });
        let snapshot =
            serde_json::to_value(&native.controller.lock().unwrap().state.subtitles).unwrap();
        assert_eq!(snapshot["displayPair"]["source"], "Complete A");
        assert_eq!(snapshot["realtimePreview"]["source"]["text"], "Draft B");
        assert_eq!(snapshot["realtimePreview"]["translation"]["text"], "");
        native.publish(|controller| {
            controller.handle_from(
                AudioSource::System,
                LiveTranslateServerEvent::UtteranceText {
                    utterance_id: "B".into(),
                    role: mimi_runtime::core::models::UtteranceRole::Translation,
                    text: "草稿 B".into(),
                    is_final: false,
                    language: None,
                },
            )
        });
        let snapshot =
            serde_json::to_value(&native.controller.lock().unwrap().state.subtitles).unwrap();
        assert_eq!(snapshot["realtimePreview"]["translation"]["text"], "草稿 B");
        native.publish(|controller| {
            controller.handle_from(AudioSource::System, pair("B", "Complete B", "完整 B"))
        });
        let snapshot =
            serde_json::to_value(&native.controller.lock().unwrap().state.subtitles).unwrap();
        assert_eq!(snapshot["displayPair"]["source"], "Complete B");
        assert!(snapshot["realtimePreview"].is_null());
    }
    #[test]
    fn volcano_timing_contracts_cross_native_exchange_without_network() {
        let contract: Value =
            serde_json::from_str(include_str!("../../translation-contracts.json")).unwrap();
        let cases = contract["volcanoSubtitleTimingEvents"].as_array().unwrap();
        assert!(!cases.is_empty());
        for case in cases {
            let request = json!({"operation":"volcano_decode","hex":case["hex"]});
            let response: Value =
                serde_json::from_str(&exchange(&request.to_string()).unwrap()).unwrap();
            assert_eq!(response["event"], case["expected"], "{}", case["id"]);
        }
    }

    #[test]
    fn invalid_volcano_decode_requests_have_content_free_errors() {
        for hex in [
            Value::Null,
            json!(12),
            json!("0"),
            json!("synthetic-private-marker"),
            json!("日"),
            json!("00".repeat(VolcanoEngineEndpoint::MAXIMUM_SERVER_FRAME_BYTES + 1)),
        ] {
            assert_eq!(
                exchange(&json!({"operation":"volcano_decode","hex":hex}).to_string()).unwrap_err(),
                "native_runtime_invalid_request"
            );
        }
    }

    #[test]
    fn malformed_native_requests_and_pcm_have_content_free_errors() {
        assert_eq!(
            exchange("invalid synthetic secret").unwrap_err(),
            "native_runtime_invalid_request"
        );
        assert_eq!(
            exchange(r#"{"operation":"create","configuration":{"secret":"synthetic"}}"#)
                .unwrap_err(),
            "invalid_configuration"
        );
        assert_eq!(
            pcm(u64::MAX, vec![1]).unwrap_err(),
            "native_runtime_invalid_pcm"
        );
        assert_eq!(
            pcm(u64::MAX, vec![1, 2]).unwrap_err(),
            "native_runtime_stale_handle"
        );
    }
    #[test]
    fn alibaba_mode_capabilities_and_normalization_use_shared_catalog() {
        let request = json!({"operation":"capabilities", "provider":"alibabaCloud",
            "route":"deepL", "target":"fr", "alibabaRealtimeMode":"liveTranslate"});
        let response: Value =
            serde_json::from_str(&exchange(&request.to_string()).unwrap()).unwrap();
        let shared = AlibabaRealtimeMode::LiveTranslate.capabilities();
        assert_eq!(response["sourceLanguages"], json!(shared.source_languages));
        assert_eq!(response["targetLanguages"], json!(shared.target_languages));
        assert_eq!(response["sampleRateHz"], json!(shared.input_sample_rate_hz));
        let mut normalize = request.clone();
        normalize["operation"] = json!("normalize");
        normalize["source"] = json!("en");
        let normalized: Value =
            serde_json::from_str(&exchange(&normalize.to_string()).unwrap()).unwrap();
        assert_eq!(
            normalized,
            json!({"sourceLanguage":"auto", "targetLanguage":"fr"})
        );
        let mut legacy = request.clone();
        legacy
            .as_object_mut()
            .unwrap()
            .remove("alibabaRealtimeMode");
        let response: Value =
            serde_json::from_str(&exchange(&legacy.to_string()).unwrap()).unwrap();
        assert!(response["sourceLanguages"].as_array().unwrap().len() > 1);
        for invalid in ["unknown", "asrMt"] {
            let mut request = request.clone();
            request["alibabaRealtimeMode"] = json!(invalid);
            assert_eq!(
                exchange(&request.to_string()).unwrap_err(),
                "invalid_configuration"
            );
        }
        let mut wrong_provider = request;
        wrong_provider["provider"] = json!("tencentCloud");
        assert_eq!(
            exchange(&wrong_provider.to_string()).unwrap_err(),
            "invalid_configuration"
        );
    }

    #[tokio::test]
    async fn native_finish_drains_current_reliable_pairs_but_not_stale_content_or_drafts() {
        // Instantiate the real shared factory/pipeline without connecting it.
        // Events are synthetic and no provider endpoint is contacted.
        let native = fixture(configuration());
        native.publish(TranslationSessionController::begin_session);
        let mut receiver = native.connection(1).unwrap();
        native.publish(TranslationSessionController::did_connect);
        let events = native
            .connection
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .events
            .clone();
        let pair = |source: &str| LiveTranslateServerEvent::SubtitleFinalPair {
            source: source.into(),
            translation: source.into(),
            language: Some("en".into()),
        };
        events.send(pair("Synthetic stale pair")).unwrap();
        events.advance_content_revision();
        events
            .send(LiveTranslateServerEvent::SourceDraft {
                text: "Synthetic teardown draft".into(),
                language: Some("en".into()),
            })
            .unwrap();
        events.send(pair("Synthetic reliable tail")).unwrap();
        native.stopping.store(true, Ordering::SeqCst);
        tokio::time::timeout(
            Duration::from_secs(10),
            native.finish_connection(&mut receiver),
        )
        .await
        .unwrap();
        let state =
            serde_json::to_value(&native.controller.lock().unwrap().state.subtitles).unwrap();
        assert_eq!(state["displayPair"]["source"], "Synthetic reliable tail");
        assert_eq!(state["history"].as_array().unwrap().len(), 1);
        assert!(!state.to_string().contains("Synthetic teardown draft"));
        assert!(native.finished.load(Ordering::SeqCst));
        assert!(native.ingress().is_none());
    }
    #[tokio::test]
    async fn setup_pcm_backpressure_recovers_the_same_handle_and_accepts_fresh_audio() {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut configuration = configuration();
        configuration.provider = ProviderKind::CustomOpenAIASR;
        configuration.target_language = TargetLanguage::Original;
        configuration.credentials =
            mimi_runtime::core::credentials::ProviderCredentials::CustomSpeech {
                endpoint: format!("ws://{}/v1/realtime", listener.local_addr().unwrap()),
                model: "synthetic-backpressure-recovery".into(),
                api_key: "synthetic-key".into(),
            };
        let native = fixture(configuration.validated().unwrap());
        const HANDLE: u64 = u64::MAX - 2;
        sessions().lock().unwrap().insert(HANDLE, native.clone());
        struct StopSession;
        impl Drop for StopSession {
            fn drop(&mut self) {
                let native = sessions().lock().unwrap().remove(&HANDLE);
                if let Some(native) = native {
                    drop(native.begin_stop());
                }
            }
        }
        let _cleanup = StopSession;
        let (first_setup, saw_first_setup) = oneshot::channel();
        let (release_setup, setup_released) = oneshot::channel();
        let (second_setup, saw_second_setup) = oneshot::channel();
        let (fresh_audio, saw_fresh_audio) = oneshot::channel();
        let server = tokio::spawn(async move {
            let mut first_setup = Some(first_setup);
            let mut setup_released = Some(setup_released);
            let mut second_setup = Some(second_setup);
            let mut fresh_audio = Some(fresh_audio);
            for attempt in 0..2 {
                let (stream, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept())
                    .await
                    .unwrap()
                    .unwrap();
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                let update = socket.next().await.unwrap().unwrap();
                let mut update: Value = serde_json::from_str(update.to_text().unwrap()).unwrap();
                assert_eq!(update["type"], "session.update");
                if attempt == 0 {
                    first_setup.take().unwrap().send(()).unwrap();
                    setup_released.take().unwrap().await.unwrap();
                }
                update["type"] = json!("session.updated");
                socket
                    .send(Message::Text(update.to_string().into()))
                    .await
                    .unwrap();
                if attempt == 1 {
                    second_setup.take().unwrap().send(()).unwrap();
                }
                loop {
                    let message = tokio::time::timeout(Duration::from_secs(3), socket.next())
                        .await
                        .unwrap();
                    match message {
                        Some(Ok(Message::Text(text))) => {
                            let message: Value = serde_json::from_str(&text).unwrap();
                            assert_eq!(message["type"], "input_audio_buffer.append");
                            if attempt == 1 {
                                let audio =
                                    STANDARD.decode(message["audio"].as_str().unwrap()).unwrap();
                                fresh_audio.take().unwrap().send(audio).unwrap();
                            }
                        }
                        Some(Ok(Message::Close(_))) | None => break,
                        message => panic!("unexpected loopback message: {message:?}"),
                    }
                }
            }
            assert!(
                tokio::time::timeout(Duration::from_millis(100), listener.accept())
                    .await
                    .is_err()
            );
        });
        exchange(&json!({"operation":"start","handle":HANDLE}).to_string()).unwrap();
        tokio::time::timeout(Duration::from_secs(3), saw_first_setup)
            .await
            .unwrap()
            .unwrap();
        let old_ingress = native.ingress().unwrap();
        let accepted = (0..22).filter(|_| pcm(HANDLE, vec![0, 0]).unwrap()).count();
        assert!(accepted <= 21);
        release_setup.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(3), saw_second_setup)
            .await
            .expect("the same native handle must reconnect after its PCM queue fails closed")
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while native.snapshot()["connectionStatus"] != "listening" {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        for _ in 0..10 {
            assert_eq!(
                old_ingress.try_send(vec![0, 0]),
                Err(AudioIngressError::Closed)
            );
        }
        // Custom ASR deliberately suppresses silence. One voiced 20 ms frame
        // proves that the replacement transport receives fresh PCM.
        let fresh = [0_u8, 16].repeat(480);
        assert!(pcm(HANDLE, fresh.clone()).unwrap());
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(3), saw_fresh_audio)
                .await
                .unwrap()
                .unwrap(),
            fresh
        );
        assert_eq!(native.epoch.load(Ordering::SeqCst), 2);
        exchange(&json!({"operation":"stop","handle":HANDLE}).to_string()).unwrap();
        assert_eq!(pcm(HANDLE, vec![0, 0]), Err("native_runtime_stale_handle"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn native_pcm_reaches_the_real_shared_transport_and_stop_rejects_the_handle() {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut configuration = configuration();
        configuration.provider = ProviderKind::CustomOpenAIASR;
        configuration.target_language = TargetLanguage::Original;
        configuration.credentials =
            mimi_runtime::core::credentials::ProviderCredentials::CustomSpeech {
                endpoint: format!("ws://{}/v1/realtime", listener.local_addr().unwrap()),
                model: "mock-transcribe".into(),
                api_key: "synthetic-key".into(),
            };
        let native = fixture(configuration.validated().unwrap());
        const HANDLE: u64 = u64::MAX - 1;
        sessions().lock().unwrap().insert(HANDLE, native.clone());
        let (audio_tx, audio_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let message = socket.next().await.unwrap().unwrap();
            let mut update: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
            assert_eq!(update["type"], "session.update");
            update["type"] = json!("session.updated");
            socket
                .send(Message::Text(update.to_string().into()))
                .await
                .unwrap();
            while let Some(Ok(message)) = socket.next().await {
                if !message.is_text() {
                    continue;
                }
                let value: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
                if value["type"] == "input_audio_buffer.append" {
                    let data = STANDARD.decode(value["audio"].as_str().unwrap()).unwrap();
                    audio_tx.send(data).unwrap();
                    break;
                }
            }
        });
        exchange(&json!({"operation":"start","handle":HANDLE}).to_string()).unwrap();
        assert!(pcm(HANDLE, [0_u8, 16].repeat(2400)).unwrap());
        let received = tokio::time::timeout(Duration::from_secs(3), audio_rx)
            .await
            .unwrap()
            .unwrap();
        assert!(!received.is_empty());
        let (samples, remainder) = received.as_chunks::<2>();
        assert!(remainder.is_empty(), "PCM ended with a partial sample");
        assert!(samples.iter().all(|sample| *sample == [0, 16]));
        exchange(&json!({"operation":"stop","handle":HANDLE}).to_string()).unwrap();
        assert_eq!(
            pcm(HANDLE, vec![0, 0]).unwrap_err(),
            "native_runtime_stale_handle"
        );
        assert_eq!(
            exchange(&json!({"operation":"poll","handle":HANDLE}).to_string()).unwrap_err(),
            "native_runtime_stale_handle"
        );
        server.await.unwrap();
    }
}
