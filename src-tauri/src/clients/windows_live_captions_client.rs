//! Local UIA snapshots adapted to the existing bounded recognition event lanes.
//! No PCM, microphone, provider socket, or caption content diagnostics exist here.

use super::provider_events::ProviderEventSender;
use super::recognition_client::RecognitionClientError;
use crate::core::protocols::live_translate::LiveTranslateServerEvent;
use crate::core::windows_caption_snapshots::{CaptionSnapshots, CaptionUpdate};
use crate::windows_live_captions::{self, CaptionSession};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::task::JoinHandle;

#[derive(Default)]
struct Events {
    sender: Option<ProviderEventSender>,
    snapshots: CaptionSnapshots,
}

struct Inner {
    events: Mutex<Events>,
    reader: tokio::sync::Mutex<Option<JoinHandle<()>>>,
    generation: AtomicU64,
    connected_generation: AtomicU64,
    cancel_setup: tokio::sync::watch::Sender<u64>,
    finish: tokio::sync::watch::Sender<u64>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(reader) = self.reader.get_mut().take() {
            reader.abort();
        }
    }
}

#[derive(Clone)]
pub struct WindowsLiveCaptionsClient {
    inner: Arc<Inner>,
}

impl WindowsLiveCaptionsClient {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                events: Mutex::new(Events::default()),
                reader: tokio::sync::Mutex::new(None),
                generation: AtomicU64::new(0),
                connected_generation: AtomicU64::new(0),
                cancel_setup: tokio::sync::watch::channel(0).0,
                finish: tokio::sync::watch::channel(0).0,
            }),
        }
    }

    pub fn set_event_sender(&self, sender: ProviderEventSender) {
        self.inner.events.lock().unwrap().sender = Some(sender);
    }

    pub async fn connect(&self) -> Result<(), RecognitionClientError> {
        self.connect_with_start(windows_live_captions::start())
            .await
    }

    async fn connect_with_start(
        &self,
        start: impl std::future::Future<Output = Result<CaptionSession, String>>,
    ) -> Result<(), RecognitionClientError> {
        let generation = self.disconnect_generation().await;
        self.start_in_generation(start, generation).await
    }

    async fn start_in_generation(
        &self,
        start: impl std::future::Future<Output = Result<CaptionSession, String>>,
        generation: u64,
    ) -> Result<(), RecognitionClientError> {
        let mut cancelled = self.inner.cancel_setup.subscribe();
        if self.inner.generation.load(Ordering::SeqCst) != generation {
            return Err(local_error("windows_live_captions_not_connected"));
        }
        let mut session = tokio::select! {
            result = tokio::time::timeout(Duration::from_secs(8), start) => result
                .map_err(|_| local_error("windows_live_captions_setup_timeout"))?
                .map_err(|error| local_error(safe_label(&error)))?,
            _ = cancelled.changed() => return Err(local_error("windows_live_captions_not_connected")),
        };
        let mut reader = self.inner.reader.lock().await;
        if self.inner.generation.load(Ordering::SeqCst) != generation {
            session.cancel();
            return Err(local_error("windows_live_captions_not_connected"));
        }
        {
            let mut events = self.inner.events.lock().unwrap();
            events.snapshots.reset_baseline();
            emit(
                &self.inner,
                &events,
                generation,
                LiveTranslateServerEvent::SessionCreated,
            )?;
        }
        self.inner
            .connected_generation
            .store(generation, Ordering::SeqCst);
        let mut finish = self.inner.finish.subscribe();
        let weak = Arc::downgrade(&self.inner);
        *reader = Some(tokio::spawn(async move {
            let started = tokio::time::Instant::now();
            let mut timer = tokio::time::interval(Duration::from_millis(100));
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                let received = tokio::select! {
                    result = session.recv() => Some(result),
                    _ = timer.tick() => None,
                    _ = finish.changed() => {
                        let Some(inner) = weak.upgrade() else { break };
                        let mut events = inner.events.lock().unwrap();
                        if inner.generation.load(Ordering::SeqCst) != generation { break; }
                        let updates = events.snapshots.finish();
                        let _ = emit_updates(&inner, &events, generation, updates);
                        let _ = emit(&inner, &events, generation, LiveTranslateServerEvent::SessionFinished);
                        clear_connected_generation(&inner, generation);
                        break;
                    }
                };
                let Some(inner) = weak.upgrade() else { break };
                if inner.generation.load(Ordering::SeqCst) != generation {
                    break;
                }
                let now_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
                let mut events = inner.events.lock().unwrap();
                if inner.generation.load(Ordering::SeqCst) != generation {
                    break;
                }
                let updates = match received {
                    Some(Ok(Some(snapshot))) => events.snapshots.update(&snapshot, now_ms),
                    None => Ok(events.snapshots.tick(now_ms)),
                    Some(Ok(None)) => Err("windows_live_captions_closed"),
                    Some(Err(error)) => Err(safe_label(&error)),
                };
                let result = match updates {
                    Ok(updates) => emit_updates(&inner, &events, generation, updates),
                    Err(label) => {
                        let _ = emit(&inner, &events, generation, error_event(label));
                        Err(local_error(label))
                    }
                };
                if result.is_err() {
                    clear_connected_generation(&inner, generation);
                    break;
                }
            }
            session.cancel();
        }));
        Ok(())
    }

    pub async fn probe(&self) -> Result<(), RecognitionClientError> {
        let support = windows_live_captions::support().await;
        match support.status.as_str() {
            "ready" if support.available => Ok(()),
            "closed" => Err(local_error("windows_live_captions_closed")),
            "setupRequired" => Err(local_error("windows_live_captions_setup_required")),
            "unreadable" => Err(local_error("windows_live_captions_unreadable")),
            _ => Err(local_error("windows_live_captions_unsupported")),
        }
    }

    pub fn ping(&self) -> Result<(), RecognitionClientError> {
        if is_connected(&self.inner) {
            Ok(())
        } else {
            Err(local_error("windows_live_captions_not_connected"))
        }
    }

    pub async fn finish(&self, timeout: Duration) {
        let generation = self.inner.generation.load(Ordering::SeqCst);
        let mut reader_slot = self.inner.reader.lock().await;
        if self.inner.generation.load(Ordering::SeqCst) != generation {
            return;
        }
        self.inner
            .cancel_setup
            .send_modify(|value| *value = value.wrapping_add(1));
        self.inner
            .finish
            .send_modify(|value| *value = value.wrapping_add(1));
        if let Some(mut reader) = reader_slot.take() {
            if tokio::time::timeout(timeout, &mut reader).await.is_err() {
                reader.abort();
                let _ = reader.await;
            }
        }
        let _ = self.inner.generation.compare_exchange(
            generation,
            generation.wrapping_add(1),
            Ordering::SeqCst,
            Ordering::SeqCst,
        );
        clear_connected_generation(&self.inner, generation);
    }

    pub async fn disconnect(&self) {
        self.disconnect_generation().await;
    }

    async fn disconnect_generation(&self) -> u64 {
        self.inner
            .cancel_setup
            .send_modify(|value| *value = value.wrapping_add(1));
        let generation = self
            .inner
            .generation
            .fetch_add(1, Ordering::SeqCst)
            .wrapping_add(1);
        let mut reader_slot = self.inner.reader.lock().await;
        // A newer teardown owns any newer installed reader. An older caller
        // must never abort it or claim its generation when it resumes.
        if self.inner.generation.load(Ordering::SeqCst) == generation {
            if let Some(reader) = reader_slot.take() {
                reader.abort();
                let _ = reader.await;
            }
        }
        clear_connected_generation(&self.inner, generation.wrapping_sub(1));
        generation
    }

    pub async fn clear_content(&self) -> u64 {
        // A fresh native attachment establishes the baseline at Clear. Keeping
        // the old native queue could let a pre-Clear unread snapshot cross it.
        let restart = is_connected(&self.inner);
        let generation = self.disconnect_generation().await;
        let revision = {
            let mut events = self.inner.events.lock().unwrap();
            if self.inner.generation.load(Ordering::SeqCst) != generation {
                return events
                    .sender
                    .as_ref()
                    .map_or(0, ProviderEventSender::content_revision);
            }
            events.snapshots.clear();
            events
                .sender
                .as_ref()
                .map_or(0, ProviderEventSender::advance_content_revision)
        };
        if restart {
            if let Err(error) = self
                .start_in_generation(windows_live_captions::start(), generation)
                .await
            {
                let events = self.inner.events.lock().unwrap();
                let _ = emit(
                    &self.inner,
                    &events,
                    generation,
                    error_event(safe_label(&error.to_string())),
                );
            }
        }
        revision
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

fn is_connected(inner: &Inner) -> bool {
    let generation = inner.generation.load(Ordering::SeqCst);
    generation != 0 && inner.connected_generation.load(Ordering::SeqCst) == generation
}

fn clear_connected_generation(inner: &Inner, generation: u64) {
    let _ = inner.connected_generation.compare_exchange(
        generation,
        0,
        Ordering::SeqCst,
        Ordering::SeqCst,
    );
}

fn emit_updates(
    inner: &Inner,
    events: &Events,
    generation: u64,
    updates: Vec<CaptionUpdate>,
) -> Result<(), RecognitionClientError> {
    for update in updates {
        let event = if update.is_final {
            LiveTranslateServerEvent::SourceUtteranceFinal {
                utterance_id: update.utterance_id,
                text: update.text,
                language: None,
            }
        } else {
            LiveTranslateServerEvent::SourceUtteranceDraft {
                utterance_id: update.utterance_id,
                text: update.text,
                language: None,
            }
        };
        emit(inner, events, generation, event)?;
    }
    Ok(())
}

fn emit(
    inner: &Inner,
    events: &Events,
    generation: u64,
    event: LiveTranslateServerEvent,
) -> Result<(), RecognitionClientError> {
    if let Some(sender) = &events.sender {
        let revision = sender.content_revision();
        sender
            .send_if(event, || {
                inner.generation.load(Ordering::SeqCst) == generation
                    && sender.content_revision() == revision
            })
            .map_err(|_| local_error("windows_live_captions_result_backlog"))?;
    }
    Ok(())
}

fn safe_label(error: &str) -> &'static str {
    match error {
        "windows_live_captions_unsupported" => "windows_live_captions_unsupported",
        "windows_live_captions_closed" => "windows_live_captions_closed",
        "windows_live_captions_setup_required" => "windows_live_captions_setup_required",
        "windows_live_captions_unreadable" | "windows_live_captions_reader_busy" => {
            "windows_live_captions_unreadable"
        }
        "windows_live_captions_snapshot_limit" => "windows_live_captions_snapshot_limit",
        "windows_live_captions_setup_timeout" | "windows_live_captions_timeout" => {
            "windows_live_captions_setup_timeout"
        }
        "windows_live_captions_not_connected" => "windows_live_captions_not_connected",
        "windows_live_captions_result_backlog" => "windows_live_captions_result_backlog",
        _ => "windows_live_captions_read_failed",
    }
}

fn local_error(label: &str) -> RecognitionClientError {
    RecognitionClientError::Windows(label.into())
}
fn error_event(label: &str) -> LiveTranslateServerEvent {
    LiveTranslateServerEvent::Error {
        code: label.into(),
        message: label.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clients::provider_events::provider_event_channel;

    #[tokio::test]
    async fn cancellation_interrupts_pending_setup_and_stale_events_cannot_publish() {
        let client = WindowsLiveCaptionsClient::new();
        let (sender, mut receiver) = provider_event_channel();
        client.set_event_sender(sender);
        let pending_client = client.clone();
        let pending = tokio::spawn(async move {
            pending_client
                .connect_with_start(std::future::pending())
                .await
        });
        tokio::task::yield_now().await;
        let old_generation = client.inner.generation.load(Ordering::SeqCst);
        client.disconnect().await;
        assert_eq!(
            pending.await.unwrap(),
            Err(local_error("windows_live_captions_not_connected"))
        );
        {
            let events = client.inner.events.lock().unwrap();
            emit_updates(
                &client.inner,
                &events,
                old_generation,
                vec![CaptionUpdate {
                    utterance_id: 1,
                    text: "Synthetic stale caption".into(),
                    is_final: true,
                }],
            )
            .unwrap();
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(30), receiver.recv())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn concurrent_teardowns_keep_distinct_tickets_and_stale_connected_state_is_inert() {
        let client = WindowsLiveCaptionsClient::new();
        let reader_guard = client.inner.reader.lock().await;
        let older_client = client.clone();
        let older = tokio::spawn(async move { older_client.disconnect_generation().await });
        tokio::task::yield_now().await;
        let newer_client = client.clone();
        let newer = tokio::spawn(async move { newer_client.disconnect_generation().await });
        tokio::task::yield_now().await;
        drop(reader_guard);
        assert_eq!(older.await.unwrap(), 1);
        assert_eq!(newer.await.unwrap(), 2);
        // A setup that loses its generation after its last check may publish
        // only its old ticket. It cannot make ping report a live connection.
        client.inner.connected_generation.store(1, Ordering::SeqCst);
        assert!(client.ping().is_err());
        client.inner.connected_generation.store(2, Ordering::SeqCst);
        clear_connected_generation(&client.inner, 1);
        assert!(client.ping().is_ok());
        client.disconnect().await;
        assert!(client.ping().is_err());
    }

    #[tokio::test]
    async fn stale_finish_does_not_cancel_newer_setup() {
        let client = WindowsLiveCaptionsClient::new();
        let cancelled = client.inner.cancel_setup.subscribe();
        let reader_guard = client.inner.reader.lock().await;
        let finishing_client = client.clone();
        let finishing = tokio::spawn(async move { finishing_client.finish(Duration::ZERO).await });
        tokio::task::yield_now().await;
        // A newer user intent invalidates the captured finish ticket while
        // that finish is waiting for the installation/teardown boundary.
        client.inner.generation.fetch_add(1, Ordering::SeqCst);
        drop(reader_guard);
        finishing.await.unwrap();
        assert!(!cancelled.has_changed().unwrap());
        assert_eq!(client.inner.generation.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn external_error_bodies_never_become_user_or_diagnostic_content() {
        assert_eq!(
            safe_label("Synthetic private caption or HRESULT body"),
            "windows_live_captions_read_failed"
        );
        assert_eq!(
            safe_label("windows_live_captions_closed"),
            "windows_live_captions_closed"
        );
        assert_eq!(
            safe_label("windows_live_captions_timeout"),
            "windows_live_captions_setup_timeout"
        );
        assert_eq!(
            safe_label("windows_live_captions_reader_busy"),
            "windows_live_captions_unreadable"
        );
    }
}
