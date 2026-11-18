//! Shared preview/final request ownership, quota spacing, cooldown and retries.
//!
//! Budget claims happen under the pipeline-state lock before HTTP begins.
//! Cancellation and reconnect carry already consumed starts and suppression;
//! the translation deadline remains the original caller's deadline.

use super::{
    pipeline_log, source_draft_event, translation_policy, Arc, HighQualityTranslationClient,
    LiveTranslateServerEvent, MTCooldown, MTRequestBudget, MeasuredTranslation, Ordering,
    PartialHandler, PipelineState, PreviewCandidate, QwenMTClientError, SourceLanguage,
    TextTranslationClient, TranslationEvidenceIdentity, TranslationRecovery,
    TranslationRecoveryReason, TranslationWorkOwner,
};

impl HighQualityTranslationClient {
    pub(crate) async fn suspend_request_budget(&self) -> MTRequestBudget {
        // Stop new starts before sampling; any slot already claimed is charged
        // in the same pipeline-state lock before the actual HTTP await.
        self.mt_work_allowed.store(false, Ordering::SeqCst);
        self.state
            .lock()
            .await
            .preview_request_pacer
            .export_budget()
    }

    pub(crate) async fn restore_request_budget(&self, budget: MTRequestBudget) {
        let mut inner = self.state.lock().await;
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

    pub(super) fn translation_work_is_current(
        &self,
        inner: &PipelineState,
        owner: TranslationWorkOwner,
    ) -> bool {
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

    pub(super) async fn wait_for_mt_request_slot(
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
                let mut inner = self.state.lock().await;
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

    pub(super) async fn translate_with_retry(
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
        // Apple also needs the recognizer's actual source for explicit routes:
        // otherwise a configured same-language pair can bypass translation of
        // an utterance that the recognizer reported in a different language.
        let source_override = if self.source_language == SourceLanguage::Automatic
            || matches!(self.mt.as_ref(), TextTranslationClient::Apple(_))
        {
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
                    let mut inner = self.state.lock().await;
                    if !self.translation_work_is_current(&inner, owner) {
                        return Err(QwenMTClientError::RequestTimedOut);
                    }
                    inner.mt_cooldown = None;
                    inner.mt_failure_streak = 0;
                    inner.preview_request_pacer.set_shared_cooldown(None);
                    // Apple can return the source without invoking translation.
                    // Preserve that result but do not report it as request work.
                    let request_ms = match self.mt.as_ref() {
                        TextTranslationClient::Apple(client)
                            if client.passthrough(text, source_override)?.is_some() =>
                        {
                            None
                        }
                        _ => Some(first_request_at.elapsed().as_millis() as u64),
                    };
                    return Ok(MeasuredTranslation {
                        text: translation,
                        request_ms,
                    });
                }
                Err(error) => {
                    let Some(reason) = error.recovery_reason() else {
                        return Err(error);
                    };
                    let class = error.retry_class();
                    let (delay, failure_streak) = {
                        let mut inner = self.state.lock().await;
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
}
