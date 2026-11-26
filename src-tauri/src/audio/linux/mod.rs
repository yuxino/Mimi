//! Linux system audio through PulseAudio (including PipeWire's PulseAudio
//! server). Resolve and verify the default output's monitor explicitly; the
//! default recording source is opened only for explicit microphone capture.
//! The server converts the selected source to the requested mono PCM16LE rate.
//!
//! All libpulse objects stay on one worker thread. Startup waits for native
//! events with a short timeout so it can still observe cancellation when the
//! server is silent. SessionManager implements pause and resume by stopping
//! this worker and starting a fresh monitor stream.

use super::capture_completion::CaptureCompletion;
use crate::audio::send_pipeline::{AudioIngress, AudioIngressError};
use crate::audio::{
    AudioCaptureFormat, CaptureFailureSender, SystemAudioCaptureError, SystemAudioCaptureFailure,
};
use crate::core::audio::audio_input::AudioSource;
use crate::core::diagnostics::latency::milliseconds;
use crate::pipeline_log;
use libpulse_binding as pulse;
use pulse::callbacks::ListResult;
use pulse::context::{Context, FlagSet as ContextFlags, State as ContextState};
use pulse::def::BufferAttr;
use pulse::mainloop::standard::{IterateResult, Mainloop};
use pulse::sample::{Format, Spec};
use pulse::stream::{FlagSet as StreamFlags, PeekResult, State as StreamState, Stream};
use pulse::time::MicroSeconds;
use std::cell::{Cell, RefCell};
use std::ops::{Deref, DerefMut};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

const POLL_INTERVAL: Duration = Duration::from_millis(5);
pub(super) const START_TIMEOUT: Duration = Duration::from_secs(5);
const FRAGMENT_MS: usize = 20;
const MAX_BUFFER_MS: usize = 250;

// libpulse 13 accepted milliseconds in pa_mainloop_prepare. Keep the
// nonblocking path on older or unrecognized client libraries.
static SUPPORTS_TIMED_POLL: LazyLock<bool> = LazyLock::new(|| {
    pulse::version::get_library_version_numbers().is_ok_and(|(major, _, _)| major >= 14)
});

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
            worker.cancel();
        }
    }
}

#[derive(Default)]
pub(super) struct WorkerControl {
    cancelled: AtomicBool,
    completion: CaptureCompletion,
    capture_thread: OnceLock<std::thread::Thread>,
}

impl WorkerControl {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Some(thread) = self.capture_thread.get() {
            thread.unpark();
        }
    }
}

/// Dropping an awaited start (for example SessionManager's timeout) must
/// cancel its native worker, including before it reports readiness.
struct CancelStartOnDrop(Option<Arc<WorkerControl>>);

impl Drop for CancelStartOnDrop {
    fn drop(&mut self) {
        if let Some(worker) = &self.0 {
            worker.cancel();
        }
    }
}

struct FinishWorkerOnDrop(Arc<WorkerControl>);

impl Drop for FinishWorkerOnDrop {
    fn drop(&mut self) {
        self.0.completion.finish();
    }
}

impl LinuxSystemAudioCapture {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub async fn start(
        &self,
        audio_ingress: AudioIngress,
        failure_tx: CaptureFailureSender,
        format: AudioCaptureFormat,
    ) -> Result<(), SystemAudioCaptureError> {
        self.start_target(audio_ingress, failure_tx, format, None)
            .await
    }

    pub async fn start_input(
        &self,
        audio_ingress: AudioIngress,
        failure_tx: CaptureFailureSender,
        format: AudioCaptureFormat,
        input: AudioSource,
    ) -> Result<(), SystemAudioCaptureError> {
        self.start_selected(audio_ingress, failure_tx, format, input, None)
            .await
    }

    pub async fn start_target(
        &self,
        audio_ingress: AudioIngress,
        failure_tx: CaptureFailureSender,
        format: AudioCaptureFormat,
        token: Option<String>,
    ) -> Result<(), SystemAudioCaptureError> {
        self.start_selected(
            audio_ingress,
            failure_tx,
            format,
            AudioSource::System,
            token,
        )
        .await
    }

    async fn start_selected(
        &self,
        audio_ingress: AudioIngress,
        failure_tx: CaptureFailureSender,
        format: AudioCaptureFormat,
        input: AudioSource,
        token: Option<String>,
    ) -> Result<(), SystemAudioCaptureError> {
        if let Some(token) = &token {
            super::linux_streams::resolve(token)?;
        }
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
                worker_control
                    .capture_thread
                    .get_or_init(std::thread::current);
                let mut capture = match PulseCapture::connect(&worker_control, format, input, token)
                {
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
            control.completion.finish();
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
            if !worker.completion.is_finished() {
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
                worker.cancel();
                Arc::clone(worker)
            })
        };
        if let Some(worker) = worker {
            // The slot stays reserved if this stop future is itself dropped.
            // A new start cannot overlap a worker still releasing its source.
            worker.completion.wait().await;
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
    target: Option<(String, u32, u32)>,
    target_deleted: Rc<Cell<bool>>,
}

/// Disconnect on every exit path, including a cancelled or timed-out
/// introspection before a stream exists. Context's own Drop only unrefs it.
pub(super) struct ConnectedContext(pub(super) Context);

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
        token: Option<String>,
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

        let target = token
            .map(|token| {
                super::linux_streams::resolve(&token).map(|(index, sink)| (token, index, sink))
            })
            .transpose()?;
        let target_deleted = Rc::new(Cell::new(false));
        if let Some((_, index, _)) = &target {
            use pulse::context::subscribe::{Facility, InterestMaskSet, Operation};
            let index = *index;
            let deleted = Rc::clone(&target_deleted);
            context.set_subscribe_callback(Some(Box::new(move |facility, operation, actual| {
                if facility == Some(Facility::SinkInput)
                    && actual == index
                    && matches!(operation, Some(Operation::Removed | Operation::New))
                {
                    deleted.set(true);
                }
            })));
            let subscribed = Rc::new(Cell::new(None));
            let result = Rc::clone(&subscribed);
            let _subscription =
                context.subscribe(InterestMaskSet::SINK_INPUT, move |ok| result.set(Some(ok)));
            if !wait_for(&mut mainloop, control, deadline, || {
                check_context(&context)?;
                Ok(subscribed.take())
            })? {
                return Err(SystemAudioCaptureError::NativeStartFailed);
            }
        }
        let (source_index, source_name) = match input {
            AudioSource::System => {
                let monitor = if let Some((token, index, sink)) = &target {
                    let monitor = resolve_stream_monitor(
                        &mut mainloop,
                        &context,
                        control,
                        deadline,
                        *index,
                        *sink,
                    )?;
                    if super::linux_streams::resolve(token)? != (*index, *sink) {
                        return Err(SystemAudioCaptureError::ApplicationUnavailable);
                    }
                    monitor
                } else {
                    resolve_monitor(&mut mainloop, &context, control, deadline)?
                };
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
        if let Some((_, index, _)) = &target {
            stream
                .set_monitor_stream(*index)
                .map_err(|_| SystemAudioCaptureError::ApplicationUnavailable)?;
        }
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
        if target_deleted.get() {
            return Err(SystemAudioCaptureError::ApplicationUnavailable);
        }
        if let Some((token, index, sink)) = &target {
            if super::linux_streams::resolve(token)? != (*index, *sink) {
                return Err(SystemAudioCaptureError::NativeStartFailed);
            }
        }
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
            target,
            target_deleted,
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
        let mut last_target_revision = None;
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
            if self.target.is_some() && self.context.get_state() != ContextState::Ready {
                return Err(SystemAudioCaptureFailure::ApplicationUnavailable);
            }
            if self.target_deleted.get() {
                return Err(SystemAudioCaptureFailure::ApplicationUnavailable);
            }
            if !matches!(poll_result, IterateResult::Success(_))
                || self.context.get_state() != ContextState::Ready
                || self.stream.get_state() != StreamState::Ready
                || self.stream.get_device_index() != Some(self.source_index)
            {
                // The record stream may fail before its deletion subscription
                // is dispatched. Query that same context before choosing recovery.
                if self.context.get_state() == ContextState::Ready {
                    if let Some((_, index, _)) = &self.target {
                        if matches!(
                            resolve_stream_sink(
                                &mut self.mainloop,
                                &self.context,
                                control,
                                Instant::now() + START_TIMEOUT,
                                *index
                            ),
                            Err(SystemAudioCaptureError::ApplicationUnavailable)
                        ) {
                            return Err(SystemAudioCaptureFailure::ApplicationUnavailable);
                        }
                    }
                }
                return Err(SystemAudioCaptureFailure::NativeStopped);
            }
            if let Some((token, index, sink)) = &self.target {
                match super::linux_streams::resolve_if_changed(token, &mut last_target_revision) {
                    Err(_) => return Err(SystemAudioCaptureFailure::ApplicationUnavailable),
                    Ok(Some(actual)) if actual != (*index, *sink) => {
                        return Err(SystemAudioCaptureFailure::NativeStopped)
                    }
                    _ => {}
                }
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
            // Cancellation stores a wake token even if it arrives just before
            // this wait. The dedicated capture thread otherwise keeps the same
            // poll timeout.
            std::thread::park_timeout(POLL_INTERVAL);
        }
        Ok(())
    }
}

pub(super) fn wait_for<T>(
    mainloop: &mut Mainloop,
    control: &WorkerControl,
    deadline: Instant,
    mut check: impl FnMut() -> Result<Option<T>, SystemAudioCaptureError>,
) -> Result<T, SystemAudioCaptureError> {
    let timed_poll = *SUPPORTS_TIMED_POLL;
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
        if timed_poll {
            let wait = POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now()));
            mainloop
                .prepare(Some(MicroSeconds(wait.as_micros() as u64)))
                .and_then(|()| mainloop.poll())
                .and_then(|_| mainloop.dispatch())
                .map_err(|_| SystemAudioCaptureError::AudioServerUnavailable)?;
        } else {
            if !matches!(mainloop.iterate(false), IterateResult::Success(_)) {
                return Err(SystemAudioCaptureError::AudioServerUnavailable);
            }
            std::thread::sleep(POLL_INTERVAL);
        }
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

    verify_monitor(mainloop, context, control, deadline, monitor)
}

fn resolve_stream_sink(
    mainloop: &mut Mainloop,
    context: &Context,
    control: &WorkerControl,
    deadline: Instant,
    index: u32,
) -> Result<u32, SystemAudioCaptureError> {
    let sink = Rc::new(RefCell::new(None));
    let result = Rc::clone(&sink);
    let _query = context
        .introspect()
        .get_sink_input_info(index, move |item| match item {
            ListResult::Item(info) => *result.borrow_mut() = Some(Some(info.sink)),
            ListResult::End | ListResult::Error => {
                if result.borrow().is_none() {
                    *result.borrow_mut() = Some(None);
                }
            }
        });
    wait_for(mainloop, control, deadline, || {
        check_context(context)?;
        Ok(sink.borrow_mut().take())
    })?
    .ok_or(SystemAudioCaptureError::ApplicationUnavailable)
}

fn resolve_stream_monitor(
    mainloop: &mut Mainloop,
    context: &Context,
    control: &WorkerControl,
    deadline: Instant,
    index: u32,
    expected_sink: u32,
) -> Result<MonitorSource, SystemAudioCaptureError> {
    let sink = resolve_stream_sink(mainloop, context, control, deadline, index)?;
    if sink != expected_sink {
        return Err(SystemAudioCaptureError::NativeStartFailed);
    }
    let monitor = Rc::new(RefCell::new(None));
    let result = Rc::clone(&monitor);
    let _query = context
        .introspect()
        .get_sink_info_by_index(sink, move |item| match item {
            ListResult::Item(info) => {
                *result.borrow_mut() = Some(
                    info.monitor_source_name
                        .as_deref()
                        .filter(|name| valid_device_name(name))
                        .filter(|_| info.monitor_source != u32::MAX)
                        .map(|name| MonitorSource {
                            sink_index: info.index,
                            source_index: info.monitor_source,
                            source_name: name.to_owned(),
                        }),
                )
            }
            ListResult::End | ListResult::Error => {
                if result.borrow().is_none() {
                    *result.borrow_mut() = Some(None);
                }
            }
        });
    let monitor = wait_for(mainloop, control, deadline, || {
        check_context(context)?;
        Ok(monitor.borrow_mut().take())
    })?
    .ok_or(SystemAudioCaptureError::NoPlaybackDevice)?;
    verify_monitor(mainloop, context, control, deadline, monitor)
}

fn verify_monitor(
    mainloop: &mut Mainloop,
    context: &Context,
    control: &WorkerControl,
    deadline: Instant,
    monitor: MonitorSource,
) -> Result<MonitorSource, SystemAudioCaptureError> {
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

pub(super) fn check_context(context: &Context) -> Result<(), SystemAudioCaptureError> {
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
mod tests;
