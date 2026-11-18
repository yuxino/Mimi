//! Apply source-owned provider events and translation deadlines under the content gate.

use super::recovery::claim_generation_recovery_atoms;
use super::SessionManager;
use crate::clients::provider_events::ProviderEvent;
use crate::core::audio_input::AudioSource;
use crate::core::protocols::live_translate::LiveTranslateServerEvent;
use crate::core::session::{ProviderEventApplication, TranslationSessionController};
use crate::pipeline_log;
use mimi_runtime::core::recovery::{
    normalize_provider_error_code, provider_error_is_retryable, provider_recovery_minimum_delay,
    NO_GENERATION,
};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

pub(super) fn generation_accepts_event(
    active_generation: u64,
    stopping_tail_generation: u64,
    generation: u64,
    event: &LiveTranslateServerEvent,
) -> bool {
    generation != NO_GENERATION
        && (active_generation == generation
            || (stopping_tail_generation == generation
                && matches!(
                    event,
                    LiveTranslateServerEvent::SubtitleFinalPair { .. }
                        | LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { .. }
                        | LiveTranslateServerEvent::SubtitleConfirmedPair { .. }
                        | LiveTranslateServerEvent::UtteranceRegistered { .. }
                )))
}

pub(super) fn subtitle_content_is_current(
    generation: u64,
    event: &ProviderEvent,
    current: (u64, u64),
) -> bool {
    !event.is_content() || current == (generation, event.content_revision)
}

pub(super) fn confirmed_history_tail_changed(
    previous: Option<&crate::core::models::SubtitlePair>,
    current: Option<&crate::core::models::SubtitlePair>,
) -> bool {
    current.is_some_and(|current| {
        previous.is_none_or(|previous| {
            current.created_at_ms != previous.created_at_ms || current != previous
        })
    })
}

/// Apply the event before capturing its shared deadline. The native timer must
/// not race the reducer and observe a missing or previous request's deadline.
/// A provider terminating either stream terminates the selected session.
pub(super) fn apply_provider_envelope(
    controller: &mut TranslationSessionController,
    source: AudioSource,
    envelope: &ProviderEvent,
) -> ProviderEventApplication {
    let application = envelope.clone().apply_to(controller, source);
    match &envelope.event {
        LiveTranslateServerEvent::SessionFinished => controller.did_stop(),
        LiveTranslateServerEvent::Error { message, .. } => controller.did_fail(message.clone()),
        _ => {}
    }
    application
}

#[cfg(test)]
pub(super) fn apply_provider_event(
    controller: &mut TranslationSessionController,
    source: AudioSource,
    event: &LiveTranslateServerEvent,
) -> Option<Instant> {
    apply_provider_envelope(
        controller,
        source,
        &ProviderEvent::unsequenced(event.clone(), 0),
    )
    .translation_pending_deadline
}

impl SessionManager {
    // MARK: event handling

    pub(super) async fn handle_event(
        self: &Arc<Self>,
        source: AudioSource,
        generation: u64,
        mut envelope: ProviderEvent,
    ) {
        let content = self.subtitle_content_lock.lock().await;
        #[cfg(any(test, feature = "development-debugger"))]
        let (content_revision, transport_sequence) =
            (envelope.content_revision, envelope.transport_sequence);
        #[cfg(any(test, feature = "development-debugger"))]
        let debug_event = |event: &LiveTranslateServerEvent, admission| {
            if crate::core::development_debug::is_enabled() {
                let mut observation = crate::core::development_debug::ProviderObservation::new(
                    source,
                    generation,
                    content_revision,
                    event,
                    admission,
                );
                observation.transport_sequence = transport_sequence;
                crate::core::development_debug::record(
                    crate::core::development_debug::DebugEvent::Provider { observation },
                );
            }
        };
        #[cfg(not(any(test, feature = "development-debugger")))]
        let debug_event =
            |_event: &LiveTranslateServerEvent,
             _admission: crate::core::development_debug::Admission| {};
        if !subtitle_content_is_current(
            generation,
            &envelope,
            *self.lane(source).subtitle_content_revision.lock().unwrap(),
        ) {
            debug_event(
                &envelope.event,
                crate::core::development_debug::Admission::StaleContent,
            );
            return;
        }
        let event = &envelope.event;
        if !self.accepts_event(generation, event) || self.is_paused() {
            debug_event(
                event,
                if self.is_paused() {
                    crate::core::development_debug::Admission::Paused
                } else {
                    crate::core::development_debug::Admission::StaleGeneration
                },
            );
            return;
        }

        // Setup acknowledgements are consumed by `connect`; only
        // `connect_and_listen` may transition to Listening after audio and the
        // send pipeline for the same generation are installed.
        if matches!(
            event,
            LiveTranslateServerEvent::SessionCreated | LiveTranslateServerEvent::SessionUpdated
        ) {
            debug_event(
                event,
                crate::core::development_debug::Admission::SetupAcknowledgement,
            );
            return;
        }

        if let LiveTranslateServerEvent::SubtitleConfirmedPair {
            utterance_id,
            source: source_text,
            translation,
            ..
        } = event
        {
            // A replay is not an accepted final: it must not cancel a newer
            // request's timeout or inflate the content-free journal counts.
            if !self.controller.lock().unwrap().accepts_confirmed_pair_from(
                source,
                *utterance_id,
                source_text,
                translation,
            ) {
                debug_event(
                    event,
                    crate::core::development_debug::Admission::DuplicateFinal,
                );
                return;
            }
        }

        if let LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
            utterance_id,
            source: source_text,
            translation,
            ..
        } = event
        {
            if !self
                .controller
                .lock()
                .unwrap()
                .accepts_identified_final_pair_from(source, utterance_id, source_text, translation)
            {
                debug_event(
                    event,
                    crate::core::development_debug::Admission::DuplicateFinal,
                );
                return;
            }
        }

        if let LiveTranslateServerEvent::Error { code, message } = &mut envelope.event {
            if normalize_provider_error_code(code) == "credential_authentication_failed" {
                *message = "credential_authentication_failed".into();
            }
        }
        let event = &envelope.event;

        if !self.record_accepted_provider_event(source, generation, event) {
            debug_event(
                event,
                crate::core::development_debug::Admission::StaleGeneration,
            );
            return;
        }
        debug_event(event, crate::core::development_debug::Admission::Accepted);

        if let LiveTranslateServerEvent::Error { code, message } = event {
            if provider_error_is_retryable(code) {
                let _teardown = self.begin_teardown_operation();
                let _operation = self.begin_lifecycle_operation();
                let recovery_owns_attempt = {
                    let _transition = self.generation_transition.lock().unwrap();
                    let Some(recovering) = claim_generation_recovery_atoms(
                        &self.active_generation,
                        &self.lifecycle_sequence,
                        &self.is_recovering,
                        &self.recovery_retry_generation,
                        generation,
                    ) else {
                        return;
                    };
                    self.lifecycle_notify.notify_waiters();
                    recovering
                };
                pipeline_log!("session recoverable provider error code={}", code);
                self.cancel_translation_timeout();
                self.controller.lock().unwrap().begin_connecting();
                self.publish_state();
                drop(content);
                let _lifecycle = Arc::clone(&self.lifecycle_lock).lock_owned().await;
                self.stop_health_checks().await;
                self.cleanup_generation_without_pump(generation).await;
                if !recovery_owns_attempt {
                    let minimum_delay = provider_recovery_minimum_delay(code);
                    self.queue_recovery_with_delay(generation, message.clone(), minimum_delay)
                        .await;
                }
                return;
            }
        }

        let is_terminal = matches!(
            event,
            LiveTranslateServerEvent::Error { .. } | LiveTranslateServerEvent::SessionFinished
        );
        let terminal_teardown = is_terminal.then(|| self.begin_teardown_operation());
        let terminal_operation = is_terminal.then(|| self.begin_lifecycle_operation());
        if is_terminal && !self.invalidate_generation(generation) {
            return;
        }
        if is_terminal {
            // A same-generation settings rebuild may already own an Abort
            // cleanup. Its completion can abort this pump while it waits for
            // teardown, so retire terminal settings before the first await.
            self.clear_active_settings_for_generation(generation);
            self.cancel_translation_timeout();
        }
        let (newly_confirmed, application) = {
            let mut controller = self.controller.lock().unwrap();
            #[cfg(any(test, feature = "development-debugger"))]
            let debug_before = crate::core::development_debug::is_enabled()
                .then(|| controller.state.subtitles.clone());
            let previous = controller.state.subtitles.history.last().cloned();
            let application = apply_provider_envelope(&mut controller, source, &envelope);
            #[cfg(any(test, feature = "development-debugger"))]
            if let Some(before) = debug_before {
                crate::core::development_debug::record(
                    crate::core::development_debug::DebugEvent::Reduced {
                        source,
                        generation,
                        changed: before != controller.state.subtitles,
                        before: (&before).into(),
                        after: (&controller.state.subtitles).into(),
                    },
                );
            }
            let current = controller.state.subtitles.history.last();
            let confirmed = confirmed_history_tail_changed(previous.as_ref(), current)
                .then(|| current.cloned())
                .flatten();
            (confirmed, application)
        };
        if application.resolves_translation_timeout {
            self.cancel_source_translation_timeout(source);
        }
        if let Some(deadline) = application.translation_pending_deadline {
            self.arm_translation_timeout(source, generation, deadline);
        }
        if let Some(pair) = newly_confirmed {
            let pending = self.history_pending_id.lock().unwrap();
            if let Some(id) = pending.as_deref() {
                if self.history_pending_text.load(Ordering::SeqCst) {
                    let mut stats = self.history_stats.lock().unwrap();
                    let bytes = pair.source.len().saturating_add(pair.translation.len());
                    if !stats.transcript_limited {
                        if stats.transcript_count
                            >= crate::core::session_archive::TRANSCRIPT_COUNT_LIMIT
                            || bytes
                                > crate::core::session_archive::TRANSCRIPT_BYTE_LIMIT
                                    .saturating_sub(stats.transcript_bytes)
                        {
                            stats.transcript_limited = true;
                        } else if self.history.append_pair(id, &pair).is_ok() {
                            stats.transcript_count += 1;
                            stats.transcript_bytes += bytes;
                        } else {
                            self.history_save_error.store(true, Ordering::SeqCst);
                            pipeline_log!(
                                "session history subtitle append failed label=write_failed"
                            );
                        }
                    }
                }
            }
        }
        self.publish_state();
        drop(content);

        if is_terminal {
            // Provider error codes are not universally trustworthy (some
            // protocols carry arbitrary server strings). Keep diagnostics on
            // a fixed local label just like free-text messages.
            let label = match event {
                LiveTranslateServerEvent::Error { .. } => "provider_terminal_error",
                LiveTranslateServerEvent::SessionFinished => "session_finished",
                _ => "terminal_event",
            };
            pipeline_log!("session terminal event label={label}");
            let _lifecycle = Arc::clone(&self.lifecycle_lock).lock_owned().await;
            self.stop_health_checks().await;
            self.cancel_recovery().await;
            self.cleanup_generation_without_pump(generation).await;
            drop(terminal_operation);
            drop(terminal_teardown);
        }
    }

    /// Schedule the shared controller's per-source translation deadline.
    /// Generation and task ownership reject stale timers. Expiry clears only
    /// pending state; it never changes the subtitle already being displayed.
    pub(super) fn arm_translation_timeout(
        self: &Arc<Self>,
        source: AudioSource,
        generation: u64,
        deadline: Instant,
    ) {
        // Install/cancel under one slot lock. Otherwise a concurrent cancel
        // can run after `spawn` but before the handle is stored, leaving a
        // detached stale handle in the slot even though its ownership id was
        // already invalidated.
        let mut slot = self.lane(source).translation_timeout_task.lock().unwrap();
        if let Some(task) = slot.take() {
            task.abort();
        }
        let task_id = self.next_background_task_id();
        self.lane(source)
            .translation_timeout_task_id
            .store(task_id, Ordering::SeqCst);
        let this = Arc::clone(self);
        let task = tokio::spawn(async move {
            tokio::time::sleep_until(deadline.into()).await;
            let _content = this.subtitle_content_lock.lock().await;
            if !this.is_generation_current(generation)
                || !this.clear_translation_timeout_task_if_id(source, task_id)
            {
                return;
            }
            pipeline_log!("translation pending timed out; clearing");
            let expired = this
                .controller
                .lock()
                .unwrap()
                .expire_translation_pending_from(source, Instant::now());
            if expired {
                this.publish_state();
            }
        });
        *slot = Some(task);
    }

    pub(super) fn cancel_translation_timeout(self: &Arc<Self>) {
        for source in [AudioSource::System, AudioSource::Microphone] {
            self.cancel_source_translation_timeout(source);
        }
    }

    pub(super) fn cancel_source_translation_timeout(self: &Arc<Self>, source: AudioSource) {
        let mut slot = self.lane(source).translation_timeout_task.lock().unwrap();
        self.lane(source)
            .translation_timeout_task_id
            .store(NO_GENERATION, Ordering::SeqCst);
        if let Some(task) = slot.take() {
            task.abort();
        }
    }

    pub(super) fn accepts_event(&self, generation: u64, event: &LiveTranslateServerEvent) -> bool {
        generation_accepts_event(
            self.active_generation.load(Ordering::SeqCst),
            self.stopping_tail_generation.load(Ordering::SeqCst),
            generation,
            event,
        )
    }
}
