//! Manual start, stop, pause and resume.
//! Own the same admission/content gates through establishment, final drain and failed-resume restoration.

use super::cleanup_owner::CleanupTicket;
use super::history::LocalCaptureStats;
use super::lanes::connect_source_group;
use super::ownership::{
    accepted_pause_transition_is_valid, begin_pause_request, pause_transition_is_valid,
    restore_failed_resume, resume_failure_is_still_owned, resume_transition_is_valid,
    start_request_can_proceed, start_request_can_proceed_after_teardown, SessionRequestGuard,
    SESSION_START_CANCELLED,
};
use super::publication::publish_stop_boundary;
use super::recording_state::RecordingEvidence;
use super::recovery_cleanup::{
    abort_recovery_task, activate_start_if_current, finish_pause, retire_recovery_for_start,
};
use super::SessionManager;
use crate::audio::echo_pipeline::EchoPipeline;
use crate::audio::send_pipeline::{AudioPipelineFailure, AudioSendPipeline};
use crate::audio::AudioCaptureFormat;
use crate::clients::translation::translation_client::{ConnectError, TranslationClient};
use crate::clients::transport::provider_events::provider_event_channel;
use crate::core::audio::audio_input::{AudioInput, AudioSource};
use crate::core::diagnostics::support_diagnostics::{
    DiagnosticEvent, LifecycleAction, RecoveryAction, SafeFailure,
};
use crate::core::lifecycle::session::TranslationSessionController;
use crate::core::models::SessionStatus;
use crate::core::protocols::realtime::live_translate::LiveTranslateServerEvent;
use crate::core::services::configuration::LiveTranslationConfiguration;
use crate::core::services::provider::ProviderKind;
use crate::pipeline_log;
use mimi_runtime::core::lifecycle::health::AUDIO_DRAIN_TIMEOUT;
use mimi_runtime::core::lifecycle::recovery::{
    provider_error_requires_user_action, MTBudgetScope, NO_GENERATION,
};
use std::future::Future;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

pub(super) fn apply_establish_failure_state(
    controller: &mut TranslationSessionController,
    error: String,
    is_recovering: bool,
) {
    if is_recovering && !session_error_requires_user_action(&error) {
        controller.begin_connecting();
    } else {
        controller.did_fail(error);
    }
}

/// These fixed application errors cannot improve through automatic reconnect.
/// Starting again is an explicit user action after authorization or system stop.
pub(super) fn configuration_failure_requires_repair(error: &str) -> bool {
    crate::core::protocols::recognition::audio3::failure_requires_configuration(error)
        || matches!(
            error,
            "apple_translation_unavailable"
                | "apple_translation_assets_missing"
                | "apple_translation_language_unsupported"
                | "apple_translation_invalid_input"
        )
}

pub(super) fn session_error_requires_user_action(error: &str) -> bool {
    provider_error_requires_user_action(error)
        || matches!(
            error,
            "System audio capture permission was denied."
                | "Microphone capture permission was denied."
                | "System audio capture was stopped by the user."
                | "alibaba_realtime_provider_error"
                | "openai_realtime_provider_error"
                | "openai_realtime_setup_expired"
                | "openai_realtime_rate_limited"
                | "openai_realtime_service_error"
                | "xai_realtime_provider_error"
                | "gemini_live_provider_error"
                | "baidu_realtime_provider_error"
                | "audio3_provider_error"
        )
}

pub(super) fn session_connect_error(error: ConnectError) -> String {
    // Setup can reject before the provider event pump observes its error.
    // Preserve the shared typed classification across this String boundary so
    // recovery cannot turn a permanent rejection into another transport retry.
    error
        .recognition_failure_label()
        .map(str::to_owned)
        .unwrap_or_else(|| error.to_string())
}

/// A successful repair may retire only the error observed before that write.
pub(super) fn clear_stale_configuration_error(
    controller: &mut TranslationSessionController,
    expected: Option<&(u64, String)>,
    current_epoch: u64,
    changed: bool,
) -> bool {
    let Some((epoch, error)) = expected else {
        return false;
    };
    if !changed
        || *epoch != current_epoch
        || !configuration_failure_requires_repair(error)
        || controller.state.status != SessionStatus::Error(error.clone())
    {
        return false;
    }
    controller.did_stop();
    true
}

impl SessionManager {
    /// Called under the settings mutation guard before a persisted repair.
    pub fn configuration_failure_snapshot(&self) -> Option<(u64, String)> {
        let controller = self.controller.lock().unwrap();
        match &controller.state.status {
            SessionStatus::Error(error)
                if configuration_failure_requires_repair(error)
                    && self.active_generation.load(Ordering::SeqCst) == NO_GENERATION =>
            {
                Some((
                    self.lifecycle_sequence.load(Ordering::SeqCst),
                    error.clone(),
                ))
            }
            _ => None,
        }
    }

    /// Does not start capture, erase subtitles or clear a newer session's error.
    pub fn configuration_saved(self: &Arc<Self>, expected: Option<(u64, String)>, changed: bool) {
        let cleared = {
            let _transition = self.generation_transition.lock().unwrap();
            if self.active_generation.load(Ordering::SeqCst) != NO_GENERATION {
                return;
            }
            if changed {
                self.recovery_continuity.lock().unwrap().clear();
            }
            clear_stale_configuration_error(
                &mut self.controller.lock().unwrap(),
                expected.as_ref(),
                self.lifecycle_sequence.load(Ordering::SeqCst),
                changed,
            )
        };
        if cleared {
            self.publish_state();
        }
    }

    pub(super) fn can_pause_current_session(&self) -> bool {
        let status = self.controller.lock().unwrap().state.status.clone();
        pause_transition_is_valid(
            &status,
            self.is_paused(),
            self.active_generation.load(Ordering::SeqCst),
        )
    }

    pub(super) fn can_resume_current_session(&self) -> bool {
        let status = self.controller.lock().unwrap().state.status.clone();
        let has_active_settings = self.active_settings.lock().unwrap().is_some();
        resume_transition_is_valid(
            &status,
            self.is_paused(),
            self.active_generation.load(Ordering::SeqCst),
            has_active_settings,
        )
    }

    /// Starts a listening session while preserving the bounded confirmed display.
    /// Only the explicit clear action erases subtitles; new sessions reset their
    /// transport state and opt-in archives independently.
    pub async fn start(self: &Arc<Self>) -> Result<(), String> {
        self.record_diagnostic_event(DiagnosticEvent::Lifecycle {
            action: LifecycleAction::StartRequested,
        });
        let status_label = {
            let controller = self.controller.lock().unwrap();
            match &controller.state.status {
                SessionStatus::Idle => "idle",
                SessionStatus::Connecting => "connecting",
                SessionStatus::Listening => "listening",
                SessionStatus::Stopping => "stopping",
                SessionStatus::Error(_) => "error",
            }
        };
        pipeline_log!(
            "session start entered status={} generation={} recovering={} startBusy={} teardownOperations={}",
            status_label,
            self.active_generation.load(Ordering::SeqCst),
            u8::from(self.is_recovering.load(Ordering::SeqCst)),
            u8::from(self.start_in_progress.load(Ordering::SeqCst)),
            self.teardown_operations.load(Ordering::SeqCst),
        );
        let Some(_start_request) = SessionRequestGuard::acquire(&self.start_in_progress) else {
            self.record_diagnostic_event(DiagnosticEvent::Lifecycle {
                action: LifecycleAction::StartBusy,
            });
            pipeline_log!("session start skipped label=start_in_progress");
            return Ok(());
        };
        let accepted_recovery_replacement = self.is_recovering.load(Ordering::SeqCst);
        if !start_request_can_proceed(
            self.has_active_session(),
            accepted_recovery_replacement,
            self.active_generation.load(Ordering::SeqCst),
        ) {
            self.record_diagnostic_event(DiagnosticEvent::Lifecycle {
                action: LifecycleAction::StartAlreadyActive,
            });
            pipeline_log!("session start skipped label=active_session");
            return Ok(());
        }
        let _operation = self.begin_lifecycle_operation();
        let request_generation = self.next_lifecycle_request();
        let lifecycle = self.lock_after_teardown().await;
        let replacement_cleanup = if accepted_recovery_replacement {
            let Some(cleanup) = retire_recovery_for_start(
                &self.generation_transition,
                &self.lifecycle_sequence,
                &self.active_generation,
                request_generation,
                |generation| self.cleanup_generation(generation),
            ) else {
                return Ok(());
            };
            cleanup
        } else {
            CleanupTicket::completed()
        };
        if !self.is_lifecycle_request_current(request_generation)
            || !start_request_can_proceed_after_teardown(
                self.has_active_session(),
                self.is_recovering.load(Ordering::SeqCst),
                self.active_generation.load(Ordering::SeqCst),
                accepted_recovery_replacement,
            )
        {
            self.record_diagnostic_event(DiagnosticEvent::Lifecycle {
                action: if self.is_lifecycle_request_current(request_generation) {
                    LifecycleAction::StartAlreadyActive
                } else {
                    LifecycleAction::StartSuperseded
                },
            });
            pipeline_log!(
                "session start skipped label={}",
                if self.is_lifecycle_request_current(request_generation) {
                    "active_session_after_teardown"
                } else {
                    "superseded"
                }
            );
            return Ok(());
        }
        // A manual start is a new session; server quota can still be exhausted,
        // but no old client accounting may be restored into this new intent.
        for lane in &self.lanes {
            lane.mt_budget_continuity.lock().unwrap().reset();
        }
        // A manual start during recovery backoff is the newer user intent.
        // Cancel the old owner before installing this generation so its
        // global recovery flag cannot affect the new session's error path.
        self.cancel_recovery().await;
        replacement_cleanup.await;
        if !self.is_lifecycle_request_current(request_generation) {
            return Ok(());
        }
        self.persist_current_history()
            .map_err(|_| "Could not save the previous session history.".to_string())?;
        let preferences = self.settings.preferences();
        *self.active_audio_input.lock().unwrap() = preferences.audio_input;
        self.controller
            .lock()
            .unwrap()
            .set_audio_input(preferences.audio_input);
        let started_at_ms = crate::core::subtitles::subtitle_reducer::now_epoch_ms();
        let history_id = (preferences.retain_session_history || preferences.record_session_audio)
            .then(|| uuid::Uuid::new_v4().to_string());
        if let Some(id) = &history_id {
            if !self.history.available() && !self.is_ui_test() {
                return Err("Local session history is unavailable.".to_string());
            }
            self.history
                .begin(id, started_at_ms)
                .map_err(|_| "Could not start local session history.".to_string())?;
        }
        if !activate_start_if_current(
            &self.generation_transition,
            &self.lifecycle_sequence,
            &self.active_generation,
            request_generation,
        ) {
            return Ok(());
        }
        self.stopping_tail_generation
            .store(NO_GENERATION, Ordering::SeqCst);
        self.is_paused.store(false, Ordering::SeqCst);
        *self.history_pending_id.lock().unwrap() = history_id;
        self.history_pending_text
            .store(preferences.retain_session_history, Ordering::SeqCst);
        self.history_pending_audio
            .store(preferences.record_session_audio, Ordering::SeqCst);
        *self.history_stats.lock().unwrap() = LocalCaptureStats {
            audio_evidence: RecordingEvidence::new(preferences.audio_input),
            ..Default::default()
        };
        self.history_save_error.store(false, Ordering::SeqCst);
        self.archive_revision.fetch_add(1, Ordering::SeqCst);
        self.controller.lock().unwrap().archive_mut().begin(
            self.is_ui_test() && preferences.retain_session_history,
            started_at_ms,
        );
        for recording in self.recording.lock().unwrap().iter_mut() {
            recording.begin(false);
        }
        self.controller.lock().unwrap().begin_session();
        // Do not put WebView delivery on the provider setup path.
        self.schedule_lifecycle_state();
        // UI fixtures must never resolve saved credentials, open a socket,
        // or touch native audio capture.
        if self.is_ui_test() {
            if self.is_generation_current(request_generation) {
                return self.establish_ui_test_session();
            }
            return Ok(());
        }
        if let Err(error) = self.settings.prepare_for_listening() {
            if self.invalidate_generation(request_generation) {
                self.record_diagnostic_failure(&error);
                self.controller.lock().unwrap().did_fail(error.clone());
                self.publish_state();
                return Err(error);
            }
            return Err(SESSION_START_CANCELLED.into());
        }
        let configuration = match self.settings.configuration() {
            Ok(configuration) => configuration,
            Err(error) => {
                pipeline_log!("session settings failed label=settings_configuration");
                if self.invalidate_generation(request_generation) {
                    self.record_diagnostic_failure(&error);
                    self.controller.lock().unwrap().did_fail(error.clone());
                    self.publish_state();
                    return Err(error);
                }
                return Err(SESSION_START_CANCELLED.into());
            }
        };
        pipeline_log!(
            "session start requested provider={} source={} target={} mode={:?}",
            configuration.provider.wire_value(),
            configuration.source_language.raw_value(),
            configuration.target_language.raw_value(),
            configuration.effective_translation_mode()
        );

        self.set_active_settings(request_generation, Some(configuration));
        // All shared state is now generation-tagged. Do not hold the
        // lifecycle gate across socket setup or capture authorization: stop
        // must be able to invalidate this generation immediately.
        drop(lifecycle);
        self.establish_session(request_generation).await
    }

    pub(super) async fn establish_session(self: &Arc<Self>, generation: u64) -> Result<(), String> {
        self.stop_health_checks().await;

        self.ensure_generation_current(generation)?;

        if self.is_ui_test() {
            return self.establish_ui_test_session();
        }

        // A language/mode switch may land while connecting (the picker stays
        // usable during connecting); at most one rebuild picks up the change
        // so a rapid switch storm cannot loop forever.
        let mut attempts = 0usize;
        loop {
            attempts += 1;
            let configuration = match self.active_settings.lock().unwrap().clone() {
                Some(configuration) => configuration,
                None => {
                    let configuration = self.settings.configuration()?;
                    self.set_active_settings(generation, Some(configuration.clone()));
                    configuration
                }
            };

            pipeline_log!("session connecting");
            self.ensure_generation_current(generation)?;
            // Manual Start already published Connecting before reading
            // credentials. Resume publishes here after changing ownership.
            let already_connecting = {
                let mut controller = self.controller.lock().unwrap();
                let was_connecting = controller.state.status == SessionStatus::Connecting;
                controller.begin_connecting();
                was_connecting
            };
            if !already_connecting {
                self.schedule_lifecycle_state();
            }

            let result = Arc::clone(self)
                .connect_and_listen(configuration.clone(), generation)
                .await;
            if let Err(error) = result {
                // Apple setup refreshes the resource cache before starting.
                // Publish even rejected or superseded starts so every picker
                // drops resources that disappeared since its last snapshot.
                if configuration.provider == ProviderKind::AppleSpeech {
                    self.publish_settings();
                }
                pipeline_log!("session establish failed label=provider_or_capture_setup");
                // Reserve teardown before invalidation permits a newer start.
                // Dropping this ticket still starts the independent owner.
                let cleanup = self.cleanup_generation_without_pump(generation);
                // Claim failure before cleanup can await. Otherwise a provider
                // pump can claim recovery during teardown and then be aborted
                // before it installs the retry task.
                let Some(failure_epoch) = self.invalidate_generation_with_epoch(generation) else {
                    cleanup.await;
                    return Err(SESSION_START_CANCELLED.into());
                };
                self.cleanup_generation(generation).await;
                drop(cleanup);
                let _lifecycle = Arc::clone(&self.lifecycle_lock).lock_owned().await;
                if self.lifecycle_sequence.load(Ordering::SeqCst) != failure_epoch {
                    return Err(SESSION_START_CANCELLED.into());
                }
                let is_recovering = self.is_recovering.load(Ordering::SeqCst);
                if !is_recovering {
                    self.clear_active_settings_for_generation(generation);
                }
                self.record_diagnostic_failure(&error);
                apply_establish_failure_state(
                    &mut self.controller.lock().unwrap(),
                    error.clone(),
                    is_recovering,
                );
                self.publish_state();
                return Err(error);
            }

            self.ensure_generation_current(generation)?;

            // The picker allows switching while connecting; if the settings
            // changed under the in-flight connect, rebuild the session with
            // the fresh configuration instead of going live with the stale
            // one (the user sees the capsule state and the session diverge
            // otherwise).
            let fresh = self.settings.configuration().ok();
            let stale = fresh.as_ref().is_none_or(|fresh| fresh != &configuration);
            if !stale || attempts >= 2 {
                self.publish_lifecycle_state().await;
                return self.ensure_generation_current(generation);
            }
            if let Some(fresh) = fresh {
                self.set_active_settings(generation, Some(fresh));
            }
            pipeline_log!("session rebuild for settings changed mid-connect");
            self.cleanup_generation(generation).await;
            self.ensure_generation_current(generation)?;
        }
    }

    pub(super) fn connect_and_listen(
        self: Arc<Self>,
        configuration: LiveTranslationConfiguration,
        generation: u64,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<(), String>> + Send>> {
        Box::pin(async move {
            let attempt = self.next_background_task_id();
            {
                let _transition = self.generation_transition.lock().unwrap();
                self.ensure_generation_current(generation)?;
                *self.audio_attempt.lock().unwrap() = Some((generation, attempt));
                self.history_stats
                    .lock()
                    .unwrap()
                    .audio_evidence
                    .begin_attempt(generation, attempt);
            }
            if *self.active_audio_input.lock().unwrap() == AudioInput::Both {
                let echo = Arc::new(
                    EchoPipeline::spawn(
                        configuration.capabilities().input_sample_rate_hz,
                        self.capture_failure_channel(generation, attempt),
                    )
                    .map_err(|error| error.to_string())?,
                );
                let _transition = self.generation_transition.lock().unwrap();
                self.ensure_generation_current(generation)?;
                let mut slot = self.echo_pipeline.lock().unwrap();
                if slot.is_some() {
                    return Err("An audio processor is already assigned to a session.".into());
                }
                *slot = Some((generation, echo));
            }
            connect_source_group(
                self.sources(),
                |source| {
                    Arc::clone(&self).connect_source(
                        configuration.clone(),
                        generation,
                        attempt,
                        source,
                    )
                },
                |pumps| self.cleanup_generation_resources(generation, pumps),
            )
            .await?;
            self.commit_listening(generation)?;
            self.start_health_checks(generation).await;
            self.ensure_generation_current(generation)
        })
    }

    pub(super) fn connect_source(
        self: Arc<Self>,
        configuration: LiveTranslationConfiguration,
        generation: u64,
        attempt: u64,
        source: AudioSource,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>> {
        Box::pin(async move {
            // Create the client and consume its events through this manager.
            let (event_tx, mut event_rx) = provider_event_channel();
            event_tx.set_debug_context(source, generation);
            let audio_error_events = event_tx.clone();
            let new_client = TranslationClient::new(&configuration, event_tx).map_err(|error| {
                pipeline_log!(
                    "provider client creation failed label={}",
                    error.diagnostic_label()
                );
                error.to_string()
            })?;
            self.ensure_generation_current(generation)?;
            let scope =
                self.settings.active_profile().ok().and_then(|profile| {
                    MTBudgetScope::for_configuration(profile.id, &configuration)
                });
            let budget = {
                let _transition = self.generation_transition.lock().unwrap();
                self.ensure_generation_current(generation)?;
                if !self.is_lifecycle_request_current(generation) {
                    return Err(SESSION_START_CANCELLED.into());
                }
                self.lane(source)
                    .mt_budget_continuity
                    .lock()
                    .unwrap()
                    .prepare(generation, scope)
            };
            new_client.restore_mt_request_budget(budget).await;
            self.ensure_generation_current(generation)?;
            self.install_client(source, generation, new_client).await?;

            // Start consuming before awaiting setup: a provider may acknowledge
            // setup and immediately send a terminal error/close in the same
            // socket read. The terminal event invalidates this generation, and
            // every later startup step checks it before installing resources.
            let self_arc = Arc::clone(&self);
            let pump = tokio::spawn(async move {
                while let Some(event) = event_rx.recv_with_revision().await {
                    self_arc.handle_event(source, generation, event).await;
                }
            });
            self.install_pump(source, generation, pump);

            let client = self
                .client_for_generation(source, generation)
                .ok_or_else(|| SESSION_START_CANCELLED.to_string())?;
            let connect_result = self
                .run_while_generation_current(
                    generation,
                    crate::development_audio::scope(source, generation, client.connect()),
                )
                .await;
            let connect_result = match connect_result {
                Ok(result) => result,
                Err(error) => {
                    self.cleanup_generation_without_pump(generation).await;
                    return Err(error);
                }
            };
            connect_result.map_err(session_connect_error)?;
            tokio::task::yield_now().await;
            if let Err(error) = self.ensure_generation_current(generation) {
                self.cleanup_generation_without_pump(generation).await;
                return Err(error);
            }
            pipeline_log!("asr websocket connected");

            // Create the sole bounded audio queue before capture starts. The
            // native callback writes directly to this synchronous ingress;
            // there is no unbounded bridge ahead of the network sender.
            let audio_format =
                AudioCaptureFormat::pcm16_mono(configuration.capabilities().input_sample_rate_hz)
                    .map_err(|error| error.to_string())?;
            let send_manager = Arc::clone(&self);
            let on_error_self = Arc::clone(&self);
            let pipeline = Arc::new(AudioSendPipeline::spawn_with_error_handler(
                move |data| {
                    let manager = Arc::clone(&send_manager);
                    Box::pin(async move {
                        manager.record_audio(
                            source,
                            generation,
                            attempt,
                            audio_format.sample_rate_hz,
                            &data,
                        );
                        let client = manager.client_for_generation(source, generation);
                        match client {
                            Some(client) => crate::development_audio::scope(
                                source,
                                generation,
                                client.send_audio(&data),
                            )
                            .await
                            .map_err(Some),
                            None => Err(None),
                        }
                    })
                },
                move |failure| {
                    if let Some(label) = failure
                        .as_ref()
                        .and_then(|error| error.recognition_failure_label())
                    {
                        // Use the same generation-bound provider lane as the
                        // recognition reader, including permanent failures.
                        if audio_error_events
                            .send(LiveTranslateServerEvent::Error {
                                code: label.into(),
                                message: label.into(),
                            })
                            .is_ok()
                        {
                            return;
                        }
                    }
                    let classification = failure
                        .as_ref()
                        .map(|error| SafeFailure::from_error(&error.to_string()));
                    let manager = Arc::clone(&on_error_self);
                    tokio::spawn(async move {
                        manager
                            .handle_audio_transport_failure(
                                generation,
                                attempt,
                                source,
                                AudioPipelineFailure::TransportStopped,
                                classification,
                            )
                            .await;
                    });
                },
            ));
            let audio_ingress = pipeline
                .ingress()
                .ok_or_else(|| "The bounded audio pipeline is unavailable.".to_string())?;
            let audio_ingress = {
                let echo = self.echo_pipeline.lock().unwrap();
                match echo.as_ref().filter(|(owner, _)| *owner == generation) {
                    Some((_, echo)) => echo.ingress(source, audio_ingress),
                    None => audio_ingress,
                }
            };
            self.ensure_generation_current(generation)?;
            self.install_pipeline(source, generation, Arc::clone(&pipeline))?;
            client.set_audio_pending_gate(pipeline.pending_pcm_gate());

            let audio_failure_tx = self.capture_failure_channel(generation, attempt);
            self.ensure_generation_current(generation)?;
            self.lane(source)
                .capture_generation
                .compare_exchange(
                    NO_GENERATION,
                    generation,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .map_err(|_| "Audio capture is already assigned to a session.".to_string())?;
            let capture = self.lane(source).audio.lock().unwrap().clone();
            #[cfg(target_os = "windows")]
            capture.set_source(self.settings.preferences().windows_audio_source);
            match self
                .run_while_generation_current(
                    generation,
                    capture.start(
                        audio_ingress,
                        audio_failure_tx,
                        audio_format,
                        source,
                        self.settings.preferences().system_audio_target,
                    ),
                )
                .await
            {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    // A platform capture may fail after it has installed a
                    // native stream but before its start acknowledgement is
                    // delivered. Always request generation-scoped teardown;
                    // the platform implementation also cleans up its own
                    // start token before returning this error.
                    self.cleanup_generation_without_pump(generation).await;
                    return Err(error.to_string());
                }
                Err(error) => {
                    self.cleanup_generation_without_pump(generation).await;
                    return Err(error);
                }
            }
            if let Err(error) = self.ensure_generation_current(generation) {
                self.cleanup_generation_without_pump(generation).await;
                return Err(error);
            }
            pipeline_log!("audio capture started");

            Ok(())
        })
    }

    pub async fn stop(self: &Arc<Self>) {
        self.record_diagnostic_event(DiagnosticEvent::Lifecycle {
            action: LifecycleAction::StopRequested,
        });
        self.record_recovery_action(RecoveryAction::UserStopped);
        let _operation = self.begin_lifecycle_operation();
        let _stop_request = self.next_lifecycle_request();
        for lane in &self.lanes {
            lane.mt_budget_continuity.lock().unwrap().reset();
        }
        let stopping_generation = self.active_generation.swap(NO_GENERATION, Ordering::SeqCst);
        if stopping_generation != NO_GENERATION {
            self.stopping_tail_generation
                .store(stopping_generation, Ordering::SeqCst);
        }
        let _lifecycle = Arc::clone(&self.lifecycle_lock).lock_owned().await;
        self.retire_audio_attempt(stopping_generation);
        if !self.controller.lock().unwrap().state.status.is_active()
            && self.lanes.iter().all(|lane| {
                lane.client.lock().unwrap().is_none()
                    && lane.capture_generation.load(Ordering::SeqCst) == NO_GENERATION
            })
        {
            if let Some(echo) = self.take_echo_pipeline(stopping_generation) {
                echo.stop();
            }
            self.stop_health_checks().await;
            self.cancel_recovery().await;
            self.cancel_translation_timeout();
            self.clear_active_settings();
            self.is_recovering.store(false, Ordering::SeqCst);
            self.is_paused.store(false, Ordering::SeqCst);
            self.stopping_tail_generation
                .store(NO_GENERATION, Ordering::SeqCst);
            self.controller.lock().unwrap().did_stop();
            if self.persist_current_history().is_err() {
                pipeline_log!("session history save failed label=write_failed");
            }
            publish_stop_boundary(|| self.publish_state(), self.publish_state_now()).await;
            return;
        }
        pipeline_log!("session stop requested");

        self.stop_health_checks().await;
        self.cancel_recovery().await;
        self.cancel_translation_timeout();
        self.clear_active_settings();
        self.is_recovering.store(false, Ordering::SeqCst);
        self.is_paused.store(false, Ordering::SeqCst);
        self.controller.lock().unwrap().begin_stopping();
        self.publish_state();

        if let Some((_, echo)) = self.echo_pipeline.lock().unwrap().as_ref() {
            echo.seal();
        }

        // Stop capture first so the queue has a fixed upper bound, then give
        // buffers already accepted by the network pipeline a finite drain
        // window before asking the provider to close its session.
        if stopping_generation != NO_GENERATION {
            tokio::join!(
                self.stop_capture_for_generation(AudioSource::System, stopping_generation),
                self.stop_capture_for_generation(AudioSource::Microphone, stopping_generation),
            );
        } else {
            tokio::join!(
                self.stop_any_capture(AudioSource::System),
                self.stop_any_capture(AudioSource::Microphone)
            );
        }
        if let Some(echo) = self.take_echo_pipeline(stopping_generation) {
            if !echo.finish(Duration::from_secs(1)).await {
                pipeline_log!("audio processing drain timed out");
            }
        }
        for source in [AudioSource::System, AudioSource::Microphone] {
            if stopping_generation != NO_GENERATION {
                self.finish_pipeline_for_generation(
                    source,
                    stopping_generation,
                    Duration::from_secs(1),
                )
                .await;
            } else {
                self.finish_source_pipeline(source, AUDIO_DRAIN_TIMEOUT)
                    .await;
            }
            let taken = if stopping_generation == NO_GENERATION {
                self.take_any_client(source)
            } else {
                self.take_client_for_generation(source, stopping_generation)
            };
            if let Some(client) = taken {
                if tokio::time::timeout(
                    client.finish_timeout(),
                    crate::development_audio::scope(source, stopping_generation, client.finish()),
                )
                .await
                .is_err()
                {
                    pipeline_log!("provider finish timed out");
                    let _ = tokio::time::timeout(Duration::from_secs(1), client.disconnect()).await;
                }
            }
            if stopping_generation != NO_GENERATION {
                self.finish_pump_for_generation(
                    source,
                    stopping_generation,
                    Duration::from_millis(500),
                )
                .await;
            } else {
                self.stop_any_pump(source);
            }
        }
        self.stopping_tail_generation
            .store(NO_GENERATION, Ordering::SeqCst);
        self.controller.lock().unwrap().did_stop();
        if self.persist_current_history().is_err() {
            pipeline_log!("session history save failed label=write_failed");
        }
        publish_stop_boundary(|| self.publish_state(), self.publish_state_now()).await;
        pipeline_log!("session stopped");
    }

    pub async fn toggle_paused(self: &Arc<Self>) -> Result<(), String> {
        let Some(request) =
            begin_pause_request(&self.pause_in_progress, &self.start_in_progress, || {
                self.is_paused()
            })
            .map_err(str::to_owned)?
        else {
            return Ok(());
        };
        if self.is_ui_test() {
            // Native smoke can arm a configuration failure for the current
            // synthetic generation without opening a provider or capture.
            if !request.resume
                && std::env::var_os("MIMI_UI_TEST_PREFERENCES_DIR").is_some_and(|directory| {
                    std::path::Path::new(&directory)
                        .join("fail-next-pause")
                        .is_file()
                })
                && self.invalidate_generation(self.active_generation.load(Ordering::SeqCst))
            {
                self.controller
                    .lock()
                    .unwrap()
                    .did_fail("credential_authentication_failed");
                self.publish_lifecycle_state().await;
                return Ok(());
            }
            self.is_paused.store(!request.resume, Ordering::SeqCst);
            if request.resume {
                self.controller.lock().unwrap().did_connect();
            } else {
                self.controller.lock().unwrap().did_pause();
            }
            self.publish_lifecycle_state().await;
            return Ok(());
        }
        if request.resume {
            self.resume().await
        } else {
            self.pause().await;
            Ok(())
        }
    }

    pub(super) async fn pause(self: &Arc<Self>) {
        if !self.can_pause_current_session() {
            return;
        }
        let _operation = self.begin_lifecycle_operation();
        let pause_request = self.next_lifecycle_request();
        let _lifecycle = Arc::clone(&self.lifecycle_lock).lock_owned().await;
        if !self.is_lifecycle_request_current(pause_request) {
            return;
        }
        let status = self.controller.lock().unwrap().state.status.clone();
        if !accepted_pause_transition_is_valid(
            &status,
            self.is_paused(),
            self.active_generation.load(Ordering::SeqCst),
            self.active_settings.lock().unwrap().is_some(),
        ) {
            return;
        }
        let paused_generation = self.active_generation.swap(NO_GENERATION, Ordering::SeqCst);
        self.record_diagnostic_event(DiagnosticEvent::Lifecycle {
            action: LifecycleAction::PauseRequested,
        });
        pipeline_log!("session pause requested");
        self.is_paused.store(true, Ordering::SeqCst);
        self.stop_health_checks().await;
        let recovery = self.take_recovery_for_cancellation();
        abort_recovery_task(recovery).await;
        self.cancel_translation_timeout();
        self.controller.lock().unwrap().did_pause();
        finish_pause(&self.cleanup_owners, self.publish_lifecycle_state(), || {
            self.cleanup_generation(paused_generation)
        })
        .await;
        pipeline_log!("session paused");
    }

    pub(super) async fn resume(self: &Arc<Self>) -> Result<(), String> {
        if !self.can_resume_current_session() {
            return Ok(());
        }
        let _operation = self.begin_lifecycle_operation();
        let resume_generation = self.next_lifecycle_request();
        let lifecycle = Arc::clone(&self.lifecycle_lock).lock_owned().await;
        if !self.is_lifecycle_request_current(resume_generation)
            || !self.can_resume_current_session()
        {
            return Ok(());
        }
        pipeline_log!("session resume requested");
        self.record_diagnostic_event(DiagnosticEvent::Lifecycle {
            action: LifecycleAction::ResumeRequested,
        });
        let paused_configuration = self.active_settings.lock().unwrap().clone();
        self.is_paused.store(false, Ordering::SeqCst);
        self.active_generation
            .store(resume_generation, Ordering::SeqCst);
        self.retag_active_settings(resume_generation);
        drop(lifecycle);
        let error = match self.establish_session(resume_generation).await {
            Ok(()) => {
                pipeline_log!("session resumed");
                return Ok(());
            }
            Err(error) if error == SESSION_START_CANCELLED => return Ok(()),
            Err(error) => error,
        };
        let restored = restore_failed_resume(
            Arc::clone(&self.lifecycle_lock),
            || {
                resume_failure_is_still_owned(
                    &error,
                    resume_generation.wrapping_add(1),
                    self.lifecycle_sequence.load(Ordering::SeqCst),
                    &self.controller.lock().unwrap().state.status,
                    self.active_generation.load(Ordering::SeqCst),
                )
            },
            || {
                self.set_active_settings(NO_GENERATION, paused_configuration);
                self.is_paused.store(true, Ordering::SeqCst);
                self.controller.lock().unwrap().did_pause();
            },
        )
        .await;
        if !restored {
            return Ok(());
        }
        // Deliver the restored paused state before rejecting the IPC action.
        // A newer stop/resume can still win; publish only its current snapshot.
        self.publish_state_now().await;
        pipeline_log!("session resume failed; remaining paused");
        Err(error)
    }
}
