//! Linux system audio through PulseAudio (including PipeWire's PulseAudio
//! server). Resolve and verify the default output's monitor explicitly; the
//! default recording source is opened only for explicit microphone capture.
//! The server converts the selected source to the requested mono PCM16LE rate.
//!
//! All libpulse objects stay on one worker thread. A nonblocking mainloop
//! keeps startup, pause/stop and cancelled start futures bounded even when
//! the server is silent or disappears. SessionManager implements pause and
//! resume by stopping this worker and starting a fresh monitor stream.

use crate::audio::send_pipeline::{AudioIngress, AudioIngressError};
use crate::audio::{
    AudioCaptureFormat, CaptureFailureSender, SystemAudioCaptureError, SystemAudioCaptureFailure,
};
use crate::core::audio_input::AudioSource;
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
        self.start_input(audio_ingress, failure_tx, format, AudioSource::System)
            .await
    }

    pub async fn start_input(
        &self,
        audio_ingress: AudioIngress,
        failure_tx: CaptureFailureSender,
        format: AudioCaptureFormat,
        input: AudioSource,
    ) -> Result<(), SystemAudioCaptureError> {
        AudioCaptureFormat::pcm16_mono(format.sample_rate_hz)?;
        let control = self.reserve_worker()?;
        let mut cancel_start = CancelStartOnDrop(Some(Arc::clone(&control)));
        let (ready_tx, ready_rx) = oneshot::channel();
        let worker_control = Arc::clone(&control);
        if std::thread::Builder::new()
            .name(
                match input {
                    AudioSource::System => "mimi-system-audio",
                    AudioSource::Microphone => "mimi-microphone",
                }
                .into(),
            )
            .spawn(move || {
                // Constructed first, dropped last: finished means libpulse
                // has disconnected and released the stream and context.
                let _finished = FinishWorkerOnDrop(Arc::clone(&worker_control));
                let mut capture = match PulseCapture::connect(&worker_control, format, input) {
                    Ok(capture) => capture,
                    Err(error) => {
                        let _ = ready_tx.send(Err(input_error(input, error)));
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
            return Err(input_error(
                input,
                SystemAudioCaptureError::NativeStartFailed,
            ));
        }

        ready_rx
            .await
            .map_err(|_| input_error(input, SystemAudioCaptureError::NativeStartFailed))??;
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

fn input_error(input: AudioSource, error: SystemAudioCaptureError) -> SystemAudioCaptureError {
    match (input, error) {
        (AudioSource::Microphone, SystemAudioCaptureError::NativeStartFailed) => {
            SystemAudioCaptureError::MicrophoneStartFailed
        }
        (_, error) => error,
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
    source_index: u32,
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
        input: AudioSource,
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

        let (source_index, source_name) = match input {
            AudioSource::System => {
                let monitor = resolve_monitor(&mut mainloop, &context, control, deadline)?;
                (monitor.source_index, monitor.source_name)
            }
            AudioSource::Microphone => {
                resolve_microphone(&mut mainloop, &context, control, deadline)?
            }
        };
        let spec = Spec {
            format: Format::S16le,
            channels: 1,
            rate: format.sample_rate_hz,
        };
        let chunk_bytes = format.sample_rate_hz as usize * 2 * FRAGMENT_MS / 1000;
        let max_buffer_bytes = format.sample_rate_hz as usize * 2 * MAX_BUFFER_MS / 1000;
        let mut stream = Stream::new(
            &mut context,
            match input {
                AudioSource::System => "System audio",
                AudioSource::Microphone => "Microphone",
            },
            &spec,
            None,
        )
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
                Some(&source_name),
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
        if stream.get_device_index() != Some(source_index)
            || stream.get_sample_spec() != Some(&spec)
        {
            return Err(SystemAudioCaptureError::UnsupportedAudioFormat);
        }

        Ok(Self {
            stream,
            context,
            mainloop,
            source_index,
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
                || self.stream.get_device_index() != Some(self.source_index)
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

/// Resolve only the server's selected recording input. Reject a monitor even
/// when it is configured as the default source; never silently capture output.
fn resolve_microphone(
    mainloop: &mut Mainloop,
    context: &Context,
    control: &WorkerControl,
    deadline: Instant,
) -> Result<(u32, String), SystemAudioCaptureError> {
    let source_name = Rc::new(RefCell::new(None));
    let source_name_result = Rc::clone(&source_name);
    let _server_operation = context.introspect().get_server_info(move |info| {
        *source_name_result.borrow_mut() =
            Some(info.default_source_name.as_deref().map(str::to_owned));
    });
    let source_name = wait_for(mainloop, control, deadline, || {
        check_context(context)?;
        Ok(source_name.borrow_mut().take())
    })?
    .filter(|name| valid_device_name(name))
    .ok_or(SystemAudioCaptureError::NoMicrophoneDevice)?;
    let source = Rc::new(RefCell::new(None));
    let source_result = Rc::clone(&source);
    let _source_operation =
        context
            .introspect()
            .get_source_info_by_name(&source_name, move |result| match result {
                ListResult::Item(info) => {
                    *source_result.borrow_mut() = Some(Some((info.index, info.monitor_of_sink)));
                }
                ListResult::Error | ListResult::End => {
                    if source_result.borrow().is_none() {
                        *source_result.borrow_mut() = Some(None);
                    }
                }
            });
    let (index, monitor) = wait_for(mainloop, control, deadline, || {
        check_context(context)?;
        Ok(source.borrow_mut().take())
    })?
    .ok_or(SystemAudioCaptureError::NoMicrophoneDevice)?;
    if !is_microphone_source(index, monitor) {
        return Err(SystemAudioCaptureError::NoMicrophoneDevice);
    }
    Ok((index, source_name))
}

fn is_microphone_source(index: u32, monitor_of_sink: Option<u32>) -> bool {
    index != u32::MAX && monitor_of_sink.is_none()
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
    fn microphone_requires_a_real_input_and_rejects_output_monitors() {
        assert!(is_microphone_source(9, None));
        assert!(!is_microphone_source(9, Some(4)));
        assert!(!is_microphone_source(u32::MAX, None));
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
        play_test_tone_on("@DEFAULT_SINK@")
    }

    fn play_test_tone_on(sink: &str) -> (tempfile::NamedTempFile, TestPlayback) {
        play_test_frequency_on(sink, 997.0)
    }

    fn play_test_frequency_on(
        sink: &str,
        frequency: f64,
    ) -> (tempfile::NamedTempFile, TestPlayback) {
        let mut audio = tempfile::NamedTempFile::new().unwrap();
        // Synthetic PCM only; never write the audio captured from a device.
        let pcm: Vec<u8> = (0..(48_000 * 8))
            .flat_map(|index| {
                let phase = std::f64::consts::TAU * frequency * f64::from(index) / 48_000.0;
                let sample = ((phase.sin() * 6_000.0) as i16).to_le_bytes();
                [sample[0], sample[1], sample[0], sample[1]]
            })
            .collect();
        audio.write_all(&pcm).unwrap();
        audio.flush().unwrap();
        let child = Command::new("paplay")
            .args(["--raw", "--format=s16le", "--rate=48000", "--channels=2"])
            .arg(format!("--device={sink}"))
            .arg(audio.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("Linux audio smoke needs paplay from pulseaudio-utils");
        (audio, TestPlayback(child))
    }

    fn matches_tone(pcm: &[u8], sample_rate: u32) -> bool {
        matches_frequency(pcm, sample_rate, 997.0)
    }

    fn matches_frequency(pcm: &[u8], sample_rate: u32, frequency: f64) -> bool {
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
            let phase = std::f64::consts::TAU * frequency * index as f64 / f64::from(sample_rate);
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

    fn isolated_audio_directory() -> std::path::PathBuf {
        let directory = std::path::PathBuf::from(
            std::env::var("MIMI_TEST_AUDIO_DIRECTORY").expect("private audio harness is required"),
        );
        assert_eq!(
            std::fs::read_to_string(directory.join("isolated-server")).unwrap(),
            "mimi isolated synthetic audio smoke\n"
        );
        let socket = directory.join("pulse.sock");
        assert_eq!(
            std::env::var("PULSE_SERVER").unwrap(),
            format!("unix:{}", socket.display())
        );
        assert!(directory.is_absolute());
        assert_eq!(directory.canonicalize().unwrap(), directory);
        assert_eq!(
            std::env::var("PULSE_RUNTIME_PATH").unwrap(),
            directory.join("runtime").to_str().unwrap()
        );
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&directory).unwrap().permissions().mode() & 0o077,
            0
        );
        directory
    }

    fn pactl(args: &[&str]) -> String {
        let output = Command::new("timeout")
            .args(["2s", "pactl"])
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "pactl {args:?} failed");
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    struct TestSink(String);

    impl TestSink {
        fn new(name: &str) -> Self {
            Self(pactl(&[
                "load-module",
                "module-null-sink",
                &format!("sink_name={name}"),
            ]))
        }
    }

    impl Drop for TestSink {
        fn drop(&mut self) {
            let _ = Command::new("timeout")
                .args(["2s", "pactl"])
                .args(["unload-module", &self.0])
                .output();
        }
    }

    struct RestoreDefaultSink;

    impl Drop for RestoreDefaultSink {
        fn drop(&mut self) {
            let _ = Command::new("timeout")
                .args(["2s", "pactl"])
                .args(["set-default-sink", "mimi-output"])
                .output();
        }
    }

    async fn expect_monitor_frequency(rx: &mut mpsc::Receiver<Vec<u8>>, rate: u32, frequency: f64) {
        tokio::time::timeout(Duration::from_secs(6), async {
            let mut window = Vec::new();
            while let Some(pcm) = rx.recv().await {
                assert_eq!(pcm.len(), rate as usize * 2 * FRAGMENT_MS / 1000);
                window.extend(pcm);
                if window.len() >= rate as usize / 2 {
                    if matches_frequency(&window, rate, frequency) {
                        return;
                    }
                    window.clear();
                }
            }
            panic!("monitor ended before its assigned tone arrived");
        })
        .await
        .expect("capture must retain the selected monitor's dominant tone");
    }

    async fn stop_and_assert_quiet(
        capture: &LinuxSystemAudioCapture,
        pipeline: &AudioSendPipeline,
        rx: &mut mpsc::Receiver<Vec<u8>>,
    ) {
        tokio::time::timeout(Duration::from_secs(1), capture.stop())
            .await
            .expect("native teardown must remain bounded after a device/server loss");
        // Drain only work accepted before stop. Keep the pipeline open so a
        // late native send cannot be hidden by closing its receiver.
        while rx.try_recv().is_ok() {}
        tokio::time::sleep(Duration::from_millis(75)).await;
        while rx.try_recv().is_ok() {}
        tokio::time::sleep(Duration::from_millis(75)).await;
        assert!(rx.try_recv().is_err(), "stopped monitor emitted new PCM");
        tokio::time::sleep(Duration::from_millis(2_100)).await;
        assert_eq!(pipeline.input_activity(), (false, false));
        let old_ingress = pipeline.ingress().unwrap();
        pipeline.stop();
        assert_eq!(
            old_ingress.try_send(vec![0, 0]),
            Err(AudioIngressError::Closed)
        );
        tokio::time::timeout(Duration::from_secs(1), async {
            while pipeline.pending_pcm_gate().has_pending() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("retired generation must release all pending PCM");
    }

    /// Changing the default output must not move an already-open DONT_MOVE
    /// stream. Only an explicit stop/start selects the new output monitor.
    #[tokio::test]
    #[ignore = "requires an isolated PulseAudio server and paplay"]
    async fn native_default_output_switch_pins_monitor_until_restart() {
        isolated_audio_directory();
        let second_sink = TestSink::new("mimi-switch-output");
        let _restore = RestoreDefaultSink;
        let capture = LinuxSystemAudioCapture::new();
        for rate in [16_000, 24_000] {
            pactl(&["set-default-sink", "mimi-output"]);
            let (pipeline, mut rx) = recording_pipeline();
            let (failure, mut failures) = CaptureFailureSender::channel();
            capture
                .start(
                    pipeline.ingress().unwrap(),
                    failure,
                    AudioCaptureFormat::pcm16_mono(rate).unwrap(),
                )
                .await
                .unwrap();
            expect_native_silence(&pipeline, &mut rx, rate, "switch-before-playback").await;
            let (_first_audio, first_playback) = play_test_frequency_on("mimi-output", 997.0);
            let (_second_audio, second_playback) =
                play_test_frequency_on("mimi-switch-output", 613.0);
            expect_monitor_frequency(&mut rx, rate, 997.0).await;
            pactl(&["set-default-sink", "mimi-switch-output"]);
            // Old native/send buffers could still contain 997 Hz. Stop that
            // tone, observe silence, then require a fresh distinct tone on A
            // while B is audible. Pre-switch PCM cannot satisfy this check.
            drop(first_playback);
            expect_native_silence(&pipeline, &mut rx, rate, "switch-pinned-silent").await;
            let (_fresh_audio, fresh_playback) = play_test_frequency_on("mimi-output", 811.0);
            expect_monitor_frequency(&mut rx, rate, 811.0).await;
            assert!(failures.try_recv().is_err());
            stop_and_assert_quiet(&capture, &pipeline, &mut rx).await;
            drop(fresh_playback);
            drop(second_playback);

            let (restarted, mut restarted_rx) = recording_pipeline();
            assert_eq!(restarted.input_activity(), (false, false));
            let (failure, mut failures) = CaptureFailureSender::channel();
            capture
                .start(
                    restarted.ingress().unwrap(),
                    failure,
                    AudioCaptureFormat::pcm16_mono(rate).unwrap(),
                )
                .await
                .unwrap();
            expect_native_silence(&restarted, &mut restarted_rx, rate, "switch-restarted").await;
            let (_audio, playback) = play_test_frequency_on("mimi-switch-output", 613.0);
            expect_monitor_frequency(&mut restarted_rx, rate, 613.0).await;
            stop_and_assert_quiet(&capture, &restarted, &mut restarted_rx).await;
            assert!(failures.try_recv().is_err());
            drop(playback);
            eprintln!("linux native route rate={rate} pinned=true restartedOnNewOutput=true");
        }
        drop(second_sink);
    }

    /// Removing the selected sink must fail instead of migrating to another
    /// audible output. Recreating it allows an explicit fresh generation.
    #[tokio::test]
    #[ignore = "requires an isolated PulseAudio server and paplay"]
    async fn native_removed_monitor_fails_and_recovers_without_fallback() {
        isolated_audio_directory();
        let _restore = RestoreDefaultSink;
        let capture = LinuxSystemAudioCapture::new();
        for rate in [16_000, 24_000] {
            let sink = TestSink::new("mimi-removable-output");
            pactl(&["set-default-sink", "mimi-removable-output"]);
            let (pipeline, mut rx) = recording_pipeline();
            let (failure, mut failures) = CaptureFailureSender::channel();
            capture
                .start(
                    pipeline.ingress().unwrap(),
                    failure,
                    AudioCaptureFormat::pcm16_mono(rate).unwrap(),
                )
                .await
                .unwrap();
            let (_audio, playback) = play_test_frequency_on("mimi-removable-output", 997.0);
            expect_monitor_frequency(&mut rx, rate, 997.0).await;
            drop(playback);
            let (_other_audio, other_playback) = play_test_frequency_on("mimi-output", 613.0);
            // Set the fallback first; removal must still report failure.
            pactl(&["set-default-sink", "mimi-output"]);
            drop(sink);
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(3), failures.recv())
                    .await
                    .expect("removed monitor must report a bounded failure"),
                Some(SystemAudioCaptureFailure::NativeStopped)
            );
            stop_and_assert_quiet(&capture, &pipeline, &mut rx).await;
            assert!(
                failures.try_recv().is_err(),
                "failure must only be reported once"
            );
            drop(other_playback);

            let restored_sink = TestSink::new("mimi-removable-output");
            pactl(&["set-default-sink", "mimi-removable-output"]);
            let (restarted, mut restarted_rx) = recording_pipeline();
            assert_eq!(restarted.input_activity(), (false, false));
            let (failure, mut failures) = CaptureFailureSender::channel();
            capture
                .start(
                    restarted.ingress().unwrap(),
                    failure,
                    AudioCaptureFormat::pcm16_mono(rate).unwrap(),
                )
                .await
                .unwrap();
            expect_native_silence(&restarted, &mut restarted_rx, rate, "monitor-restored").await;
            let (_audio, playback) = play_test_frequency_on("mimi-removable-output", 997.0);
            expect_monitor_frequency(&mut restarted_rx, rate, 997.0).await;
            stop_and_assert_quiet(&capture, &restarted, &mut restarted_rx).await;
            assert!(failures.try_recv().is_err());
            drop(playback);
            drop(restored_sink);
            eprintln!("linux native monitor-loss rate={rate} failedClosed=true recovered=true");
        }
    }

    /// The harness runs this test last, in its own test process. It may stop
    /// only its own child on the marked private socket, never the desktop server.
    #[tokio::test]
    #[ignore = "requires the private server lifecycle setup in linux-audio-smoke.sh"]
    async fn native_server_disconnect_fails_and_recovers() {
        let directory = isolated_audio_directory();
        async fn start_private_server(directory: &std::path::Path) -> TestPlayback {
            let socket = directory.join("pulse.sock");
            let child = Command::new("pulseaudio")
                .args([
                    "--daemonize=no",
                    "--exit-idle-time=-1",
                    "--use-pid-file=no",
                    "-n",
                ])
                .arg(format!(
                    "--load=module-native-protocol-unix socket={} auth-anonymous=1",
                    socket.display()
                ))
                .arg("--load=module-null-sink sink_name=mimi-output")
                .arg(format!(
                    "--log-target=file:{}/restarted-pulse.log",
                    directory.display()
                ))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let server = TestPlayback(child);
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    if Command::new("timeout")
                        .args(["2s", "pactl"])
                        .arg("info")
                        .output()
                        .unwrap()
                        .status
                        .success()
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            })
            .await
            .expect("private replacement server must become ready");
            pactl(&["set-default-sink", "mimi-output"]);
            server
        }
        let capture = LinuxSystemAudioCapture::new();
        for rate in [16_000, 24_000] {
            let server = start_private_server(&directory).await;
            let (pipeline, mut rx) = recording_pipeline();
            let (failure, mut failures) = CaptureFailureSender::channel();
            capture
                .start(
                    pipeline.ingress().unwrap(),
                    failure,
                    AudioCaptureFormat::pcm16_mono(rate).unwrap(),
                )
                .await
                .unwrap();
            let (_audio, playback) = play_test_tone();
            expect_monitor_frequency(&mut rx, rate, 997.0).await;
            drop(playback);
            drop(server);
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(3), failures.recv())
                    .await
                    .expect("server loss must report a bounded failure"),
                Some(SystemAudioCaptureFailure::NativeStopped)
            );
            stop_and_assert_quiet(&capture, &pipeline, &mut rx).await;
            assert!(failures.try_recv().is_err());
            let (unavailable, _rx) = recording_pipeline();
            let (failure, _) = CaptureFailureSender::channel();
            assert_eq!(
                tokio::time::timeout(
                    Duration::from_secs(7),
                    capture.start(
                        unavailable.ingress().unwrap(),
                        failure,
                        AudioCaptureFormat::pcm16_mono(rate).unwrap(),
                    ),
                )
                .await
                .expect("missing server must fail within the startup bound"),
                Err(SystemAudioCaptureError::AudioServerUnavailable)
            );
            capture.stop().await;
            unavailable.stop();

            let _server = start_private_server(&directory).await;
            let (restarted, mut restarted_rx) = recording_pipeline();
            assert_eq!(restarted.input_activity(), (false, false));
            let (failure, mut failures) = CaptureFailureSender::channel();
            capture
                .start(
                    restarted.ingress().unwrap(),
                    failure,
                    AudioCaptureFormat::pcm16_mono(rate).unwrap(),
                )
                .await
                .unwrap();
            expect_native_silence(&restarted, &mut restarted_rx, rate, "server-restored").await;
            let (_audio, playback) = play_test_tone();
            expect_monitor_frequency(&mut restarted_rx, rate, 997.0).await;
            stop_and_assert_quiet(&capture, &restarted, &mut restarted_rx).await;
            assert!(failures.try_recv().is_err());
            drop(playback);
            eprintln!("linux native server-loss rate={rate} unavailable=true recovered=true");
        }
    }

    /// Both native streams run concurrently with different synthetic tones.
    /// Matching the dominant tone rejects swapped streams and mixed PCM.
    #[tokio::test]
    #[ignore = "requires an isolated PulseAudio server with mimi-input and paplay"]
    async fn native_dual_inputs_keep_audio_separate_and_restart() {
        isolated_audio_directory();
        struct RestoreDefaultSource;
        impl Drop for RestoreDefaultSource {
            fn drop(&mut self) {
                let _ = Command::new("timeout")
                    .args(["2s", "pactl"])
                    .args(["set-default-source", "mimi-microphone.monitor"])
                    .status();
            }
        }
        let _restore = RestoreDefaultSource;
        assert!(Command::new("timeout")
            .args(["2s", "pactl"])
            .args(["set-default-source", "mimi-input"])
            .status()
            .unwrap()
            .success());
        async fn expect_frequency(rx: &mut mpsc::Receiver<Vec<u8>>, rate: u32, frequency: f64) {
            tokio::time::timeout(Duration::from_secs(6), async {
                let mut window = Vec::new();
                while let Some(pcm) = rx.recv().await {
                    assert_eq!(pcm.len(), rate as usize * 2 * FRAGMENT_MS / 1000);
                    window.extend(pcm);
                    if window.len() >= rate as usize / 2 {
                        if matches_frequency(&window, rate, frequency) {
                            return;
                        }
                        window.clear();
                    }
                }
                panic!("independent capture ended before its assigned tone arrived");
            })
            .await
            .expect("each source must retain its own dominant tone");
        }
        let system = LinuxSystemAudioCapture::new();
        let microphone = LinuxSystemAudioCapture::new();
        for rate in [16_000, 24_000] {
            let (system_pipeline, mut system_rx) = recording_pipeline();
            let (mic_pipeline, mut mic_rx) = recording_pipeline();
            let (system_failure, mut system_failures) = CaptureFailureSender::channel();
            let (mic_failure, mut mic_failures) = CaptureFailureSender::channel();
            let format = AudioCaptureFormat::pcm16_mono(rate).unwrap();
            system
                .start_input(
                    system_pipeline.ingress().unwrap(),
                    system_failure,
                    format,
                    AudioSource::System,
                )
                .await
                .unwrap();
            microphone
                .start_input(
                    mic_pipeline.ingress().unwrap(),
                    mic_failure,
                    format,
                    AudioSource::Microphone,
                )
                .await
                .unwrap();
            let (_output_audio, output_playback) = play_test_frequency_on("mimi-output", 997.0);
            let (_input_audio, input_playback) = play_test_frequency_on("mimi-microphone", 613.0);
            tokio::join!(
                expect_frequency(&mut system_rx, rate, 997.0),
                expect_frequency(&mut mic_rx, rate, 613.0)
            );
            // Releasing one native worker must not terminate the other lane.
            system.stop().await;
            assert!(system_pipeline.finish(Duration::from_secs(1)).await);
            while system_rx.try_recv().is_ok() {}
            while mic_rx.try_recv().is_ok() {}
            expect_frequency(&mut mic_rx, rate, 613.0).await;
            assert!(system_rx.try_recv().is_err());
            microphone.stop().await;
            assert!(mic_pipeline.finish(Duration::from_secs(1)).await);
            assert!(system_failures.try_recv().is_err());
            assert!(mic_failures.try_recv().is_err());
            drop(output_playback);
            drop(input_playback);
            eprintln!("linux native dual rate={rate} isolated=true restart=true");
        }
    }

    /// The private server exposes a non-monitor recording source backed by a
    /// separate synthetic sink. This proves microphone mode neither accepts
    /// a default monitor nor substitutes the audible output mix.
    #[tokio::test]
    #[ignore = "requires an isolated PulseAudio server with mimi-input and paplay"]
    async fn native_microphone_captures_only_explicit_input_and_restarts() {
        isolated_audio_directory();
        fn set_default_source(source: &str) {
            assert!(Command::new("timeout")
                .args(["2s", "pactl"])
                .args(["set-default-source", source])
                .status()
                .unwrap()
                .success());
        }
        struct RestoreDefaultSource;
        impl Drop for RestoreDefaultSource {
            fn drop(&mut self) {
                set_default_source("mimi-microphone.monitor");
            }
        }
        let _restore = RestoreDefaultSource;
        let capture = LinuxSystemAudioCapture::new();
        set_default_source("mimi-microphone.monitor");
        let (pipeline, _rx) = recording_pipeline();
        let (failure, _) = CaptureFailureSender::channel();
        assert_eq!(
            capture
                .start_input(
                    pipeline.ingress().unwrap(),
                    failure,
                    AudioCaptureFormat::pcm16_mono(16_000).unwrap(),
                    AudioSource::Microphone
                )
                .await,
            Err(SystemAudioCaptureError::NoMicrophoneDevice)
        );
        capture.stop().await;
        pipeline.stop();

        set_default_source("mimi-input");
        for rate in [16_000, 24_000] {
            let (pipeline, mut rx) = recording_pipeline();
            let (failure, mut failures) = CaptureFailureSender::channel();
            tokio::time::timeout(
                Duration::from_secs(7),
                capture.start_input(
                    pipeline.ingress().unwrap(),
                    failure,
                    AudioCaptureFormat::pcm16_mono(rate).unwrap(),
                    AudioSource::Microphone,
                ),
            )
            .await
            .unwrap()
            .unwrap();
            expect_native_silence(&pipeline, &mut rx, rate, "microphone-before-playback").await;
            let (_output_audio, output_playback) = play_test_tone();
            // Drain a full half-second while output is audible: microphone
            // mode must not capture that independently playing system tone.
            let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
            let mut silent_frames = 0;
            while let Ok(Some(pcm)) = tokio::time::timeout_at(deadline, rx.recv()).await {
                assert!(pcm
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .all(|sample| i16::from_le_bytes(*sample).unsigned_abs() <= 2));
                silent_frames += 1;
            }
            assert!(
                silent_frames >= 5,
                "microphone must supply silent PCM while system output is audible"
            );
            drop(output_playback);

            let (_input_audio, input_playback) = play_test_tone_on("mimi-microphone");
            tokio::time::timeout(Duration::from_secs(6), async {
                let mut window = Vec::new();
                while let Some(pcm) = rx.recv().await {
                    assert_eq!(pcm.len(), rate as usize * 2 * FRAGMENT_MS / 1000);
                    window.extend(pcm);
                    if window.len() >= rate as usize / 2 {
                        if matches_tone(&window, rate) {
                            return;
                        }
                        window.clear();
                    }
                }
                panic!("microphone capture ended before its input tone arrived");
            })
            .await
            .expect("default input must contain its 997 Hz tone at the provider rate");
            drop(input_playback);
            tokio::time::timeout(Duration::from_secs(1), capture.stop())
                .await
                .expect("stop releases input without waiting for audio");
            while rx.try_recv().is_ok() {}
            tokio::time::sleep(Duration::from_millis(75)).await;
            while rx.try_recv().is_ok() {}
            tokio::time::sleep(Duration::from_millis(75)).await;
            assert!(
                rx.try_recv().is_err(),
                "stopped microphone must emit no new PCM"
            );
            assert!(failures.try_recv().is_err());
            pipeline.stop();
        }
    }

    /// Run under scripts/linux-audio-smoke.sh: it supplies a private server,
    /// a null playback sink, and a deliberately different default source.
    /// No provider credentials, device capture, or display are needed.
    #[tokio::test]
    #[ignore = "requires an isolated PulseAudio server and paplay"]
    async fn native_monitor_capture_is_pcm16_and_restarts() {
        isolated_audio_directory();
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
