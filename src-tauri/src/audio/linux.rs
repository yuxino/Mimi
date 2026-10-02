//! Linux system audio through PulseAudio (including PipeWire's PulseAudio
//! server). Resolve and verify the default output's monitor explicitly; the
//! default recording source is never opened. The sound server converts the
//! monitor mix to the requested mono PCM16LE rate.
//!
//! All libpulse objects stay on one worker thread. A nonblocking mainloop
//! keeps startup, pause/stop and cancelled start futures bounded even when
//! the server is silent or disappears. SessionManager implements pause and
//! resume by stopping this worker and starting a fresh monitor stream.

use crate::audio::send_pipeline::{AudioIngress, AudioIngressError};
use crate::audio::{
    AudioCaptureFormat, CaptureFailureSender, SystemAudioCaptureError, SystemAudioCaptureFailure,
};
use crate::core::diagnostics::milliseconds;
use crate::pipeline_log;
use libpulse_binding as pulse;
use pulse::callbacks::ListResult;
use pulse::context::{Context, FlagSet as ContextFlags, State as ContextState};
use pulse::def::BufferAttr;
use pulse::mainloop::standard::{IterateResult, Mainloop};
use pulse::sample::{Format, Spec};
use pulse::stream::{FlagSet as StreamFlags, PeekResult, State as StreamState, Stream};
use std::cell::RefCell;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

const POLL_INTERVAL: Duration = Duration::from_millis(5);
const START_TIMEOUT: Duration = Duration::from_secs(5);
const FRAGMENT_MS: usize = 20;
const MAX_BUFFER_MS: usize = 250;

#[derive(Clone, Default)]
pub struct LinuxSystemAudioCapture {
    state: Arc<CaptureState>,
}

#[derive(Default)]
struct CaptureState {
    worker: Mutex<Option<Arc<WorkerControl>>>,
}

impl Drop for CaptureState {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.get_mut().unwrap().as_ref() {
            worker.cancelled.store(true, Ordering::SeqCst);
        }
    }
}

#[derive(Default)]
struct WorkerControl {
    cancelled: AtomicBool,
    finished: AtomicBool,
}

/// Dropping an awaited start (for example SessionManager's timeout) must
/// cancel its native worker, including before it reports readiness.
struct CancelStartOnDrop(Option<Arc<WorkerControl>>);

impl Drop for CancelStartOnDrop {
    fn drop(&mut self) {
        if let Some(worker) = &self.0 {
            worker.cancelled.store(true, Ordering::SeqCst);
        }
    }
}

struct FinishWorkerOnDrop(Arc<WorkerControl>);

impl Drop for FinishWorkerOnDrop {
    fn drop(&mut self) {
        self.0.finished.store(true, Ordering::SeqCst);
    }
}

impl LinuxSystemAudioCapture {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn start(
        &self,
        audio_ingress: AudioIngress,
        failure_tx: CaptureFailureSender,
        format: AudioCaptureFormat,
    ) -> Result<(), SystemAudioCaptureError> {
        AudioCaptureFormat::pcm16_mono(format.sample_rate_hz)?;
        let control = self.reserve_worker()?;
        let mut cancel_start = CancelStartOnDrop(Some(Arc::clone(&control)));
        let (ready_tx, ready_rx) = oneshot::channel();
        let worker_control = Arc::clone(&control);
        if std::thread::Builder::new()
            .name("mimi-system-audio".into())
            .spawn(move || {
                // Constructed first, dropped last: finished means libpulse
                // has disconnected and released the stream and context.
                let _finished = FinishWorkerOnDrop(Arc::clone(&worker_control));
                let mut capture = match PulseCapture::connect(&worker_control, format) {
                    Ok(capture) => capture,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                if worker_control.cancelled.load(Ordering::SeqCst) {
                    let _ = ready_tx.send(Err(SystemAudioCaptureError::StartCancelled));
                    return;
                }
                if ready_tx.send(Ok(())).is_err() {
                    return;
                }
                if let Err(failure) = capture.run(&worker_control, &audio_ingress, &failure_tx) {
                    if !worker_control.cancelled.load(Ordering::SeqCst) {
                        failure_tx.report(failure);
                    }
                }
            })
            .is_err()
        {
            control.finished.store(true, Ordering::SeqCst);
            return Err(SystemAudioCaptureError::NativeStartFailed);
        }

        ready_rx
            .await
            .map_err(|_| SystemAudioCaptureError::NativeStartFailed)??;
        if control.cancelled.load(Ordering::SeqCst) {
            return Err(SystemAudioCaptureError::StartCancelled);
        }
        cancel_start.0 = None;
        Ok(())
    }

    fn reserve_worker(&self) -> Result<Arc<WorkerControl>, SystemAudioCaptureError> {
        let mut slot = self.state.worker.lock().unwrap();
        if let Some(worker) = slot.as_ref() {
            if !worker.finished.load(Ordering::SeqCst) {
                return Err(if worker.cancelled.load(Ordering::SeqCst) {
                    SystemAudioCaptureError::PreviousCaptureStopping
                } else {
                    SystemAudioCaptureError::AlreadyRunning
                });
            }
        }
        let worker = Arc::new(WorkerControl::default());
        *slot = Some(Arc::clone(&worker));
        Ok(worker)
    }

    pub async fn stop(&self) {
        let worker = {
            let slot = self.state.worker.lock().unwrap();
            slot.as_ref().map(|worker| {
                worker.cancelled.store(true, Ordering::SeqCst);
                Arc::clone(worker)
            })
        };
        if let Some(worker) = worker {
            // The slot stays reserved if this stop future is itself dropped.
            // A new start cannot overlap a worker still releasing its source.
            while !worker.finished.load(Ordering::SeqCst) {
                tokio::time::sleep(POLL_INTERVAL).await;
            }
        }
    }
}

struct MonitorSource {
    sink_index: u32,
    source_index: u32,
    source_name: String,
}

impl MonitorSource {
    fn matches(&self, source_index: u32, monitor_of_sink: Option<u32>) -> bool {
        self.source_index == source_index && monitor_of_sink == Some(self.sink_index)
    }
}

struct PulseCapture {
    // Drop order matters: the stream and context depend on the mainloop.
    stream: Stream,
    context: ConnectedContext,
    mainloop: Mainloop,
    monitor_index: u32,
    chunk_bytes: usize,
    max_buffer_bytes: usize,
}

/// Disconnect on every exit path, including a cancelled or timed-out
/// introspection before a stream exists. Context's own Drop only unrefs it.
struct ConnectedContext(Context);

impl Deref for ConnectedContext {
    type Target = Context;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for ConnectedContext {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for ConnectedContext {
    fn drop(&mut self) {
        self.0.disconnect();
    }
}

impl PulseCapture {
    fn connect(
        control: &WorkerControl,
        format: AudioCaptureFormat,
    ) -> Result<Self, SystemAudioCaptureError> {
        let deadline = Instant::now() + START_TIMEOUT;
        let mut mainloop =
            Mainloop::new().ok_or(SystemAudioCaptureError::AudioServerUnavailable)?;
        let mut context = ConnectedContext(
            Context::new(&mainloop, "mimi")
                .ok_or(SystemAudioCaptureError::AudioServerUnavailable)?,
        );
        context
            .connect(None, ContextFlags::NOAUTOSPAWN, None)
            .map_err(|_| SystemAudioCaptureError::AudioServerUnavailable)?;
        wait_for(&mut mainloop, control, deadline, || {
            match context.get_state() {
                ContextState::Ready => Ok(Some(())),
                ContextState::Failed | ContextState::Terminated => {
                    Err(SystemAudioCaptureError::AudioServerUnavailable)
                }
                _ => Ok(None),
            }
        })?;

        let monitor = resolve_monitor(&mut mainloop, &context, control, deadline)?;
        let spec = Spec {
            format: Format::S16le,
            channels: 1,
            rate: format.sample_rate_hz,
        };
        let chunk_bytes = format.sample_rate_hz as usize * 2 * FRAGMENT_MS / 1000;
        let max_buffer_bytes = format.sample_rate_hz as usize * 2 * MAX_BUFFER_MS / 1000;
        let mut stream = Stream::new(&mut context, "System audio", &spec, None)
            .ok_or(SystemAudioCaptureError::NativeStartFailed)?;
        let buffer = BufferAttr {
            maxlength: max_buffer_bytes as u32,
            tlength: u32::MAX,
            prebuf: u32::MAX,
            minreq: u32::MAX,
            fragsize: chunk_bytes as u32,
        };
        stream
            .connect_record(
                Some(&monitor.source_name),
                Some(&buffer),
                StreamFlags::DONT_MOVE | StreamFlags::ADJUST_LATENCY,
            )
            .map_err(|_| SystemAudioCaptureError::NativeStartFailed)?;
        wait_for(&mut mainloop, control, deadline, || {
            if context.get_state() != ContextState::Ready {
                return Err(SystemAudioCaptureError::AudioServerUnavailable);
            }
            match stream.get_state() {
                StreamState::Ready => Ok(Some(())),
                StreamState::Failed | StreamState::Terminated => {
                    Err(SystemAudioCaptureError::NativeStartFailed)
                }
                _ => Ok(None),
            }
        })?;
        if stream.get_device_index() != Some(monitor.source_index)
            || stream.get_sample_spec() != Some(&spec)
        {
            return Err(SystemAudioCaptureError::UnsupportedAudioFormat);
        }

        Ok(Self {
            stream,
            context,
            mainloop,
            monitor_index: monitor.source_index,
            chunk_bytes,
            max_buffer_bytes,
        })
    }

    fn run(
        &mut self,
        control: &WorkerControl,
        ingress: &AudioIngress,
        failure_tx: &CaptureFailureSender,
    ) -> Result<(), SystemAudioCaptureFailure> {
        // Each capture owns its partial frame. Stop/cancellation drops less
        // than 20 ms without waiting, padding, or carrying it into a restart.
        let mut frames = PcmFrameAssembler::new(self.chunk_bytes, self.max_buffer_bytes);
        let mut last_poll_finished_at = Instant::now();
        while !control.cancelled.load(Ordering::SeqCst) && !failure_tx.has_reported() {
            let poll_started_at = Instant::now();
            let poll_result = self.mainloop.iterate(false);
            let poll_finished_at = Instant::now();
            let gap_ms = milliseconds(last_poll_finished_at, poll_started_at);
            let iterate_ms = milliseconds(poll_started_at, poll_finished_at);
            last_poll_finished_at = poll_finished_at;
            if gap_ms > 100 || iterate_ms > 100 {
                pipeline_log!(
                    "capture native poll gapMs={} iterateMs={}",
                    gap_ms,
                    iterate_ms
                );
            }
            if !matches!(poll_result, IterateResult::Success(_))
                || self.context.get_state() != ContextState::Ready
                || self.stream.get_state() != StreamState::Ready
                || self.stream.get_device_index() != Some(self.monitor_index)
            {
                return Err(SystemAudioCaptureFailure::NativeStopped);
            }
            loop {
                if control.cancelled.load(Ordering::SeqCst) {
                    return Ok(());
                }
                let keep_running = match self
                    .stream
                    .peek()
                    .map_err(|_| SystemAudioCaptureFailure::NativeStopped)?
                {
                    PeekResult::Empty => break,
                    PeekResult::Data(data) => {
                        frames.forward_fragment(Some(data), data.len(), control, ingress)
                    }
                    // A PulseAudio hole is missing/silent audio, never an
                    // initialized data buffer that may be copied blindly.
                    PeekResult::Hole(bytes) => {
                        frames.forward_fragment(None, bytes, control, ingress)
                    }
                };
                self.stream
                    .discard()
                    .map_err(|_| SystemAudioCaptureFailure::NativeStopped)?;
                if !keep_running? {
                    return Ok(());
                }
            }
            std::thread::sleep(POLL_INTERVAL);
        }
        Ok(())
    }
}

fn wait_for<T>(
    mainloop: &mut Mainloop,
    control: &WorkerControl,
    deadline: Instant,
    mut check: impl FnMut() -> Result<Option<T>, SystemAudioCaptureError>,
) -> Result<T, SystemAudioCaptureError> {
    loop {
        if control.cancelled.load(Ordering::SeqCst) {
            return Err(SystemAudioCaptureError::StartCancelled);
        }
        if Instant::now() >= deadline {
            return Err(SystemAudioCaptureError::StartTimedOut);
        }
        if let Some(value) = check()? {
            return Ok(value);
        }
        if !matches!(mainloop.iterate(false), IterateResult::Success(_)) {
            return Err(SystemAudioCaptureError::AudioServerUnavailable);
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn resolve_monitor(
    mainloop: &mut Mainloop,
    context: &Context,
    control: &WorkerControl,
    deadline: Instant,
) -> Result<MonitorSource, SystemAudioCaptureError> {
    let sink_name = Rc::new(RefCell::new(None));
    let sink_name_result = Rc::clone(&sink_name);
    let _server_operation = context.introspect().get_server_info(move |info| {
        *sink_name_result.borrow_mut() = Some(info.default_sink_name.as_deref().map(str::to_owned));
    });
    let sink_name = wait_for(mainloop, control, deadline, || {
        check_context(context)?;
        Ok(sink_name.borrow_mut().take())
    })?
    .filter(|name| valid_device_name(name))
    .ok_or(SystemAudioCaptureError::NoPlaybackDevice)?;

    let monitor = Rc::new(RefCell::new(None));
    let monitor_result = Rc::clone(&monitor);
    let _sink_operation = context
        .introspect()
        .get_sink_info_by_name(&sink_name, move |result| match result {
            ListResult::Item(info) => {
                *monitor_result.borrow_mut() = Some(
                    info.monitor_source_name
                        .as_deref()
                        .filter(|name| valid_device_name(name))
                        .filter(|_| info.monitor_source != u32::MAX)
                        .map(|name| MonitorSource {
                            sink_index: info.index,
                            source_index: info.monitor_source,
                            source_name: name.to_owned(),
                        }),
                );
            }
            ListResult::Error | ListResult::End => {
                if monitor_result.borrow().is_none() {
                    *monitor_result.borrow_mut() = Some(None);
                }
            }
        });
    let monitor = wait_for(mainloop, control, deadline, || {
        check_context(context)?;
        Ok(monitor.borrow_mut().take())
    })?
    .ok_or(SystemAudioCaptureError::NoPlaybackDevice)?;

    // Validate the native monitor relationship. Names ending in '.monitor'
    // are not sufficient evidence that a source is not a microphone.
    let source = Rc::new(RefCell::new(None));
    let source_result = Rc::clone(&source);
    let _source_operation =
        context
            .introspect()
            .get_source_info_by_name(&monitor.source_name, move |result| match result {
                ListResult::Item(info) => {
                    *source_result.borrow_mut() = Some(Some((info.index, info.monitor_of_sink)));
                }
                ListResult::Error | ListResult::End => {
                    if source_result.borrow().is_none() {
                        *source_result.borrow_mut() = Some(None);
                    }
                }
            });
    let (source_index, monitor_of_sink) = wait_for(mainloop, control, deadline, || {
        check_context(context)?;
        Ok(source.borrow_mut().take())
    })?
    .ok_or(SystemAudioCaptureError::NoPlaybackDevice)?;
    if !monitor.matches(source_index, monitor_of_sink) {
        return Err(SystemAudioCaptureError::NoPlaybackDevice);
    }
    Ok(monitor)
}

fn check_context(context: &Context) -> Result<(), SystemAudioCaptureError> {
    if context.get_state() == ContextState::Ready {
        Ok(())
    } else {
        Err(SystemAudioCaptureError::AudioServerUnavailable)
    }
}

fn valid_device_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('\0')
}

/// PulseAudio's peek boundaries need not match the requested fragment size.
/// Assemble complete 20 ms frames so tiny native fragments do not consume
/// extra slots in the bounded send queue. Keep at most one partial frame.
struct PcmFrameAssembler {
    pending: Vec<u8>,
    chunk_bytes: usize,
    max_buffer_bytes: usize,
}

impl PcmFrameAssembler {
    fn new(chunk_bytes: usize, max_buffer_bytes: usize) -> Self {
        Self {
            pending: Vec::with_capacity(chunk_bytes),
            chunk_bytes,
            max_buffer_bytes,
        }
    }

    fn forward_fragment(
        &mut self,
        data: Option<&[u8]>,
        bytes: usize,
        control: &WorkerControl,
        ingress: &AudioIngress,
    ) -> Result<bool, SystemAudioCaptureFailure> {
        if control.cancelled.load(Ordering::SeqCst) {
            self.pending.clear();
            return Ok(false);
        }
        // Reject an oversized fragment before copying or allocating for it.
        if bytes > self.max_buffer_bytes {
            pipeline_log!(
                "capture oversized fragment bytes={} maxBytes={}",
                bytes,
                self.max_buffer_bytes
            );
            return Err(SystemAudioCaptureFailure::Backpressure);
        }
        if !bytes.is_multiple_of(2) {
            return Err(SystemAudioCaptureFailure::AudioProcessingFailed);
        }
        let mut offset = 0;
        while offset < bytes {
            if control.cancelled.load(Ordering::SeqCst) {
                self.pending.clear();
                return Ok(false);
            }
            let count = (self.chunk_bytes - self.pending.len()).min(bytes - offset);
            match data {
                Some(data) => self
                    .pending
                    .extend_from_slice(&data[offset..offset + count]),
                None => self.pending.resize(self.pending.len() + count, 0),
            }
            offset += count;
            if self.pending.len() == self.chunk_bytes {
                if control.cancelled.load(Ordering::SeqCst) {
                    self.pending.clear();
                    return Ok(false);
                }
                let pcm =
                    std::mem::replace(&mut self.pending, Vec::with_capacity(self.chunk_bytes));
                match ingress.try_send(pcm) {
                    Ok(()) => {}
                    Err(AudioIngressError::Backpressure) => {
                        return Err(SystemAudioCaptureFailure::Backpressure);
                    }
                    Err(AudioIngressError::Closed) => return Ok(false),
                }
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::send_pipeline::AudioSendPipeline;
    use std::io::Write;
    use std::process::{Child, Command, Stdio};
    use tokio::sync::mpsc;

    #[test]
    fn only_the_selected_sinks_real_monitor_is_accepted() {
        let monitor = MonitorSource {
            sink_index: 4,
            source_index: 9,
            source_name: "an_arbitrary_name".into(),
        };
        assert!(monitor.matches(9, Some(4)));
        assert!(
            !monitor.matches(9, None),
            "a microphone has no monitor sink"
        );
        assert!(!monitor.matches(9, Some(5)), "another sink is not selected");
        assert!(!monitor.matches(10, Some(4)), "source identity must match");
        assert!(!valid_device_name(""));
        assert!(!valid_device_name("sink\0name"));
    }

    #[test]
    fn cancelled_start_holds_its_slot_until_native_teardown_finishes() {
        let capture = LinuxSystemAudioCapture::new();
        let first = capture.reserve_worker().unwrap();
        assert!(matches!(
            capture.reserve_worker(),
            Err(SystemAudioCaptureError::AlreadyRunning)
        ));
        drop(CancelStartOnDrop(Some(Arc::clone(&first))));
        assert!(first.cancelled.load(Ordering::SeqCst));
        assert!(matches!(
            capture.reserve_worker(),
            Err(SystemAudioCaptureError::PreviousCaptureStopping)
        ));
        drop(FinishWorkerOnDrop(Arc::clone(&first)));
        let second = capture.reserve_worker().unwrap();
        assert!(!Arc::ptr_eq(&first, &second));
        assert!(!second.cancelled.load(Ordering::SeqCst));
        // Dropping the final handle also stops an active native worker.
        drop(capture);
        assert!(second.cancelled.load(Ordering::SeqCst));
    }

    fn recording_pipeline() -> (AudioSendPipeline, mpsc::Receiver<Vec<u8>>) {
        let (tx, rx) = mpsc::channel(32);
        let pipeline = AudioSendPipeline::spawn(
            move |pcm| {
                let tx = tx.clone();
                async move { tx.send(pcm).await }
            },
            |_| {},
        );
        (pipeline, rx)
    }

    #[tokio::test]
    async fn tiny_fragments_use_audio_duration_instead_of_native_fragment_count() {
        for rate in [16_000, 24_000] {
            let (pipeline, mut rx) = recording_pipeline();
            let ingress = pipeline.ingress().unwrap();
            let control = WorkerControl::default();
            let chunk_bytes = rate * 2 * FRAGMENT_MS / 1000;
            let mut frames = PcmFrameAssembler::new(chunk_bytes, rate * 2 * MAX_BUFFER_MS / 1000);
            let pcm: Vec<u8> = (0..chunk_bytes * 10)
                .map(|index| (index % 251) as u8)
                .collect();
            // Forty 5 ms fragments arrive before the sender can run. They
            // need ten queue slots, not forty, and retain every sample.
            for fragment in pcm.chunks(chunk_bytes / 4) {
                assert!(frames
                    .forward_fragment(Some(fragment), fragment.len(), &control, &ingress)
                    .unwrap());
                assert!(frames.pending.len() < chunk_bytes);
            }
            assert!(frames.pending.is_empty());
            assert!(pipeline.finish(Duration::from_secs(1)).await);
            let mut actual = Vec::new();
            while let Ok(frame) = rx.try_recv() {
                assert_eq!(frame.len(), chunk_bytes);
                actual.extend(frame);
            }
            assert_eq!(actual, pcm);
        }
    }

    #[tokio::test]
    async fn frames_preserve_sample_order_across_data_fragments_and_holes() {
        let (pipeline, mut rx) = recording_pipeline();
        let ingress = pipeline.ingress().unwrap();
        let control = WorkerControl::default();
        let mut frames = PcmFrameAssembler::new(8, 32);
        let pcm: Vec<u8> = (0..18).collect();
        assert!(frames
            .forward_fragment(Some(&pcm[..6]), 6, &control, &ingress)
            .unwrap());
        assert!(frames
            .forward_fragment(None, 6, &control, &ingress)
            .unwrap());
        assert!(frames
            .forward_fragment(Some(&pcm[6..16]), 10, &control, &ingress)
            .unwrap());
        assert_eq!(frames.pending, pcm[10..16]);
        assert!(frames
            .forward_fragment(Some(&pcm[16..]), 2, &control, &ingress)
            .unwrap());
        assert!(frames.pending.is_empty());
        assert!(pipeline.finish(Duration::from_secs(1)).await);
        assert_eq!(rx.recv().await.unwrap(), vec![0, 1, 2, 3, 4, 5, 0, 0]);
        assert_eq!(rx.recv().await.unwrap(), vec![0, 0, 0, 0, 6, 7, 8, 9]);
        assert_eq!(
            rx.recv().await.unwrap(),
            vec![10, 11, 12, 13, 14, 15, 16, 17]
        );
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn oversized_and_misaligned_fragments_are_rejected_before_copying() {
        let (pipeline, mut rx) = recording_pipeline();
        let ingress = pipeline.ingress().unwrap();
        let control = WorkerControl::default();
        let mut frames = PcmFrameAssembler::new(4, 16);
        assert!(frames
            .forward_fragment(Some(&[4, 5]), 2, &control, &ingress)
            .unwrap());
        assert_eq!(
            frames.forward_fragment(None, 18, &control, &ingress),
            Err(SystemAudioCaptureFailure::Backpressure)
        );
        assert_eq!(
            frames.forward_fragment(None, 3, &control, &ingress),
            Err(SystemAudioCaptureFailure::AudioProcessingFailed)
        );
        assert_eq!(frames.pending, vec![4, 5]);
        assert!(pipeline.finish(Duration::from_secs(1)).await);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn partial_frames_are_discarded_on_stop_or_cancellation_and_never_reused() {
        for cancelled in [false, true] {
            let (pipeline, mut rx) = recording_pipeline();
            let ingress = pipeline.ingress().unwrap();
            let control = WorkerControl::default();
            let mut frames = PcmFrameAssembler::new(4, 16);
            assert!(frames
                .forward_fragment(Some(&[98, 99]), 2, &control, &ingress)
                .unwrap());
            if cancelled {
                control.cancelled.store(true, Ordering::SeqCst);
                assert!(!frames
                    .forward_fragment(None, 4, &control, &ingress)
                    .unwrap());
                assert!(frames.pending.is_empty());
            }
            // The native run owns the assembler; every exit drops its tail.
            drop(frames);
            let control = WorkerControl::default();
            let mut restarted = PcmFrameAssembler::new(4, 16);
            assert!(restarted
                .forward_fragment(Some(&[1, 2, 3, 4]), 4, &control, &ingress)
                .unwrap());
            assert!(pipeline.finish(Duration::from_secs(1)).await);
            assert_eq!(rx.recv().await.unwrap(), vec![1, 2, 3, 4]);
            assert!(rx.try_recv().is_err());
        }
    }

    #[tokio::test]
    async fn complete_frames_still_fail_when_the_twenty_slot_queue_is_full() {
        let (pipeline, _rx) = recording_pipeline();
        let ingress = pipeline.ingress().unwrap();
        let control = WorkerControl::default();
        let mut frames = PcmFrameAssembler::new(640, 8_000);
        // No await: the current-thread sender cannot drain these 400 ms.
        for _ in 0..20 {
            assert!(frames
                .forward_fragment(None, 640, &control, &ingress)
                .unwrap());
        }
        assert!(frames
            .forward_fragment(None, 320, &control, &ingress)
            .unwrap());
        assert_eq!(
            frames.forward_fragment(None, 320, &control, &ingress),
            Err(SystemAudioCaptureFailure::Backpressure)
        );
        assert!(!frames
            .forward_fragment(None, 640, &control, &ingress)
            .unwrap());
        pipeline.stop();
    }

    struct TestPlayback(Child);

    impl Drop for TestPlayback {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn play_test_tone() -> (tempfile::NamedTempFile, TestPlayback) {
        let mut audio = tempfile::NamedTempFile::new().unwrap();
        // Synthetic PCM only; never write the audio captured from a device.
        let pcm: Vec<u8> = (0..(48_000 * 8))
            .flat_map(|index| {
                let phase = std::f64::consts::TAU * 997.0 * f64::from(index) / 48_000.0;
                let sample = ((phase.sin() * 6_000.0) as i16).to_le_bytes();
                [sample[0], sample[1], sample[0], sample[1]]
            })
            .collect();
        audio.write_all(&pcm).unwrap();
        audio.flush().unwrap();
        let child = Command::new("paplay")
            .args([
                "--raw",
                "--format=s16le",
                "--rate=48000",
                "--channels=2",
                "--device=@DEFAULT_SINK@",
            ])
            .arg(audio.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("Linux audio smoke needs paplay from pulseaudio-utils");
        (audio, TestPlayback(child))
    }

    fn matches_tone(pcm: &[u8], sample_rate: u32) -> bool {
        let samples: Vec<f64> = pcm
            .as_chunks::<2>()
            .0
            .iter()
            .map(|sample| f64::from(i16::from_le_bytes(*sample)) / 32768.0)
            .collect();
        let count = samples.len() as f64;
        let mut sine = 0.0;
        let mut cosine = 0.0;
        let mut energy = 0.0;
        let mut peak: f64 = 0.0;
        for (index, sample) in samples.iter().enumerate() {
            let phase = std::f64::consts::TAU * 997.0 * index as f64 / f64::from(sample_rate);
            sine += sample * phase.sin();
            cosine += sample * phase.cos();
            energy += sample * sample;
            peak = peak.max(sample.abs());
        }
        let rms = (energy / count).sqrt();
        let tone_fraction = 2.0 * (sine * sine + cosine * cosine) / (count * energy);
        rms > 0.02 && rms < 0.4 && peak < 0.9 && tone_fraction > 0.8
    }

    #[test]
    fn tone_detector_rejects_silence_wrong_rates_and_clipping() {
        let tone = |amplitude: f64| -> Vec<u8> {
            (0..4_000)
                .flat_map(|index| {
                    let phase = std::f64::consts::TAU * 997.0 * f64::from(index) / 16_000.0;
                    ((phase.sin() * amplitude) as i16).to_le_bytes()
                })
                .collect()
        };
        assert!(matches_tone(&tone(6_000.0), 16_000));
        assert!(!matches_tone(&tone(6_000.0), 24_000));
        assert!(!matches_tone(&[0; 8_000], 16_000));
        assert!(!matches_tone(&tone(32_767.0), 16_000));
    }

    async fn expect_native_silence(
        pipeline: &AudioSendPipeline,
        rx: &mut mpsc::Receiver<Vec<u8>>,
        rate: u32,
        stage: &str,
    ) {
        tokio::time::timeout(Duration::from_secs(6), async {
            while let Some(pcm) = rx.recv().await {
                assert_eq!(pcm.len(), rate as usize * 2 * FRAGMENT_MS / 1000);
                assert!(pcm.len().is_multiple_of(2));
                let silent = pcm
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .all(|sample| i16::from_le_bytes(*sample).unsigned_abs() <= 2);
                if silent && pipeline.input_activity() == (true, false) {
                    return;
                }
            }
            panic!("capture ended before silent PCM was observed");
        })
        .await
        .expect("the monitor must publish silent PCM and let old sound activity expire");
        eprintln!("linux native rate={rate} stage={stage} pcm=true sound=false");
    }

    /// Run under scripts/linux-audio-smoke.sh: it supplies a private server,
    /// a null playback sink, and a deliberately different default source.
    /// No provider credentials, device capture, or display are needed.
    #[tokio::test]
    #[ignore = "requires an isolated PulseAudio server and paplay"]
    async fn native_monitor_capture_is_pcm16_and_restarts() {
        let capture = LinuxSystemAudioCapture::new();
        for rate in [16_000, 24_000] {
            let (pipeline, mut rx) = recording_pipeline();
            assert_eq!(
                pipeline.input_activity(),
                (false, false),
                "a fresh capture generation must not inherit old PCM or sound activity"
            );
            let (failure_tx, mut failures) = CaptureFailureSender::channel();
            tokio::time::timeout(
                Duration::from_secs(7),
                capture.start(
                    pipeline.ingress().unwrap(),
                    failure_tx,
                    AudioCaptureFormat::pcm16_mono(rate).unwrap(),
                ),
            )
            .await
            .expect("monitor capture startup is bounded")
            .expect("default output monitor should open");
            expect_native_silence(&pipeline, &mut rx, rate, "before-playback").await;
            let (_audio, playback) = play_test_tone();
            tokio::time::timeout(Duration::from_secs(6), async {
                let mut window = Vec::new();
                while let Some(pcm) = rx.recv().await {
                    assert_eq!(pcm.len(), rate as usize * 2 * FRAGMENT_MS / 1000);
                    assert!(pcm.len().is_multiple_of(2));
                    window.extend(pcm);
                    if window.len() >= rate as usize / 2 {
                        if matches_tone(&window, rate) {
                            return;
                        }
                        window.clear();
                    }
                }
                panic!("capture ended before the playback tone was detected");
            })
            .await
            .expect("monitor must contain the 997 Hz output tone at the requested sample rate");
            assert_eq!(pipeline.input_activity(), (true, true));
            eprintln!("linux native rate={rate} stage=playback pcm=true sound=true");
            drop(playback);
            expect_native_silence(&pipeline, &mut rx, rate, "after-playback").await;
            let stop_started = Instant::now();
            tokio::time::timeout(Duration::from_secs(1), capture.stop())
                .await
                .expect("stop must release the monitor without waiting for more audio");
            eprintln!(
                "linux native rate={rate} stage=stop elapsedMs={}",
                milliseconds(stop_started, Instant::now())
            );
            while rx.try_recv().is_ok() {}
            tokio::time::sleep(Duration::from_millis(75)).await;
            // Anything accepted before stop may finish its bounded send.
            while rx.try_recv().is_ok() {}
            tokio::time::sleep(Duration::from_millis(75)).await;
            assert!(
                rx.try_recv().is_err(),
                "stopped capture must emit no new PCM"
            );
            assert!(
                failures.try_recv().is_err(),
                "capture should not report a failure"
            );
            tokio::time::sleep(Duration::from_millis(2_100)).await;
            assert_eq!(
                pipeline.input_activity(),
                (false, false),
                "stopped capture must not keep old PCM activity alive"
            );
            eprintln!("linux native rate={rate} stage=stopped pcm=false sound=false");
            pipeline.stop();
        }
    }
}
