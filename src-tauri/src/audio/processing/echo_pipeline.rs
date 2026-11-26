//! A bounded, attempt-owned acoustic echo cancellation stage. System PCM is
//! forwarded unchanged; its reference copy never waits for either provider.
//! Microphone PCM is processed before entering its independent send queue.

use super::send_pipeline::{AudioIngress, AudioIngressError, PendingAudio};
use super::{CaptureFailureSender, SystemAudioCaptureFailure};
use crate::core::audio::audio_input::AudioSource;
use crate::core::audio::echo_cancellation::EchoCanceller;
use crate::core::audio::pending_pcm::PendingPcmGuard;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};

const QUEUE_MESSAGES: usize = 128;
const QUEUE_AUDIO_MS: usize = 500;
const MAX_CALLBACK_MS: usize = 250;
const CAPTURE_BUFFER_MS: usize = 700;
const REFERENCE_FRAMES: usize = 100;
const FRAME_DURATION: Duration = Duration::from_millis(10);
// DSP needs 10 ms frames, but publishing each frame halves the duration of
// the 20-message provider queue relative to normal 20 ms capture packets.
const OUTPUT_BATCH_WAIT: Duration = FRAME_DURATION;
const CAPTURE_GAP: Duration = Duration::from_millis(100);
const CAPTURE_DELAY: Duration = Duration::from_millis(350);
const REFERENCE_LOOKAHEAD: Duration = Duration::from_millis(100);

trait EchoProcessor: Send {
    fn process(&mut self, render: &[i16], capture: &[i16]) -> Result<Vec<i16>, ()>;
}

impl EchoProcessor for EchoCanceller {
    fn process(&mut self, render: &[i16], capture: &[i16]) -> Result<Vec<i16>, ()> {
        self.process_pair(render, capture).map_err(|_| ())
    }
}

type ProcessorFactory = Arc<dyn Fn() -> Result<Box<dyn EchoProcessor>, ()> + Send + Sync>;

struct Shared {
    accepting: AtomicBool,
    queued_bytes: [AtomicUsize; 2],
    byte_limit: usize,
    failure: CaptureFailureSender,
}

impl Shared {
    fn fail(&self, failure: SystemAudioCaptureFailure) {
        self.accepting.store(false, Ordering::SeqCst);
        self.failure.report(failure);
    }

    fn reserve(self: &Arc<Self>, source: AudioSource, bytes: usize) -> Option<ByteLease> {
        let index = source_index(source);
        let queued_bytes = &self.queued_bytes[index];
        let mut used = queued_bytes.load(Ordering::SeqCst);
        loop {
            let next = used
                .checked_add(bytes)
                .filter(|sum| *sum <= self.byte_limit)?;
            match queued_bytes.compare_exchange_weak(used, next, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => break,
                Err(current) => used = current,
            }
        }
        Some(ByteLease {
            shared: Arc::clone(self),
            index,
            bytes,
        })
    }
}

struct ByteLease {
    shared: Arc<Shared>,
    index: usize,
    bytes: usize,
}

impl Drop for ByteLease {
    fn drop(&mut self) {
        self.shared.queued_bytes[self.index].fetch_sub(self.bytes, Ordering::SeqCst);
    }
}

struct Work {
    source: AudioSource,
    packet: PendingAudio,
    output: AudioIngress,
    _bytes: ByteLease,
}

struct AbortOnDrop(tokio::task::JoinHandle<bool>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(crate) struct EchoPipeline {
    shared: Arc<Shared>,
    tx: mpsc::Sender<Work>,
    finish_tx: Mutex<Option<oneshot::Sender<()>>>,
    worker: Mutex<Option<AbortOnDrop>>,
    abort_worker: tokio::task::AbortHandle,
}

impl EchoPipeline {
    pub fn spawn(
        sample_rate: u32,
        failure: CaptureFailureSender,
    ) -> Result<Self, SystemAudioCaptureFailure> {
        Self::spawn_with_factory(
            sample_rate,
            failure,
            Arc::new(move || {
                EchoCanceller::new(sample_rate)
                    .map(|processor| Box::new(processor) as Box<dyn EchoProcessor>)
                    .map_err(|_| ())
            }),
        )
    }

    fn spawn_with_factory(
        sample_rate: u32,
        failure: CaptureFailureSender,
        factory: ProcessorFactory,
    ) -> Result<Self, SystemAudioCaptureFailure> {
        if !matches!(sample_rate, 16_000 | 24_000) {
            return Err(SystemAudioCaptureFailure::AudioProcessingFailed);
        }
        let processor = factory().map_err(|()| SystemAudioCaptureFailure::AudioProcessingFailed)?;
        let shared = Arc::new(Shared {
            accepting: AtomicBool::new(true),
            queued_bytes: Default::default(),
            byte_limit: sample_rate as usize * 2 * QUEUE_AUDIO_MS / 1000,
            failure,
        });
        let (tx, mut rx) = mpsc::channel::<Work>(QUEUE_MESSAGES);
        let (finish_tx, mut finish_rx) = oneshot::channel();
        let worker_shared = Arc::clone(&shared);
        let worker = tokio::spawn(async move {
            let mut state = ProcessorState::new(sample_rate as usize / 100, processor, factory);
            let mut finishing = false;
            loop {
                let due = state.next_due();
                let work = tokio::select! {
                    biased;
                    _ = &mut finish_rx, if !finishing => {
                        finishing = true;
                        rx.close();
                        continue;
                    }
                    work = rx.recv() => work,
                    _ = tokio::time::sleep_until(tokio::time::Instant::from_std(
                        due.unwrap_or_else(Instant::now)
                    )), if due.is_some() => {
                        if let Err(failure) = state.process_ready(Instant::now(), false) {
                            worker_shared.fail(failure);
                            return false;
                        }
                        continue;
                    }
                };
                let result = match work {
                    Some(work) => state
                        .push(work)
                        .and_then(|()| state.process_ready(Instant::now(), false)),
                    None => {
                        if let Err(failure) = state.finish() {
                            worker_shared.fail(failure);
                            return false;
                        }
                        return true;
                    }
                };
                if let Err(failure) = result {
                    worker_shared.fail(failure);
                    return false;
                }
            }
        });
        let abort_worker = worker.abort_handle();
        Ok(Self {
            shared,
            tx,
            finish_tx: Mutex::new(Some(finish_tx)),
            worker: Mutex::new(Some(AbortOnDrop(worker))),
            abort_worker,
        })
    }

    pub fn ingress(&self, source: AudioSource, output: AudioIngress) -> AudioIngress {
        let shared = Arc::clone(&self.shared);
        let tx = self.tx.clone();
        let direct = output.clone();
        output.redirected(move |packet| {
            if !shared.accepting.load(Ordering::SeqCst) {
                return Err(AudioIngressError::Closed);
            }
            if packet.data.len() > shared.byte_limit * MAX_CALLBACK_MS / QUEUE_AUDIO_MS {
                shared.fail(SystemAudioCaptureFailure::Backpressure);
                return Err(AudioIngressError::Backpressure);
            }
            if !packet.data.len().is_multiple_of(2) {
                shared.fail(SystemAudioCaptureFailure::AudioProcessingFailed);
                return Err(AudioIngressError::Closed);
            }
            if packet.data.is_empty() {
                return Ok(());
            }
            let Some(bytes) = shared.reserve(source, packet.data.len()) else {
                shared.fail(SystemAudioCaptureFailure::Backpressure);
                return Err(AudioIngressError::Backpressure);
            };
            let packet = if source == AudioSource::System {
                let reference = PendingAudio {
                    data: packet.data.clone(),
                    pending: None,
                    captured_at: packet.captured_at,
                };
                if let Err(error) = direct.enqueue_packet(packet) {
                    shared.fail(ingress_failure(error));
                    return Err(error);
                }
                reference
            } else {
                packet
            };
            match tx.try_send(Work {
                source,
                packet,
                output: direct.clone(),
                _bytes: bytes,
            }) {
                Ok(()) => Ok(()),
                Err(mpsc::error::TrySendError::Full(_)) => {
                    shared.fail(SystemAudioCaptureFailure::Backpressure);
                    Err(AudioIngressError::Backpressure)
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    shared.fail(SystemAudioCaptureFailure::NativeStopped);
                    Err(AudioIngressError::Closed)
                }
            }
        })
    }

    /// Native capture is stopped before this call. Accepted microphone samples
    /// are processed, including a padded final frame trimmed to its real length.
    pub async fn finish(&self, timeout: Duration) -> bool {
        self.seal();
        if let Some(finish) = self.finish_tx.lock().unwrap().take() {
            let _ = finish.send(());
        }
        let worker = self.worker.lock().unwrap().take();
        let Some(mut worker) = worker else {
            return true;
        };
        matches!(
            tokio::time::timeout(timeout, &mut worker.0).await,
            Ok(Ok(true))
        )
    }

    pub fn stop(&self) {
        self.seal();
        self.abort_worker.abort();
        self.worker.lock().unwrap().take();
    }

    /// Close native ingress while retaining accepted DSP work for graceful
    /// drain. A native callback racing teardown cannot reopen this attempt.
    pub fn seal(&self) {
        self.shared.accepting.store(false, Ordering::SeqCst);
    }
}

impl Drop for EchoPipeline {
    fn drop(&mut self) {
        self.stop();
    }
}

fn source_index(source: AudioSource) -> usize {
    match source {
        AudioSource::System => 0,
        AudioSource::Microphone => 1,
    }
}

fn ingress_failure(error: AudioIngressError) -> SystemAudioCaptureFailure {
    match error {
        AudioIngressError::Backpressure => SystemAudioCaptureFailure::Backpressure,
        AudioIngressError::Closed => SystemAudioCaptureFailure::NativeStopped,
    }
}

struct AudioFrame {
    samples: Vec<i16>,
    /// Estimated time of the final sample, before DSP/network scheduling.
    at: Instant,
    epoch: u64,
}

#[derive(Default)]
struct FrameAssembler {
    pending: VecDeque<i16>,
    last_end: Option<Instant>,
}

impl FrameAssembler {
    fn push(&mut self, data: &[u8], at: Instant, frame_size: usize) -> Vec<AudioFrame> {
        self.pending.extend(decode(data));
        let mut frames = Vec::new();
        while self.pending.len() >= frame_size {
            let samples = self.pending.drain(..frame_size).collect();
            let remaining = sample_duration(self.pending.len(), frame_size);
            let observed = at.checked_sub(remaining).unwrap_or(at);
            // Samples within a stream are continuous. Slew the estimated
            // clock gently rather than allowing callback scheduling jitter
            // to duplicate/drop a whole reference frame on every callback.
            let end = match self.last_end {
                Some(last) if observed.saturating_duration_since(last) < CAPTURE_GAP => {
                    let expected = last + FRAME_DURATION;
                    if observed >= expected {
                        expected
                            + (observed - expected)
                                .div_f64(16.0)
                                .min(Duration::from_millis(1))
                    } else {
                        expected
                            - (expected - observed)
                                .div_f64(16.0)
                                .min(Duration::from_millis(1))
                    }
                }
                _ => observed,
            };
            self.last_end = Some(end);
            frames.push(AudioFrame {
                samples,
                at: end,
                epoch: 0,
            });
        }
        frames
    }
}

struct CaptureFrame {
    frame: AudioFrame,
    actual_samples: usize,
    reset: bool,
    _pending: PendingPcmGuard,
}

struct ProcessorState {
    frame_size: usize,
    processor: Box<dyn EchoProcessor>,
    factory: ProcessorFactory,
    render: FrameAssembler,
    capture: FrameAssembler,
    capture_pending_guard: Option<PendingPcmGuard>,
    capture_output: Option<AudioIngress>,
    reference: VecDeque<AudioFrame>,
    queued_capture: VecDeque<CaptureFrame>,
    last_capture_at: Option<Instant>,
    last_render_at: Option<Instant>,
    render_epoch: u64,
    applied_render_epoch: Option<u64>,
    reset_next_capture: bool,
}

impl ProcessorState {
    fn new(
        frame_size: usize,
        processor: Box<dyn EchoProcessor>,
        factory: ProcessorFactory,
    ) -> Self {
        Self {
            frame_size,
            processor,
            factory,
            render: FrameAssembler::default(),
            capture: FrameAssembler::default(),
            capture_pending_guard: None,
            capture_output: None,
            reference: VecDeque::new(),
            queued_capture: VecDeque::new(),
            last_capture_at: None,
            last_render_at: None,
            render_epoch: 0,
            applied_render_epoch: None,
            reset_next_capture: false,
        }
    }

    fn next_due(&self) -> Option<Instant> {
        self.queued_capture
            .front()
            .map(|work| work.frame.at + CAPTURE_DELAY + OUTPUT_BATCH_WAIT)
    }

    fn push(&mut self, work: Work) -> Result<(), SystemAudioCaptureFailure> {
        let Work {
            source,
            packet,
            output,
            _bytes,
        } = work;
        let PendingAudio {
            data,
            pending,
            captured_at,
        } = packet;
        match source {
            AudioSource::System => {
                let duration = sample_duration(data.len() / 2, self.frame_size);
                if self.last_render_at.is_some_and(|last| {
                    captured_at.saturating_duration_since(last) > duration + CAPTURE_GAP
                }) {
                    // A partial reference before a real gap cannot be joined
                    // to samples after it. Keep old complete frames only for
                    // microphone work already waiting in the jitter buffer.
                    self.render.pending.clear();
                    self.render.last_end = None;
                    self.render_epoch = self.render_epoch.wrapping_add(1);
                }
                self.last_render_at = Some(captured_at);
                for mut frame in self.render.push(&data, captured_at, self.frame_size) {
                    frame.epoch = self.render_epoch;
                    if self.reference.len() == REFERENCE_FRAMES {
                        self.reference.pop_front();
                    }
                    self.reference.push_back(frame);
                }
            }
            AudioSource::Microphone => {
                let incoming = data.len() / 2;
                let samples = self.queued_capture.len() * self.frame_size
                    + self.capture.pending.len()
                    + incoming;
                if samples > self.frame_size * CAPTURE_BUFFER_MS / 10 {
                    return Err(SystemAudioCaptureFailure::Backpressure);
                }
                let duration = sample_duration(incoming, self.frame_size);
                if self.last_capture_at.is_some_and(|last| {
                    captured_at.saturating_duration_since(last) > duration + CAPTURE_GAP
                }) {
                    self.queue_partial();
                    self.capture.last_end = None;
                    self.reset_next_capture = true;
                }
                self.last_capture_at = Some(captured_at);
                self.capture_output = Some(output.clone());
                if self.capture_pending_guard.is_none() {
                    self.capture_pending_guard = Some(output.acquire_pending());
                }
                for frame in self.capture.push(&data, captured_at, self.frame_size) {
                    let reset = std::mem::take(&mut self.reset_next_capture);
                    self.queued_capture.push_back(CaptureFrame {
                        frame,
                        actual_samples: self.frame_size,
                        reset,
                        _pending: output.acquire_pending(),
                    });
                }
                if self.capture.pending.is_empty() {
                    self.capture_pending_guard.take();
                }
            }
        }
        // Per-frame guards are installed before the original ingress guard is
        // released, including while a frame waits in the jitter buffer.
        drop(pending);
        Ok(())
    }

    fn process_ready(
        &mut self,
        now: Instant,
        force: bool,
    ) -> Result<(), SystemAudioCaptureFailure> {
        if !force && self.next_due().is_none_or(|due| due > now) {
            return Ok(());
        }
        let mut result = Vec::new();
        // Retain each frame's guard until its output owns the network guard.
        let mut processed = Vec::new();
        while self
            .queued_capture
            .front()
            .is_some_and(|work| force || work.frame.at + CAPTURE_DELAY <= now)
        {
            let work = self.queued_capture.pop_front().unwrap();
            let reference = self.reference_for(work.frame.at + REFERENCE_LOOKAHEAD);
            let reference_epoch = reference.as_ref().map(|frame| frame.epoch);
            if work.reset
                || (self.applied_render_epoch.is_some()
                    && self.applied_render_epoch != reference_epoch)
            {
                // Missing reference is a discontinuity, not known silence.
                // Do not subtract a learned old tail from near-end speech.
                self.processor = (self.factory)()
                    .map_err(|()| SystemAudioCaptureFailure::AudioProcessingFailed)?;
            }
            self.applied_render_epoch = reference_epoch;
            let reference =
                reference.map_or_else(|| vec![0; self.frame_size], |frame| frame.samples);
            let output = self
                .processor
                .process(&reference, &work.frame.samples)
                .map_err(|()| SystemAudioCaptureFailure::AudioProcessingFailed)?;
            if output.len() != self.frame_size {
                return Err(SystemAudioCaptureFailure::AudioProcessingFailed);
            }
            result.extend_from_slice(&output[..work.actual_samples]);
            processed.push(work);
        }
        if !result.is_empty() {
            if let Some(output) = &self.capture_output {
                output
                    .try_send_processed(encode(&result))
                    .map_err(ingress_failure)?;
            }
        }
        drop(processed);
        Ok(())
    }

    fn reference_for(&mut self, at: Instant) -> Option<AudioFrame> {
        let distance = |frame: &AudioFrame| {
            frame
                .at
                .saturating_duration_since(at)
                .max(at.saturating_duration_since(frame.at))
        };
        let nearest = self
            .reference
            .iter()
            .enumerate()
            .min_by_key(|(_, frame)| distance(frame))
            .filter(|(_, frame)| distance(frame) <= FRAME_DURATION)
            .map(|(index, _)| index);
        if let Some(index) = nearest {
            // A sample is used once. Missing reference is silence, never an
            // old frame replay; near-end speech must continue unchanged.
            self.reference.drain(..index);
            self.reference.pop_front()
        } else {
            while self
                .reference
                .front()
                .is_some_and(|frame| frame.at + FRAME_DURATION < at)
            {
                self.reference.pop_front();
            }
            None
        }
    }

    fn queue_partial(&mut self) {
        if self.capture.pending.is_empty() {
            return;
        }
        let mut samples = self.capture.pending.drain(..).collect::<Vec<_>>();
        let actual_samples = samples.len();
        samples.resize(self.frame_size, 0);
        if let Some(output) = &self.capture_output {
            let reset = std::mem::take(&mut self.reset_next_capture);
            self.queued_capture.push_back(CaptureFrame {
                frame: AudioFrame {
                    samples,
                    at: self.last_capture_at.unwrap_or_else(Instant::now),
                    epoch: 0,
                },
                actual_samples,
                reset,
                _pending: output.acquire_pending(),
            });
        }
        self.capture_pending_guard.take();
    }

    fn finish(&mut self) -> Result<(), SystemAudioCaptureFailure> {
        self.queue_partial();
        self.process_ready(Instant::now(), true)
    }
}

fn sample_duration(samples: usize, frame_size: usize) -> Duration {
    Duration::from_secs_f64(samples as f64 / (frame_size * 100) as f64)
}

fn decode(data: &[u8]) -> impl Iterator<Item = i16> + '_ {
    data.as_chunks::<2>()
        .0
        .iter()
        .map(|bytes| i16::from_le_bytes(*bytes))
}

fn encode(data: &[i16]) -> Vec<u8> {
    data.iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect()
}

#[cfg(test)]
#[path = "echo_pipeline/tests.rs"]
mod tests;
