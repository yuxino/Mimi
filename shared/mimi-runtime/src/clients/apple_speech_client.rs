//! Native Apple recognition adapter. Each capture source owns a separate session.
//! Only bounded sentence identities are retained; text remains in the shared
//! preview/final event lanes and is never logged here.

use super::provider_events::ProviderEventSender;
use super::recognition_client::RecognitionClientError;
use crate::apple_speech::{self, AppleSpeechError, AppleSpeechEvent, AppleSpeechSession};
use crate::core::models::SourceLanguage;
use crate::core::protocols::live_translate::LiveTranslateServerEvent;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::task::JoinHandle;

const MAX_TRACKED_RANGES: usize = 64;
const SETUP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Default)]
struct SentenceRanges {
    next_id: u64,
    ranges: BTreeMap<u64, (u64, bool)>,
    retired_before: Option<u64>,
    clear_before_ms: Option<f64>,
}

impl SentenceRanges {
    fn admit(&mut self, event: &AppleSpeechEvent) -> Result<Option<u64>, &'static str> {
        if !event.start_ms.is_finite()
            || !event.end_ms.is_finite()
            || event.start_ms < 0.0
            || event.end_ms < event.start_ms
        {
            return Err("apple_speech_invalid_result");
        }
        if event.text.trim().is_empty()
            || self
                .clear_before_ms
                .is_some_and(|cutoff| event.start_ms < cutoff)
        {
            return Ok(None);
        }
        let start = (event.start_ms * 1_000.0).round() as u64;
        if self.retired_before.is_some_and(|cutoff| start <= cutoff) {
            return Ok(None);
        }
        if !self.ranges.contains_key(&start) {
            if self.ranges.len() >= MAX_TRACKED_RANGES {
                // Retire only a finished oldest sentence. Unbounded volatile
                // ranges fail explicitly instead of discarding durable finals.
                let (&oldest, &(_, finished)) = self.ranges.first_key_value().unwrap();
                if !finished {
                    return Err("apple_speech_result_backlog");
                }
                self.ranges.remove(&oldest);
                self.retired_before = Some(oldest);
            }
            self.next_id = self.next_id.saturating_add(1);
            self.ranges.insert(start, (self.next_id, false));
        }
        let (id, finished) = self.ranges.get_mut(&start).unwrap();
        if *finished {
            return Ok(None);
        }
        *finished = event.is_final;
        Ok(Some(*id))
    }

    fn clear(&mut self, sent_pcm_bytes: u64) {
        // PCM is mono 16 kHz / 16 bit. The cutoff also covers audio submitted
        // before Clear whose first callback has not arrived yet.
        self.clear_before_ms = Some(sent_pcm_bytes as f64 / 32.0);
        self.ranges.clear();
    }
}

#[derive(Default)]
struct EventState {
    sender: Option<ProviderEventSender>,
    ranges: SentenceRanges,
}

#[derive(Default)]
struct Lifecycle {
    session: Option<AppleSpeechSession>,
    reader: Option<JoinHandle<()>>,
}

struct Inner {
    source: SourceLanguage,
    lifecycle: tokio::sync::Mutex<Lifecycle>,
    events: Mutex<EventState>,
    generation: AtomicU64,
    sent_pcm_bytes: AtomicU64,
    connected: AtomicBool,
    finishing: AtomicBool,
    setup_cancel: tokio::sync::watch::Sender<u64>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        let lifecycle = self.lifecycle.get_mut();
        if let Some(reader) = lifecycle.reader.take() {
            reader.abort();
        }
        if let Some(mut session) = lifecycle.session.take() {
            session.cancel();
        }
    }
}

#[derive(Clone)]
pub struct AppleSpeechClient {
    inner: Arc<Inner>,
}

impl AppleSpeechClient {
    pub fn new(source: SourceLanguage) -> Self {
        Self {
            inner: Arc::new(Inner {
                source,
                lifecycle: tokio::sync::Mutex::new(Lifecycle::default()),
                events: Mutex::new(EventState::default()),
                generation: AtomicU64::new(0),
                sent_pcm_bytes: AtomicU64::new(0),
                connected: AtomicBool::new(false),
                finishing: AtomicBool::new(false),
                setup_cancel: tokio::sync::watch::channel(0).0,
            }),
        }
    }

    pub fn set_event_sender(&self, sender: ProviderEventSender) {
        self.inner.events.lock().unwrap().sender = Some(sender);
    }

    pub async fn connect(&self) -> Result<(), RecognitionClientError> {
        self.connect_with_start(async {
            // This native check happens before session_manager starts capture.
            // Starting never requests a language download or falls back to cloud.
            let locale = crate::apple_speech_support::locale_for_source(self.inner.source)
                .await
                .map_err(RecognitionClientError::Apple)?;
            apple_speech::start(&locale).await.map_err(start_error)
        })
        .await
    }

    async fn connect_with_start(
        &self,
        start: impl std::future::Future<
            Output = Result<
                (AppleSpeechSession, apple_speech::AppleSpeechEvents),
                RecognitionClientError,
            >,
        >,
    ) -> Result<(), RecognitionClientError> {
        self.disconnect().await;
        let generation = self.inner.generation.load(Ordering::SeqCst);
        let mut cancel = self.inner.setup_cancel.subscribe();
        let (session, mut events) = tokio::select! {
            result = tokio::time::timeout(SETUP_TIMEOUT, start) => result
                .map_err(|_| local_error("apple_speech_setup_timeout"))??,
            _ = cancel.changed() => return Err(local_error("apple_speech_not_connected")),
        };
        let mut lifecycle = self.inner.lifecycle.lock().await;
        if self.inner.generation.load(Ordering::SeqCst) != generation {
            return Err(local_error("apple_speech_not_connected"));
        }
        self.inner.sent_pcm_bytes.store(0, Ordering::SeqCst);
        self.inner.finishing.store(false, Ordering::SeqCst);
        {
            let mut state = self.inner.events.lock().unwrap();
            state.ranges = SentenceRanges::default();
            if let Some(sender) = &state.sender {
                sender
                    .send_if(LiveTranslateServerEvent::SessionCreated, || {
                        self.inner.generation.load(Ordering::SeqCst) == generation
                    })
                    .map_err(|_| local_error("apple_speech_result_backlog"))?;
            }
        }
        self.inner.connected.store(true, Ordering::SeqCst);
        lifecycle.session = Some(session);
        let weak = Arc::downgrade(&self.inner);
        lifecycle.reader = Some(tokio::spawn(async move {
            loop {
                let result = events.recv().await;
                let Some(inner) = weak.upgrade() else { break };
                if inner.generation.load(Ordering::SeqCst) != generation {
                    break;
                }
                let mut state = inner.events.lock().unwrap();
                let event = match result {
                    Ok(Some(result)) => match state.ranges.admit(&result) {
                        Ok(Some(utterance_id)) => {
                            let language = Some(inner.source.raw_value().to_owned());
                            if result.is_final {
                                LiveTranslateServerEvent::SourceUtteranceFinal {
                                    utterance_id,
                                    text: result.text,
                                    language,
                                }
                            } else {
                                LiveTranslateServerEvent::SourceUtteranceDraft {
                                    utterance_id,
                                    text: result.text,
                                    language,
                                }
                            }
                        }
                        Ok(None) => continue,
                        Err(label) => error_event(label),
                    },
                    Ok(None) if inner.finishing.load(Ordering::SeqCst) => {
                        LiveTranslateServerEvent::SessionFinished
                    }
                    Ok(None) | Err(_) => error_event("apple_speech_recognition_failed"),
                };
                let terminal = matches!(
                    event,
                    LiveTranslateServerEvent::SessionFinished
                        | LiveTranslateServerEvent::Error { .. }
                );
                if terminal {
                    inner.connected.store(false, Ordering::SeqCst);
                }
                if let Some(sender) = &state.sender {
                    // clear_content and event admission share this state lock,
                    // so an old callback cannot cross the revision boundary.
                    let revision = sender.content_revision();
                    if sender
                        .send_if(event, || {
                            inner.generation.load(Ordering::SeqCst) == generation
                                && sender.content_revision() == revision
                        })
                        .is_err()
                    {
                        inner.connected.store(false, Ordering::SeqCst);
                        break;
                    }
                }
                if terminal {
                    break;
                }
            }
        }));
        Ok(())
    }

    pub async fn send_audio(&self, pcm: &[u8]) -> Result<(), RecognitionClientError> {
        if !self.inner.connected.load(Ordering::SeqCst)
            || self.inner.finishing.load(Ordering::SeqCst)
        {
            return Err(local_error("apple_speech_not_connected"));
        }
        let lifecycle = self.inner.lifecycle.lock().await;
        let _content = self.inner.events.lock().unwrap();
        lifecycle
            .session
            .as_ref()
            .ok_or_else(|| local_error("apple_speech_not_connected"))?
            .send_pcm(pcm)
            .map_err(|_| local_error("apple_speech_audio_failed"))?;
        self.inner
            .sent_pcm_bytes
            .fetch_add(pcm.len() as u64, Ordering::SeqCst);
        Ok(())
    }

    pub fn ping(&self) -> Result<(), RecognitionClientError> {
        if self.inner.connected.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(local_error("apple_speech_not_connected"))
        }
    }

    pub async fn finish(&self, timeout: Duration) {
        self.inner.finishing.store(true, Ordering::SeqCst);
        self.inner
            .setup_cancel
            .send_modify(|value| *value = value.wrapping_add(1));
        let deadline = tokio::time::Instant::now() + timeout;
        let Ok(mut lifecycle) =
            tokio::time::timeout_at(deadline, self.inner.lifecycle.lock()).await
        else {
            self.inner.generation.fetch_add(1, Ordering::SeqCst);
            self.inner.connected.store(false, Ordering::SeqCst);
            return;
        };
        if let Some(session) = lifecycle.session.as_mut() {
            let _ = tokio::time::timeout_at(deadline, session.finish()).await;
        }
        if let Some(mut reader) = lifecycle.reader.take() {
            if tokio::time::timeout_at(deadline, &mut reader)
                .await
                .is_err()
            {
                reader.abort();
                let _ = reader.await;
            }
        }
        if let Some(mut session) = lifecycle.session.take() {
            session.cancel();
        }
        if self.inner.connected.swap(false, Ordering::SeqCst) {
            if let Some(sender) = &self.inner.events.lock().unwrap().sender {
                let _ = sender.send(error_event("apple_speech_finalize_timeout"));
            }
        }
        self.inner.generation.fetch_add(1, Ordering::SeqCst);
    }

    pub async fn disconnect(&self) {
        self.inner
            .setup_cancel
            .send_modify(|value| *value = value.wrapping_add(1));
        self.inner.generation.fetch_add(1, Ordering::SeqCst);
        self.inner.connected.store(false, Ordering::SeqCst);
        let mut lifecycle = self.inner.lifecycle.lock().await;
        if let Some(reader) = lifecycle.reader.take() {
            reader.abort();
            let _ = reader.await;
        }
        if let Some(mut session) = lifecycle.session.take() {
            session.cancel();
        }
    }

    pub fn clear_content(&self) -> u64 {
        let mut state = self.inner.events.lock().unwrap();
        state
            .ranges
            .clear(self.inner.sent_pcm_bytes.load(Ordering::SeqCst));
        state
            .sender
            .as_ref()
            .map_or(0, ProviderEventSender::advance_content_revision)
    }

    pub fn content_revision(&self) -> u64 {
        self.inner
            .events
            .lock()
            .unwrap()
            .sender
            .as_ref()
            .map_or(0, ProviderEventSender::content_revision)
    }
}

fn local_error(label: &str) -> RecognitionClientError {
    RecognitionClientError::Apple(label.to_owned())
}

fn start_error(error: AppleSpeechError) -> RecognitionClientError {
    local_error(match error {
        AppleSpeechError::Unavailable => "apple_speech_unavailable",
        AppleSpeechError::InvalidLocale => "apple_speech_language_unsupported",
        AppleSpeechError::AssetsNotInstalled => "apple_speech_assets_missing",
        AppleSpeechError::AssetsDownloading => "apple_speech_preparing",
        AppleSpeechError::StatusUnavailable => "apple_speech_status_failed",
        AppleSpeechError::ServiceUnavailable => "apple_speech_service_unavailable",
        _ => "apple_speech_start_failed",
    })
}

fn error_event(label: &str) -> LiveTranslateServerEvent {
    LiveTranslateServerEvent::Error {
        code: label.to_owned(),
        message: label.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_readiness_races_keep_actionable_resource_errors() {
        for (error, expected) in [
            (
                AppleSpeechError::AssetsNotInstalled,
                "apple_speech_assets_missing",
            ),
            (
                AppleSpeechError::AssetsDownloading,
                "apple_speech_preparing",
            ),
            (
                AppleSpeechError::StatusUnavailable,
                "apple_speech_status_failed",
            ),
            (
                AppleSpeechError::ServiceUnavailable,
                "apple_speech_service_unavailable",
            ),
            (
                AppleSpeechError::Native {
                    domain: "private/context".into(),
                    code: 9,
                },
                "apple_speech_start_failed",
            ),
        ] {
            assert!(
                matches!(start_error(error), RecognitionClientError::Apple(label) if label == expected)
            );
        }
    }

    fn result(start_ms: f64, text: &str, is_final: bool) -> AppleSpeechEvent {
        AppleSpeechEvent {
            text: text.into(),
            start_ms,
            end_ms: start_ms + 500.0,
            is_final,
        }
    }

    #[test]
    fn apple_sentence_drafts_replace_and_final_is_admitted_once() {
        let mut ranges = SentenceRanges::default();
        assert_eq!(ranges.admit(&result(0.0, "A", false)), Ok(Some(1)));
        assert_eq!(
            ranges.admit(&result(0.0, "A corrected sentence.", false)),
            Ok(Some(1))
        );
        assert_eq!(
            ranges.admit(&result(0.0, "A corrected sentence.", true)),
            Ok(Some(1))
        );
        assert_eq!(ranges.admit(&result(0.0, "Duplicate", true)), Ok(None));
        assert_eq!(ranges.admit(&result(0.0, "Late draft", false)), Ok(None));
        assert_eq!(ranges.admit(&result(1000.0, "Next", true)), Ok(Some(2)));
    }

    #[test]
    fn apple_clear_rejects_unseen_inflight_audio_and_preserves_identity_sequence() {
        let mut ranges = SentenceRanges::default();
        assert_eq!(ranges.admit(&result(0.0, "Old draft", false)), Ok(Some(1)));
        ranges.clear(32_000);
        assert_eq!(ranges.admit(&result(0.0, "Old final", true)), Ok(None));
        assert_eq!(
            ranges.admit(&result(500.0, "Unseen old audio", true)),
            Ok(None)
        );
        assert_eq!(
            ranges.admit(&result(1000.0, "New audio", true)),
            Ok(Some(2))
        );
    }

    #[test]
    fn apple_range_memory_is_bounded_and_retired_finals_cannot_reappear() {
        let mut ranges = SentenceRanges::default();
        for index in 0..200 {
            assert_eq!(
                ranges.admit(&result(index as f64 * 1000.0, "Sentence", true)),
                Ok(Some(index + 1))
            );
            assert!(ranges.ranges.len() <= MAX_TRACKED_RANGES);
        }
        assert_eq!(ranges.admit(&result(0.0, "Late old final", true)), Ok(None));
        let mut invalid = result(0.0, "Bad time", true);
        invalid.start_ms = f64::NAN;
        assert_eq!(ranges.admit(&invalid), Err("apple_speech_invalid_result"));
    }

    #[tokio::test]
    async fn apple_finish_cancels_inflight_setup_without_waiting_for_setup_timeout() {
        let client = AppleSpeechClient::new(SourceLanguage::English);
        let pending = client.clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let connect = tokio::spawn(async move {
            pending
                .connect_with_start(async {
                    let _ = started_tx.send(());
                    std::future::pending().await
                })
                .await
        });
        started_rx.await.unwrap();
        tokio::time::timeout(
            Duration::from_millis(100),
            client.finish(Duration::from_millis(25)),
        )
        .await
        .unwrap();
        let result = tokio::time::timeout(Duration::from_millis(100), connect)
            .await
            .unwrap()
            .unwrap();
        assert!(result.is_err());
        assert!(client.ping().is_err());
    }

    #[tokio::test]
    async fn apple_idle_finish_is_bounded_and_two_sources_keep_independent_revisions() {
        let a = AppleSpeechClient::new(SourceLanguage::English);
        let b = AppleSpeechClient::new(SourceLanguage::English);
        let (sender_a, _receiver_a) = super::super::provider_events::provider_event_channel();
        let (sender_b, _receiver_b) = super::super::provider_events::provider_event_channel();
        a.set_event_sender(sender_a);
        b.set_event_sender(sender_b);
        assert_eq!(a.clear_content(), 1);
        assert_eq!(b.content_revision(), 0);
        tokio::time::timeout(Duration::from_millis(100), a.finish(Duration::ZERO))
            .await
            .unwrap();
        assert!(a.ping().is_err());
        assert!(b.ping().is_err());
    }
}
