//! Bounded audio send pipeline with newest-drop buffering, finite graceful
//! drain, throttled level diagnostics, and a single fell-behind error signal.

use crate::core::diagnostics::milliseconds;
use crate::core::pending_pcm::{PendingPcmGate, PendingPcmGuard};
use crate::pipeline_log;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};

const QUEUE_CAPACITY: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AudioPipelineFailure {
    #[error("The audio transport stopped unexpectedly.")]
    TransportStopped,
}

impl AudioPipelineFailure {
    pub fn diagnostic_label(self) -> &'static str {
        match self {
            Self::TransportStopped => "audio_pipeline.transport_stopped",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AudioIngressError {
    #[error("The bounded audio queue is full.")]
    Backpressure,
    #[error("The audio pipeline is closed.")]
    Closed,
}

type AudioRedirect = Arc<dyn Fn(PendingAudio) -> Result<(), AudioIngressError> + Send + Sync>;

/// Cloneable, synchronous ingress for native audio callbacks. `try_send`
/// performs no await and holds no mutex; a full queue rejects the newest
/// buffer and permanently closes this generation's ingress.
#[derive(Clone)]
pub struct AudioIngress {
    tx: mpsc::Sender<PendingAudio>,
    failed: Arc<AtomicBool>,
    accepting: Arc<AtomicBool>,
    progress: Arc<SendProgress>,
    pending_pcm: PendingPcmGate,
    redirect: Option<AudioRedirect>,
}

// The guard travels with the buffer, including while it has left the bounded
// queue and waits inside the send future. Queue rejection/drop and task
// cancellation release it without a separate decrement or reset path.
pub struct PendingAudio {
    pub data: Vec<u8>,
    pub pending: Option<PendingPcmGuard>,
    pub captured_at: Instant,
}

impl AudioIngress {
    pub fn try_send(&self, data: Vec<u8>) -> Result<(), AudioIngressError> {
        self.send(data, true)
    }

    /// DSP output already has an observation at the native capture boundary.
    /// Its reduced amplitude must not replace the raw microphone activity.
    pub fn try_send_processed(&self, data: Vec<u8>) -> Result<(), AudioIngressError> {
        self.send(data, false)
    }

    pub fn redirected(
        &self,
        redirect: impl Fn(PendingAudio) -> Result<(), AudioIngressError> + Send + Sync + 'static,
    ) -> Self {
        let mut ingress = self.clone();
        ingress.redirect = Some(Arc::new(redirect));
        ingress
    }

    pub fn acquire_pending(&self) -> PendingPcmGuard {
        self.pending_pcm.acquire()
    }

    pub fn enqueue_packet(&self, packet: PendingAudio) -> Result<(), AudioIngressError> {
        if !self.accepting.load(Ordering::SeqCst) {
            return Err(AudioIngressError::Closed);
        }
        self.tx.try_send(packet).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => AudioIngressError::Backpressure,
            mpsc::error::TrySendError::Closed(_) => AudioIngressError::Closed,
        })
    }

    fn send(&self, data: Vec<u8>, observe_capture: bool) -> Result<(), AudioIngressError> {
        if !self.accepting.load(Ordering::SeqCst) {
            return Err(AudioIngressError::Closed);
        }
        let has_data = !data.is_empty();
        let pending = has_data.then(|| self.pending_pcm.acquire());
        let has_sound = peak_pcm16_sample(&data) > 32;
        let captured_at = Instant::now();
        let packet = PendingAudio {
            data,
            pending,
            captured_at,
        };
        let result = match &self.redirect {
            Some(redirect) => redirect(packet),
            None => self.enqueue_packet(packet),
        };
        // A DSP stage owns its capture ingress lifetime separately. Closing
        // that ingress must still allow its already accepted output to drain
        // into this provider queue (including late native callback clones).
        if self.redirect.is_some() && result.is_err() {
            return result;
        }
        match result {
            Ok(()) => {
                if has_data && observe_capture {
                    self.progress.captured(has_sound, captured_at);
                }
                Ok(())
            }
            Err(AudioIngressError::Backpressure) => {
                self.accepting.store(false, Ordering::SeqCst);
                if !self.failed.swap(true, Ordering::SeqCst) {
                    let completed_ago_ms = self.progress.completed_ago_ms(Instant::now());
                    pipeline_log!(
                        "audio queue full capacity={} hasCompletedSend={} lastSendCompletedAgoMs={}",
                        QUEUE_CAPACITY,
                        completed_ago_ms.is_some(),
                        completed_ago_ms.unwrap_or(0)
                    );
                    Err(AudioIngressError::Backpressure)
                } else {
                    Err(AudioIngressError::Closed)
                }
            }
            Err(AudioIngressError::Closed) => {
                self.accepting.store(false, Ordering::SeqCst);
                self.failed.store(true, Ordering::SeqCst);
                Err(AudioIngressError::Closed)
            }
        }
    }
}

pub struct AudioSendPipeline {
    tx: Mutex<Option<mpsc::Sender<PendingAudio>>>,
    failed: Arc<AtomicBool>,
    accepting: Arc<AtomicBool>,
    progress: Arc<SendProgress>,
    pending_pcm: PendingPcmGate,
    finish_tx: Mutex<Option<oneshot::Sender<()>>>,
    worker: Mutex<Option<AbortOnDropTask>>,
    abort_worker: tokio::task::AbortHandle,
}

// A monotonic timestamp shared with the native ingress without introducing
// a lock. Zero means no send has completed; other values encode elapsed+1.
struct SendProgress {
    epoch: Instant,
    last_completed_ms: AtomicU64,
    last_pcm_ms: AtomicU64,
    last_sound_ms: AtomicU64,
}

impl SendProgress {
    fn new(epoch: Instant) -> Self {
        Self {
            epoch,
            last_completed_ms: AtomicU64::new(0),
            last_pcm_ms: AtomicU64::new(0),
            last_sound_ms: AtomicU64::new(0),
        }
    }

    fn captured(&self, sound: bool, at: Instant) {
        let stamp = milliseconds(self.epoch, at).saturating_add(1);
        self.last_pcm_ms.fetch_max(stamp, Ordering::SeqCst);
        if sound {
            self.last_sound_ms.fetch_max(stamp, Ordering::SeqCst);
        }
    }

    fn input_activity(&self, now: Instant) -> (bool, bool) {
        let recent = |stamp: u64| {
            stamp != 0 && milliseconds(self.epoch, now).saturating_sub(stamp - 1) < 2000
        };
        // Read sound first: its publication follows PCM, so concurrent reads
        // cannot claim sound without a corresponding data observation.
        let sound = recent(self.last_sound_ms.load(Ordering::SeqCst));
        let pcm = recent(self.last_pcm_ms.load(Ordering::SeqCst));
        (pcm, sound)
    }

    fn completed(&self, at: Instant) {
        self.last_completed_ms.store(
            milliseconds(self.epoch, at).saturating_add(1),
            Ordering::Release,
        );
    }

    fn completed_ago_ms(&self, now: Instant) -> Option<u64> {
        self.last_completed_ms
            .load(Ordering::Acquire)
            .checked_sub(1)
            .map(|completed| milliseconds(self.epoch, now).saturating_sub(completed))
    }
}

// A cancelled finish future must not detach a worker that still owns queued
// audio and a provider client. Keep abort ownership while awaiting the task.
struct AbortOnDropTask(tokio::task::JoinHandle<bool>);

impl Drop for AbortOnDropTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl AudioSendPipeline {
    pub fn spawn<F, Fut, E>(
        send_audio: F,
        on_error: impl Fn(AudioPipelineFailure) + Send + Sync + 'static,
    ) -> Self
    where
        F: Fn(Vec<u8>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), E>> + Send + 'static,
        E: Send + 'static,
    {
        let (tx, mut rx) = mpsc::channel::<PendingAudio>(QUEUE_CAPACITY);
        let (finish_tx, mut finish_rx) = oneshot::channel();
        let failed = Arc::new(AtomicBool::new(false));
        let failed_worker = failed.clone();
        let accepting = Arc::new(AtomicBool::new(true));
        let accepting_worker = Arc::clone(&accepting);
        let progress = Arc::new(SendProgress::new(Instant::now()));
        let progress_worker = Arc::clone(&progress);
        let pending_pcm = PendingPcmGate::default();
        let on_error = Arc::new(on_error);
        let on_error_worker = on_error.clone();

        let worker = tokio::spawn(async move {
            let mut sent_buffer_count: u64 = 0;
            let mut sent_byte_count: u64 = 0;
            let mut peak_audio_sample: i32 = 0;
            let mut finishing = false;

            loop {
                let data = tokio::select! {
                    biased;
                    _ = &mut finish_rx, if !finishing => {
                        finishing = true;
                        // Native callbacks may retain ingress clones until
                        // their asynchronous teardown completes.
                        rx.close();
                        continue;
                    }
                    data = rx.recv() => match data {
                        Some(data) => data,
                        None => return true,
                    },
                };
                let PendingAudio { data, pending, .. } = data;
                let bytes = data.len();
                peak_audio_sample = peak_audio_sample.max(peak_pcm16_sample(&data));
                let started_at = Instant::now();
                let result = send_audio(data).await;
                // Freeze the measurement before level/counter diagnostics:
                // a slow log writer is not part of the send operation.
                let finished_at = Instant::now();
                drop(pending);
                let send_ms = milliseconds(started_at, finished_at);
                match result {
                    Ok(()) => {
                        progress_worker.completed(finished_at);
                        sent_buffer_count += 1;
                        sent_byte_count += bytes as u64;
                        if sent_buffer_count == 1 || sent_buffer_count.is_multiple_of(100) {
                            pipeline_log!(
                                "audio sent buffers={} bytes={} peakDbFS={}",
                                sent_buffer_count,
                                sent_byte_count,
                                decibels_full_scale(peak_audio_sample)
                            );
                            peak_audio_sample = 0;
                        }
                        if send_ms > 200 {
                            pipeline_log!("audio send blockedMs={} bytes={}", send_ms, bytes);
                        }
                    }
                    Err(_) => {
                        accepting_worker.store(false, Ordering::SeqCst);
                        pipeline_log!("audio send failed label=audio_pipeline.transport_stopped");
                        if !failed_worker.swap(true, Ordering::SeqCst) {
                            on_error_worker(AudioPipelineFailure::TransportStopped);
                        }
                        return false;
                    }
                }
            }
        });

        let abort_worker = worker.abort_handle();
        Self {
            tx: Mutex::new(Some(tx)),
            failed,
            accepting,
            progress,
            pending_pcm,
            finish_tx: Mutex::new(Some(finish_tx)),
            worker: Mutex::new(Some(AbortOnDropTask(worker))),
            abort_worker,
        }
    }

    pub fn input_activity(&self) -> (bool, bool) {
        self.progress.input_activity(Instant::now())
    }

    pub fn pending_pcm_gate(&self) -> PendingPcmGate {
        self.pending_pcm.clone()
    }

    pub fn ingress(&self) -> Option<AudioIngress> {
        self.tx.lock().unwrap().as_ref().map(|tx| AudioIngress {
            tx: tx.clone(),
            failed: Arc::clone(&self.failed),
            accepting: Arc::clone(&self.accepting),
            progress: Arc::clone(&self.progress),
            pending_pcm: self.pending_pcm.clone(),
            redirect: None,
        })
    }

    /// Stops accepting new buffers, closes the channel, and lets the worker
    /// send everything already queued. A stalled transport is aborted when
    /// the finite drain deadline expires.
    pub async fn finish(&self, timeout: Duration) -> bool {
        self.accepting.store(false, Ordering::SeqCst);
        self.tx.lock().unwrap().take();
        if let Some(finish_tx) = self.finish_tx.lock().unwrap().take() {
            let _ = finish_tx.send(());
        }
        let worker = self.worker.lock().unwrap().take();
        let Some(mut worker) = worker else {
            return true;
        };
        match tokio::time::timeout(timeout, &mut worker.0).await {
            Ok(Ok(drained)) => drained,
            Ok(Err(_)) => false,
            Err(_) => {
                worker.0.abort();
                false
            }
        }
    }

    pub fn stop(&self) {
        self.accepting.store(false, Ordering::SeqCst);
        self.failed.store(true, Ordering::SeqCst);
        self.tx.lock().unwrap().take();
        self.abort_worker.abort();
        self.worker.lock().unwrap().take();
    }
}

impl Drop for AudioSendPipeline {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Absolute peak of a little-endian i16 PCM buffer.
fn peak_pcm16_sample(data: &[u8]) -> i32 {
    let mut peak = 0i32;
    for chunk in data.as_chunks::<2>().0 {
        let sample = i16::from_le_bytes(*chunk) as i32;
        peak = peak.max(sample.abs());
    }
    peak
}

fn decibels_full_scale(peak: i32) -> i32 {
    if peak <= 0 {
        return -96;
    }
    (20.0 * (peak as f64 / 32_768.0).log10()).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[tokio::test]
    async fn simultaneous_source_queues_stay_independent_and_retired_ingress_cannot_restart() {
        let (system, system_released) = stalled_pipeline();
        let system_ingress = system.ingress().unwrap();
        let system_pending = system.pending_pcm_gate();
        system_ingress.try_send(vec![11, 0]).unwrap();
        let (tx, mut rx) = mpsc::channel(4);
        let microphone = AudioSendPipeline::spawn(
            move |data| {
                let tx = tx.clone();
                async move { tx.send(data).await }
            },
            |_| {},
        );
        let mic_ingress = microphone.ingress().unwrap();
        let mic_pending = microphone.pending_pcm_gate();
        mic_ingress.try_send(vec![22, 0]).unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), rx.recv())
                .await
                .unwrap(),
            Some(vec![22, 0])
        );
        assert!(system_pending.has_pending());
        wait_for_no_pending_pcm(&mic_pending).await;
        assert!(rx.try_recv().is_err());
        // Pause/recovery/stop closes both inputs, including work already popped.
        system.stop();
        microphone.stop();
        tokio::time::timeout(Duration::from_secs(1), system_released)
            .await
            .unwrap()
            .unwrap();
        wait_for_no_pending_pcm(&system_pending).await;
        assert_eq!(
            system_ingress.try_send(vec![33, 0]),
            Err(AudioIngressError::Closed)
        );
        assert_eq!(
            mic_ingress.try_send(vec![44, 0]),
            Err(AudioIngressError::Closed)
        );
        let (tx, mut resumed_rx) = mpsc::channel(4);
        let resumed = AudioSendPipeline::spawn(
            move |data| {
                let tx = tx.clone();
                async move { tx.send(data).await }
            },
            |_| {},
        );
        resumed.ingress().unwrap().try_send(vec![55, 0]).unwrap();
        assert!(resumed.finish(Duration::from_secs(1)).await);
        assert_eq!(resumed_rx.recv().await, Some(vec![55, 0]));
        assert!(resumed_rx.try_recv().is_err());
    }

    #[test]
    fn send_progress_distinguishes_no_completion_and_tracks_the_latest_success() {
        let start = Instant::now();
        let progress = SendProgress::new(start);
        assert_eq!(
            progress.completed_ago_ms(start + Duration::from_secs(1)),
            None
        );
        // Completion in the epoch's first millisecond must not look absent.
        progress.completed(start);
        assert_eq!(
            progress.completed_ago_ms(start + Duration::from_millis(450)),
            Some(450)
        );
        progress.completed(start + Duration::from_millis(440));
        assert_eq!(
            progress.completed_ago_ms(start + Duration::from_millis(450)),
            Some(10)
        );
    }

    #[tokio::test]
    async fn graceful_finish_drains_buffers_already_in_the_queue() {
        let sent = Arc::new(AtomicUsize::new(0));
        let sent_by_worker = Arc::clone(&sent);
        let pipeline = AudioSendPipeline::spawn(
            move |_data| {
                let sent = Arc::clone(&sent_by_worker);
                async move {
                    sent.fetch_add(1, Ordering::SeqCst);
                    Ok::<(), ()>(())
                }
            },
            |_| {},
        );
        let ingress = pipeline.ingress().unwrap();
        let pending = pipeline.pending_pcm_gate();
        ingress.try_send(vec![1, 0]).unwrap();
        ingress.try_send(vec![2, 0]).unwrap();
        // Native teardown can retain the callback and its ingress even after
        // capture has stopped; that must not keep the receiver open.
        assert!(pipeline.finish(Duration::from_millis(200)).await);
        assert_eq!(pending.pending_count(), 0);
        assert_eq!(sent.load(Ordering::SeqCst), 2);
        assert!(ingress.progress.completed_ago_ms(Instant::now()).is_some());
        assert_eq!(ingress.try_send(vec![3, 0]), Err(AudioIngressError::Closed));
    }

    #[tokio::test]
    async fn graceful_finish_reports_transport_failure() {
        let pipeline = AudioSendPipeline::spawn(|_data| async { Err::<(), ()>(()) }, |_| {});
        let pending = pipeline.pending_pcm_gate();
        pipeline.ingress().unwrap().try_send(vec![0, 0]).unwrap();
        pipeline.ingress().unwrap().try_send(vec![1, 0]).unwrap();

        assert!(!pipeline.finish(Duration::from_millis(200)).await);
        assert_eq!(pending.pending_count(), 0);
        assert_eq!(pipeline.progress.completed_ago_ms(Instant::now()), None);
    }

    #[tokio::test]
    async fn empty_buffers_do_not_claim_pending_pcm() {
        let pipeline = AudioSendPipeline::spawn(|_data| async { Ok::<(), ()>(()) }, |_| {});
        let pending = pipeline.pending_pcm_gate();
        pipeline.ingress().unwrap().try_send(Vec::new()).unwrap();
        assert_eq!(pending.pending_count(), 0);
        assert!(pipeline.finish(Duration::from_millis(200)).await);
        assert_eq!(pending.pending_count(), 0);
    }

    struct ReleaseSignal(Option<oneshot::Sender<()>>);

    impl Drop for ReleaseSignal {
        fn drop(&mut self) {
            let _ = self.0.take().unwrap().send(());
        }
    }

    fn stalled_pipeline() -> (AudioSendPipeline, oneshot::Receiver<()>) {
        let (released_tx, released_rx) = oneshot::channel();
        let guard = Arc::new(ReleaseSignal(Some(released_tx)));
        let pipeline = AudioSendPipeline::spawn(
            move |_data| {
                let guard = Arc::clone(&guard);
                async move {
                    let _guard = guard;
                    std::future::pending::<Result<(), ()>>().await
                }
            },
            |_| {},
        );
        pipeline.ingress().unwrap().try_send(vec![0, 0]).unwrap();
        (pipeline, released_rx)
    }

    #[tokio::test]
    async fn dropping_pipeline_releases_a_stalled_worker() {
        let (pipeline, released) = stalled_pipeline();
        let ingress = pipeline.ingress().unwrap();
        let pending = pipeline.pending_pcm_gate();
        ingress.try_send(vec![1, 0]).unwrap();
        ingress.try_send(vec![2, 0]).unwrap();
        assert_eq!(pending.pending_count(), 3);
        drop(pipeline);

        tokio::time::timeout(Duration::from_secs(1), released)
            .await
            .unwrap()
            .unwrap();
        wait_for_no_pending_pcm(&pending).await;
        assert_eq!(ingress.try_send(vec![0, 0]), Err(AudioIngressError::Closed));
    }

    #[tokio::test]
    async fn cancelling_finish_releases_a_stalled_worker() {
        let (pipeline, released) = stalled_pipeline();
        let pending = pipeline.pending_pcm_gate();
        pipeline.ingress().unwrap().try_send(vec![1, 0]).unwrap();
        // Timing out the caller drops finish after it has taken ownership of
        // the worker handle, before the pipeline's own drain deadline.
        assert!(tokio::time::timeout(
            Duration::from_millis(20),
            pipeline.finish(Duration::from_secs(60)),
        )
        .await
        .is_err());

        tokio::time::timeout(Duration::from_secs(1), released)
            .await
            .unwrap()
            .unwrap();
        wait_for_no_pending_pcm(&pending).await;
    }

    #[tokio::test]
    async fn stop_can_abort_a_worker_already_owned_by_finish() {
        let (pipeline, released) = stalled_pipeline();
        let finish = pipeline.finish(Duration::from_secs(60));
        tokio::pin!(finish);
        assert!(tokio::time::timeout(Duration::from_millis(20), &mut finish)
            .await
            .is_err());
        pipeline.stop();

        assert!(!finish.await);
        released.await.unwrap();
    }

    #[tokio::test]
    async fn graceful_finish_aborts_a_stalled_transport_at_the_deadline() {
        let pipeline = AudioSendPipeline::spawn(
            |_data| async move {
                std::future::pending::<()>().await;
                Ok::<(), ()>(())
            },
            |_| {},
        );
        let ingress = pipeline.ingress().unwrap();
        ingress.try_send(vec![1, 0]).unwrap();
        drop(ingress);

        let started = tokio::time::Instant::now();
        assert!(!pipeline.finish(Duration::from_millis(30)).await);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn graceful_finish_reports_a_cancelled_worker_as_failure() {
        let pipeline = AudioSendPipeline::spawn(|_data| async move { Ok::<(), ()>(()) }, |_| {});
        pipeline.abort_worker.abort();

        assert!(!pipeline.finish(Duration::from_millis(200)).await);
    }

    #[tokio::test]
    async fn native_ingress_is_non_blocking_and_fails_closed_when_full() {
        let pipeline = AudioSendPipeline::spawn(
            |_data| async move {
                std::future::pending::<()>().await;
                Ok::<(), ()>(())
            },
            |_| {},
        );
        let ingress = pipeline.ingress().unwrap();
        let pending = pipeline.pending_pcm_gate();
        assert_eq!(ingress.try_send(vec![0, 0]), Ok(()));

        let mut accepted = 1;
        while ingress.try_send(vec![0, 0]) == Ok(()) {
            accepted += 1;
            assert!(accepted <= QUEUE_CAPACITY + 1);
        }

        assert!(accepted <= QUEUE_CAPACITY + 1);
        assert_eq!(pending.pending_count(), accepted);
        assert_eq!(ingress.progress.completed_ago_ms(Instant::now()), None);
        assert_eq!(ingress.try_send(vec![0, 0]), Err(AudioIngressError::Closed));
        assert_eq!(pending.pending_count(), accepted);
        pipeline.stop();
        wait_for_no_pending_pcm(&pending).await;
    }

    async fn wait_for_no_pending_pcm(pending: &PendingPcmGate) {
        tokio::time::timeout(Duration::from_secs(1), async {
            while pending.has_pending() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn queue_backpressure_is_reported_exactly_once_by_ingress() {
        let pipeline = AudioSendPipeline::spawn(
            |_data| async move {
                std::future::pending::<()>().await;
                Ok::<(), ()>(())
            },
            |_| {},
        );
        let ingress = pipeline.ingress().unwrap();
        let mut backpressure_count = 0;
        for _ in 0..QUEUE_CAPACITY + 4 {
            if ingress.try_send(vec![0, 0]) == Err(AudioIngressError::Backpressure) {
                backpressure_count += 1;
            }
        }

        assert_eq!(backpressure_count, 1);
        pipeline.stop();
    }
}

#[cfg(test)]
mod activity_tests {
    use super::*;
    #[test]
    fn concurrent_activity_does_not_claim_sound_without_pcm() {
        let now = Instant::now();
        let progress = Arc::new(SendProgress::new(now));
        std::thread::scope(|scope| {
            let writer = Arc::clone(&progress);
            scope.spawn(move || {
                for _ in 0..1000 {
                    writer.captured(true, now);
                }
            });
            for _ in 0..1000 {
                let (pcm, sound) = progress.input_activity(now);
                assert!(!sound || pcm);
            }
        });
    }

    #[test]
    fn real_ingress_progress_separates_pcm_from_sound_and_expires() {
        let now = Instant::now();
        let progress = SendProgress::new(now);
        assert_eq!(progress.input_activity(now), (false, false));
        progress.captured(false, now);
        assert_eq!(progress.input_activity(now), (true, false));
        progress.captured(true, now);
        assert_eq!(progress.input_activity(now), (true, true));
        progress.captured(false, now + Duration::from_secs(3));
        assert_eq!(
            progress.input_activity(now + Duration::from_secs(3)),
            (true, false)
        );
        assert_eq!(
            progress.input_activity(now + Duration::from_secs(5)),
            (false, false)
        );
    }
}
