//! Bounded durable final admission, serial translation and finish-tail draining.
//!
//! This lane retains source identity and enqueue deadlines. It uses the same
//! request budget as previews, and only its current worker may publish a final.

use super::{
    abort_task, clear_task_if_id, next_nonzero, pipeline_log, source_draft_event,
    translation_policy, Arc, Duration, EnqueueOutcome, FinalBoundary, HighQualityTranslationClient,
    LiveTranslateServerEvent, MeasuredTranslation, PartialHandler, PreviewCancellationReason,
    QwenMTClientError, TaskSlot, TranslationEvidenceIdentity, TranslationLatency,
    TranslationLatencyKind, TranslationRecoveryReason, TranslationRequest, TranslationWorkOwner,
    MAX_FINAL_QUEUE_DEPTH, MAX_FINAL_REQUEST_AGE, OVERLOAD_ERROR_CODE, OVERLOAD_ERROR_MESSAGE,
};

impl HighQualityTranslationClient {
    pub(super) async fn enqueue_final(
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
            let mut inner = self.state.lock().await;
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

    pub(super) async fn start_final_worker_if_needed(&self) {
        let mut inner = self.state.lock().await;
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

    pub(super) async fn run_final_worker(&self, worker_id: u64) {
        loop {
            let request = {
                let mut inner = self.state.lock().await;
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
                    self.state.lock().await.final_queue.len()
                );
                self.emit_content(
                    request.content_revision,
                    LiveTranslateServerEvent::TranslationStarted,
                );
            }
            {
                let inner = self.state.lock().await;
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
                        let mut inner = self.state.lock().await;
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
                        let mut inner = self.state.lock().await;
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
                            self.state.lock().await.final_queue.len()
                        );
                    } else {
                        pipeline_log!(
                            "mt final completed boundary={} requestMs={} remaining={} sourceLength={} translationLength={}",
                            request.boundary.label(),
                            translation.request_ms.unwrap_or_default(),
                            self.state.lock().await.final_queue.len(),
                            request.text.chars().count(),
                            translation.text.chars().count()
                        );
                    }
                }
                Err(error) => {
                    let should_emit = {
                        let mut inner = self.state.lock().await;
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

    pub(super) async fn fail_final_worker_overload(&self, worker_id: u64, queue_age: Duration) {
        let should_emit = {
            let mut inner = self.state.lock().await;
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

    pub(super) async fn resume_pending_preview_if_final_lane_idle(&self) {
        let revision = {
            let inner = self.state.lock().await;
            (!inner.pipeline_failed
                && !inner.final_lane_busy()
                && inner.committer.has_pending_text())
            .then_some(inner.draft_revision)
        };
        if let Some(revision) = revision {
            self.schedule_draft_finalization(revision).await;
        }
    }

    pub(super) fn emit_overload_error(&self) {
        self.emit(LiveTranslateServerEvent::Error {
            code: OVERLOAD_ERROR_CODE.into(),
            message: OVERLOAD_ERROR_MESSAGE.into(),
        });
    }

    pub(super) fn handle_translation_failure(&self, error: &QwenMTClientError) {
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

    pub(super) async fn flush_pending_draft(&self) {
        let pending = {
            let mut inner = self.state.lock().await;
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

    pub(super) async fn cancel_final_translations(&self) {
        let mut inner = self.state.lock().await;
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

    pub(super) async fn wait_for_final_translations(&self, timeout: Duration) {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let has_worker = self.state.lock().await.final_worker.is_some();
            if !has_worker || tokio::time::Instant::now() >= deadline {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}
