//! Recognition event identity, authoritative source boundaries and bridge teardown.
//!
//! These operations share the pipeline state and content-operation gate with
//! Clear. An accepted recognizer finish acknowledges events only after this
//! bridge has consumed them; the public lifecycle waits on that acknowledgement.

use super::{
    abort_task, next_nonzero, oneshot, pipeline_log, source_draft_event, trim, Duration,
    FinalBoundary, HighQualityTranslationClient, LiveTranslateServerEvent, Ordering,
    PreviewCancellationReason, PreviewCandidate, ProviderEventReceiver,
};

impl HighQualityTranslationClient {
    pub(super) async fn handle_asr_event(&self, event: LiveTranslateServerEvent) {
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
                    let mut inner = self.state.lock().await;
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

    pub(super) async fn prepare_identified_draft(&self, source_utterance_id: u64) -> bool {
        if source_utterance_id == 0 {
            return false;
        }
        let mut inner = self.state.lock().await;
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

    pub(super) async fn handle_server_final(
        &self,
        text: String,
        language: Option<String>,
        source_utterance_id: Option<u64>,
    ) {
        let text = trim(&text);
        if text.is_empty() {
            self.retire_empty_server_final(source_utterance_id).await;
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
            self.state.lock().await.final_queue.len()
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

    pub(super) async fn retire_empty_server_final(&self, source_utterance_id: Option<u64>) {
        let Some(id) = source_utterance_id.filter(|id| *id > 0) else {
            return;
        };
        let mut inner = self.state.lock().await;
        if inner
            .last_source_utterance_id
            .is_some_and(|last| id <= last)
        {
            return;
        }
        inner.last_source_utterance_id = Some(id);
        // A delayed empty completion cannot withdraw a newer source or touch
        // ordered durable work. Only this source's current draft is revoked.
        if inner.current_source_utterance_id == Some(id) {
            abort_task(&mut inner.draft_stability_task);
            abort_task(&mut inner.draft_maximum_wait_task);
            self.cancel_preview(&mut inner, PreviewCancellationReason::SourceFinal);
            self.advance_preview_epoch();
            inner.committer.reset();
            inner.latest_draft_language = None;
            inner.current_source_utterance_id = None;
            next_nonzero(&mut inner.draft_revision);
            inner.preview_request_pacer.finish_utterance();
        }
        self.emit(LiveTranslateServerEvent::SubtitlePreviewWithdrawn {
            source_utterance_id: id,
        });
    }

    pub(super) async fn prepare_server_final(
        &self,
        text: &str,
        source_utterance_id: Option<u64>,
    ) -> Option<u64> {
        if !crate::core::models::subtitle_text_within_limit(text) {
            return None;
        }
        let mut inner = self.state.lock().await;
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

    pub(super) async fn install_asr_bridge(&self, mut receiver: ProviderEventReceiver) {
        receiver.enable_recognition_finish_drain();
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

    pub(super) async fn wait_for_asr_bridge_finish(&self, timeout: Duration) -> bool {
        let finish_ack = self.take_asr_bridge_finish_ack().await;
        let Some(finish_ack) = finish_ack else {
            return false;
        };
        matches!(tokio::time::timeout(timeout, finish_ack).await, Ok(Ok(())))
    }

    pub(super) async fn take_asr_bridge_finish_ack(&self) -> Option<oneshot::Receiver<()>> {
        self.asr_bridge.lock().await.finish_ack.take()
    }

    pub(super) async fn disconnect_asr_bridge(&self) {
        let mut bridge = self.asr_bridge.lock().await;
        bridge.finish_ack = None;
        if let Some(task) = bridge.task.take() {
            task.abort();
        }
    }
}
