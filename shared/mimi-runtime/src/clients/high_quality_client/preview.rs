//! Replaceable draft timers, preview ownership and latest-candidate handoff.
//!
//! Draft revisions replace waiting candidates; they do not transfer durable
//! final ownership or refund a request that has already started.

use super::{
    abort_task, next_nonzero, pipeline_log, source_draft_event, Arc, DraftTimerKind, Duration,
    HighQualityTranslationClient, LiveTranslateServerEvent, Ordering, PartialHandler,
    PipelineState, PreviewCancellationReason, PreviewCandidate, QwenMTClientError, TaskSlot,
    TranslationEvidenceIdentity, TranslationLatency, TranslationLatencyKind, TranslationRecovery,
    TranslationRecoveryReason, TranslationWorkOwner, MAX_PREVIEW_REQUEST_AGE,
};

impl HighQualityTranslationClient {
    pub(super) async fn schedule_draft_finalization(&self, revision: u64) {
        let mut inner = self.state.lock().await;
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
            // Briefly collect an opening phrase, but never wait indefinitely
            // for a length threshold or a missing provider final. Later work
            // retains the configured cadence and the same non-resetting timer.
            let opening = inner.preview_task.is_none()
                && !inner.preview_request_pacer.has_previewed_utterance();
            let maximum_delay = if opening {
                self.maximum_wait_delay.max(Duration::from_secs(2))
            } else {
                self.maximum_wait_delay
            };
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

    pub(super) async fn handle_draft_timer(
        &self,
        kind: DraftTimerKind,
        timer_id: u64,
        scheduled_revision: u64,
    ) {
        let preview = {
            let mut inner = self.state.lock().await;
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
            // Do not replace a readable complete sentence with the first tiny
            // fragment of the next one. Honor the existing opening threshold;
            // once preview work starts, shorter ASR corrections remain eligible.
            // Both timers still translate the whole draft, including its tail.
            let opening = inner.preview_task.is_none()
                && !inner.preview_request_pacer.has_previewed_utterance();
            let text = inner
                .committer
                .preview_latest_draft(opening && matches!(kind, DraftTimerKind::Stable));
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

    pub(super) async fn start_preview(
        &self,
        text: String,
        language: Option<String>,
        revision: u64,
    ) {
        if !crate::core::models::subtitle_text_within_limit(&text) {
            self.emit(LiveTranslateServerEvent::text_limit_error());
            return;
        }
        let mut inner = self.state.lock().await;
        self.start_preview_locked(&mut inner, text, language, revision);
    }

    // Synchronous under the state lock so completion can hand off to exactly
    // one newest candidate without a recursive async spawn/cleanup chain.
    pub(super) fn start_preview_locked(
        &self,
        inner: &mut PipelineState,
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

    pub(super) async fn run_preview(
        &self,
        preview_id: u64,
        text: String,
        language: Option<String>,
    ) {
        if !self.preview_is_current(preview_id) {
            return;
        }
        let source_utterance_id = self.state.lock().await.current_source_utterance_id;
        if self
            .target_language
            .matches_reported_asr(language.as_deref())
        {
            {
                let inner = self.state.lock().await;
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
                    let inner = self.state.lock().await;
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
                    let mut inner = self.state.lock().await;
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

    pub(super) fn emit_preview(&self, preview_id: u64, event: LiveTranslateServerEvent) {
        let _ = self
            .events
            .send_if(event, || self.preview_is_current(preview_id));
    }

    pub(super) fn preview_partial_handler(&self, preview_id: u64) -> PartialHandler {
        let client = self.clone();
        Arc::new(move |partial| {
            client.emit_preview(
                preview_id,
                LiveTranslateServerEvent::TranslationDraft(partial),
            );
        })
    }

    pub(super) fn preview_is_current(&self, preview_id: u64) -> bool {
        self.mt_work_allowed.load(Ordering::SeqCst)
            && self.preview_epoch.load(Ordering::SeqCst) == preview_id
    }

    pub(super) fn advance_preview_epoch(&self) -> u64 {
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

    pub(super) async fn clear_preview_task(&self, preview_id: u64, resume_latest: bool) {
        let mut inner = self.state.lock().await;
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

    pub(super) fn finish_preview_http(&self, inner: &mut PipelineState, request_id: u64) {
        if inner.preview_http_pending == Some(request_id) {
            inner.preview_http_pending = None;
            self.emit_preview(
                request_id,
                LiveTranslateServerEvent::PreviewTranslationFinished { request_id },
            );
        }
    }

    pub(super) fn cancel_preview(
        &self,
        inner: &mut PipelineState,
        reason: PreviewCancellationReason,
    ) {
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

    pub(super) async fn cancel_replaceable_work(&self) {
        let mut inner = self.state.lock().await;
        abort_task(&mut inner.draft_stability_task);
        abort_task(&mut inner.draft_maximum_wait_task);
        self.cancel_preview(&mut inner, PreviewCancellationReason::Stop);
        self.advance_preview_epoch();
        next_nonzero(&mut inner.draft_revision);
    }

    pub(super) async fn reset_draft_state(&self) {
        let mut inner = self.state.lock().await;
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
}
