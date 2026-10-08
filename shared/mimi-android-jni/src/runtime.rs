//! Thin native lifecycle adapter. Provider transport, queues, recovery schedule
//! and subtitle state are the same Rust implementations as the desktop shell.
use base64::{engine::general_purpose::STANDARD, Engine as _};
use mimi_runtime::{
    audio::send_pipeline::{AudioIngress, AudioSendPipeline},
    clients::{
        provider_events::{provider_event_channel, ProviderEventReceiver, ProviderEventSender},
        provider_network::ProviderNetwork,
        translation_client::TranslationClient,
    },
    core::{
        audio_input::AudioSource,
        configuration::LiveTranslationConfiguration,
        models::{SourceLanguage, TargetLanguage, TranslationMode},
        network_proxy::ProxyConfig,
        protocols::live_translate::LiveTranslateServerEvent,
        provider::{ProviderKind, ProviderPreferences, TextTranslation},
        recovery::{
            advance_lifecycle_sequence_if_current, provider_error_is_retryable,
            provider_recovery_minimum_delay, MTBudgetContinuity, MTBudgetScope, RecoverySchedule,
        },
        session::TranslationSessionController,
    },
};
use serde::Deserialize;
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
    sync::{watch, Notify},
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
struct Connection {
    client: TranslationClient,
    pipeline: AudioSendPipeline,
    ready: watch::Sender<bool>,
    events: ProviderEventSender,
    budget: Option<mimi_runtime::core::preview_pacing::MTRequestBudget>,
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
    fn publish(&self, update: impl FnOnce(&mut TranslationSessionController)) {
        update(&mut self.controller.lock().unwrap());
        self.version.fetch_add(1, Ordering::SeqCst);
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
        *self.connection.lock().unwrap() = Some(Connection {
            client,
            pipeline,
            ready,
            events,
            budget,
        });
        Ok(receiver)
    }
    fn ingress(&self) -> Option<AudioIngress> {
        self.connection
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|value| value.pipeline.ingress())
    }
    async fn retire(&self, generation: u64) {
        let connection = self.connection.lock().unwrap().take();
        if let Some(connection) = connection {
            connection.pipeline.stop();
            let lease = self.budget.lock().unwrap().take_lease(generation);
            if let Some((token, scope)) = lease {
                if let Some(budget) = connection.client.suspend_mt_request_budget().await {
                    self.budget.lock().unwrap().remember(token, scope, budget);
                }
            }
            let _ = tokio::time::timeout(
                mimi_runtime::core::health::DISCONNECT_TIMEOUT,
                connection.client.disconnect(),
            )
            .await;
        }
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
                            match event.event {
                                LiveTranslateServerEvent::Error {code,..} => { failure = Some(mimi_runtime::core::recovery::normalize_provider_error_code(&code).into()); break; }
                                LiveTranslateServerEvent::SessionFinished => {
                                    health.abort();
                                    self.publish(TranslationSessionController::did_stop);
                                    self.retire(generation).await;
                                    self.finished.store(true,Ordering::SeqCst);
                                    return;
                                }
                                event => self.publish(|controller|controller.handle_from(AudioSource::System,event)),
                            }
                        }
                        _ = ticks.tick() => {
                            if self.controller.lock().unwrap().tick(std::time::Instant::now()) {
                                self.version.fetch_add(1,Ordering::SeqCst);
                            }
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
        let connection = self.connection.lock().unwrap().take();
        if let Some(connection) = connection {
            let finish = async {
                connection
                    .pipeline
                    .finish(mimi_runtime::core::health::AUDIO_DRAIN_TIMEOUT)
                    .await;
                let _ = tokio::time::timeout(
                    mimi_runtime::core::health::PROVIDER_FINISH_TIMEOUT,
                    connection.client.finish(),
                )
                .await;
            };
            tokio::pin!(finish);
            loop {
                tokio::select! {
                    _ = &mut finish => break,
                    event = receiver.recv_with_revision() => {if let Some(event)=event {self.publish(|controller|controller.handle_from(AudioSource::System,event.event));}else{break;}}
                }
            }
            while let Ok(event) = receiver.try_recv_with_revision() {
                self.publish(|controller| controller.handle_from(AudioSource::System, event.event));
            }
            let _ = tokio::time::timeout(
                mimi_runtime::core::health::DISCONNECT_TIMEOUT,
                connection.client.disconnect(),
            )
            .await;
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
            let capabilities = provider.capabilities_for_route(route, target);
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
                let normalized = provider
                    .capabilities_for_route(route, first.target_language)
                    .normalize(first);
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
            json!({"speechEndpoint":speech,"textEndpoint":text})
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
            if native.started.swap(true, Ordering::SeqCst) {
                return Err("native_runtime_already_started");
            }
            let _guard = executor().enter();
            let receiver = native.connection(1)?;
            *native.task.lock().unwrap() = Some(executor().spawn(native.clone().drive(receiver)));
            json!({})
        }
        "poll" => {
            let native = session(
                value["handle"]
                    .as_u64()
                    .ok_or("native_runtime_invalid_request")?,
            )?;
            let controller = native.controller.lock().unwrap();
            let state = &controller.state;
            json!({"version":native.version.load(Ordering::SeqCst),"snapshot":state.subtitles,"atomicPreview":state.atomic_preview,"detectedLanguage":state.detected_language.as_ref().map(|language|language.code.as_str()),"isTranslationPending":state.is_translation_pending,"isTranslationPreviewPending":state.is_translation_preview_pending,"isTranslationTimedOut":state.is_translation_timed_out,"translationRecovery":state.translation_recovery,"finished":native.finished.load(Ordering::SeqCst),"errorCode":*native.error_code.lock().unwrap()})
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
                native.stopping.store(true, Ordering::SeqCst);
                native.epoch.fetch_add(1, Ordering::SeqCst);
                if let Some(task) = native.task.lock().unwrap().take() {
                    task.abort();
                }
                executor().spawn(async move {
                    native.retire(0).await;
                });
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
        assert!(received.chunks_exact(2).all(|sample| sample == [0, 16]));
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
