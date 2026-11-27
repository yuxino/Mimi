//! macOS system-audio capture (audio only, own process audio excluded,
//! provider-rate PCM16 mono). Without an existing screen grant, the system mix
//! uses Core Audio taps on macOS 14.2+; existing screen grants, older systems
//! and application capture use ScreenCaptureKit.
//!
//! ScreenCaptureKit types are main-thread-only (!Send). All stream, handler,
//! and content objects are therefore created, used, and destroyed inside
//! closures dispatched to the main thread; completions that fire on arbitrary
//! queues re-dispatch there before touching those objects.

mod applications;
mod decode;

use crate::audio::applications::ApplicationSnapshot;
use crate::audio::send_pipeline::{AudioIngress, AudioIngressError};
use crate::audio::{
    AudioCaptureFormat, CaptureFailureSender, SystemAudioCaptureError, SystemAudioCaptureFailure,
};
use crate::core::audio::system_audio_target::SystemAudioTarget;
use crate::pipeline_log;
use applications::{list_audio_applications, own_bundle_identifier};
use core_media::sample_buffer::CMSampleBufferRef;
use core_media::time::CMTime;
use decode::capture_to_pcm16;
pub(super) use decode::decode_to_f32_mono;
use dispatch2::{DispatchQueue, DispatchQueueAttr};
use objc2::rc::Retained;
use objc2::runtime::{NSObject, ProtocolObject};
use objc2::{define_class, msg_send, AnyThread, DefinedClass};
use objc2_foundation::{NSArray, NSObjectProtocol};
use screen_capture_kit::error::{SCStreamErrorCode, SCStreamErrorDomain};
use screen_capture_kit::shareable_content::{SCDisplay, SCShareableContent};
use screen_capture_kit::stream::{
    SCContentFilter, SCStream, SCStreamConfiguration, SCStreamDelegate, SCStreamOutput,
    SCStreamOutputType,
};
use std::cell::{Cell, RefCell};
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{oneshot, Notify};

/// Executes a closure on the main thread (wraps `AppHandle::run_on_main_thread`).
pub type MainThreadDispatcher = Arc<dyn Fn(Box<dyn FnOnce() + Send>) + Send + Sync>;

struct AudioHandlerState {
    audio_ingress: AudioIngress,
    failure_tx: CaptureFailureSender,
    generation: CaptureGeneration,
    generation_token: u64,
    last_audio_buffer_at: Mutex<Option<Instant>>,
    /// Decoded-buffer counter used to throttle diagnostics.
    decoded_buffers: Mutex<u64>,
    /// Provider-rate resampler paired with the input sample rate it was built
    /// for; rebuilt whenever the session starts or the stream sample rate changes.
    /// SCStream delivers the device rate (typically 48 kHz) regardless of the
    /// requested configuration, so the samples must be resampled before the
    /// selected provider's input rate (16 or 24 kHz PCM).
    resampler: Mutex<Option<(u32, rubato::Fft<f32>)>>,
    pending_frames: Mutex<Vec<f32>>,
    target_sample_rate_hz: u32,
}

/// A pointer that is only ever dereferenced on the main thread, inside the
/// closure it is delivered to. `unsafe impl Send` is sound because raw pointer
/// values carry no ownership and the pointee never crosses threads.
struct MainThreadPtr(*mut ());
unsafe impl Send for MainThreadPtr {}

impl MainThreadPtr {
    /// Consumes the wrapper and returns the raw pointer. Called inside the
    /// main-thread closure so the whole wrapper (with its Send impl) is
    /// captured rather than the bare field.
    fn into_inner(self) -> *mut () {
        self.0
    }
}

pub struct MimiAudioStreamHandlerIvars {
    state: AudioHandlerState,
}

define_class!(
    // SAFETY:
    // - The superclass NSObject has no subclassing requirements.
    // - MimiAudioStreamHandler does not implement Drop; objc2 synthesizes
    //   `dealloc` to drop the owned Send + Sync ivars when the last Objective-C
    //   reference is released.
    #[unsafe(super(NSObject))]
    #[name = "MimiAudioStreamHandler"]
    #[ivars = MimiAudioStreamHandlerIvars]
    pub struct MimiAudioStreamHandler;

    unsafe impl NSObjectProtocol for MimiAudioStreamHandler {}

    unsafe impl SCStreamOutput for MimiAudioStreamHandler {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        unsafe fn stream_did_output_sample_buffer(
            &self,
            _stream: &SCStream,
            sample_buffer: CMSampleBufferRef,
            of_type: SCStreamOutputType,
        ) {
            if of_type != SCStreamOutputType::Audio {
                return;
            }
            let state = &self.ivars().state;
            if !state.generation.is_current(state.generation_token)
                || state.failure_tx.has_reported()
            {
                return;
            }
            let now = Instant::now();
            if let Some(last) = *state.last_audio_buffer_at.lock().unwrap() {
                let gap_ms = now.saturating_duration_since(last).as_millis();
                if gap_ms > 500 {
                    pipeline_log!("capture gapMs={}", gap_ms);
                }
            }
            *state.last_audio_buffer_at.lock().unwrap() = Some(now);

            match capture_to_pcm16(state, sample_buffer) {
                Ok(Some(data)) => {
                    // Content-free capture diagnostics: report decoded-buffer
                    // statistics every 200 buffers.
                    let count = {
                        let mut count = state.decoded_buffers.lock().unwrap();
                        *count += 1;
                        *count
                    };
                    if count % 200 == 1 {
                        let nonzero = data.iter().filter(|byte| **byte != 0).count();
                        pipeline_log!(
                            "capture decode buffers={} bytes={} nonzero={}/{}",
                            count,
                            data.len(),
                            nonzero,
                            data.len()
                        );
                    }
                    match state.audio_ingress.try_send(data) {
                        Ok(()) | Err(AudioIngressError::Closed) => {}
                        Err(AudioIngressError::Backpressure) => {
                            state
                                .failure_tx
                                .report(SystemAudioCaptureFailure::Backpressure);
                        }
                    }
                }
                Ok(None) => {}
                Err(_) => {
                    state
                        .failure_tx
                        .report(SystemAudioCaptureFailure::AudioProcessingFailed);
                }
            }
        }
    }

    unsafe impl SCStreamDelegate for MimiAudioStreamHandler {
        #[unsafe(method(stream:didStopWithError:))]
        unsafe fn stream_did_stop_with_error(
            &self,
            _stream: &SCStream,
            error: &objc2_foundation::NSError,
        ) {
            let state = &self.ivars().state;
            if state.generation.is_current(state.generation_token) {
                state.failure_tx.report(classify_native_stop_error(error));
            }
        }
    }
);

// Main-thread-only live capture state. Only ever accessed from main-thread
// closures (see `MainThreadDispatcher`).
thread_local! {
    static MAIN_STREAM: RefCell<Option<Retained<SCStream>>> = const { RefCell::new(None) };
    static MAIN_HANDLER: RefCell<Option<Retained<MimiAudioStreamHandler>>> =
        const { RefCell::new(None) };
    static MAIN_GENERATION: Cell<Option<u64>> = const { Cell::new(None) };
    static MAIN_TARGET_APPS: RefCell<Vec<Retained<objc2_app_kit::NSRunningApplication>>> = const { RefCell::new(Vec::new()) };
}

const CAPTURE_START_TIMEOUT: Duration = Duration::from_secs(10);
const CAPTURE_STOP_TIMEOUT: Duration = Duration::from_secs(2);
type CaptureStartResult = Result<(), SystemAudioCaptureError>;
type CaptureStartDelivery = (oneshot::Sender<CaptureStartResult>, CaptureStartResult);

/// Joins the two asynchronous halves of native startup: ScreenCaptureKit's
/// completion callback and installation of the main-thread-owned stream state.
/// Either may happen first; readiness is acknowledged only after both succeed.
#[derive(Clone)]
struct CaptureStartBarrier {
    state: Arc<Mutex<CaptureStartBarrierState>>,
}

struct CaptureStartBarrierState {
    installed: bool,
    completion: Option<CaptureStartResult>,
    sender: Option<oneshot::Sender<CaptureStartResult>>,
}

impl CaptureStartBarrier {
    fn new(sender: oneshot::Sender<CaptureStartResult>) -> Self {
        Self {
            state: Arc::new(Mutex::new(CaptureStartBarrierState {
                installed: false,
                completion: None,
                sender: Some(sender),
            })),
        }
    }

    fn did_install(&self) {
        let delivery = {
            let mut state = self.state.lock().unwrap();
            state.installed = true;
            Self::take_delivery(&mut state)
        };
        Self::deliver(delivery);
    }

    fn did_complete(&self, result: CaptureStartResult) {
        let delivery = {
            let mut state = self.state.lock().unwrap();
            if state.sender.is_none() {
                return;
            }
            state.completion = Some(result);
            Self::take_delivery(&mut state)
        };
        Self::deliver(delivery);
    }

    fn fail(&self, error: SystemAudioCaptureError) {
        let sender = self.state.lock().unwrap().sender.take();
        if let Some(sender) = sender {
            let _ = sender.send(Err(error));
        }
    }

    fn take_delivery(state: &mut CaptureStartBarrierState) -> Option<CaptureStartDelivery> {
        if !state.installed {
            return None;
        }
        match (state.sender.take(), state.completion.take()) {
            (Some(sender), Some(result)) => Some((sender, result)),
            (sender, completion) => {
                state.sender = sender;
                state.completion = completion;
                None
            }
        }
    }

    fn deliver(delivery: Option<CaptureStartDelivery>) {
        if let Some((sender, result)) = delivery {
            let _ = sender.send(result);
        }
    }
}

/// Invalidates native callbacks and asynchronous completions from older starts.
/// A stop changes the value synchronously, before its main-thread teardown is
/// dispatched, so a pending content callback cannot install a ghost stream.
#[derive(Clone, Default)]
pub(super) struct CaptureGeneration {
    value: Arc<AtomicU64>,
}

impl CaptureGeneration {
    fn begin(&self) -> u64 {
        self.value.fetch_add(1, Ordering::SeqCst).wrapping_add(1)
    }

    fn invalidate(&self) -> u64 {
        self.value.fetch_add(1, Ordering::SeqCst)
    }

    fn invalidate_if_current(&self, token: u64) -> bool {
        self.value
            .compare_exchange(
                token,
                token.wrapping_add(1),
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_ok()
    }

    pub(super) fn is_current(&self, token: u64) -> bool {
        self.value.load(Ordering::SeqCst) == token
    }
}

#[derive(Clone, Default)]
pub(super) struct PendingTeardown {
    count: Arc<AtomicUsize>,
    notify: Arc<Notify>,
}

impl PendingTeardown {
    pub(super) fn begin(&self) -> PendingTeardownGuard {
        self.count.fetch_add(1, Ordering::SeqCst);
        PendingTeardownGuard {
            count: Arc::clone(&self.count),
            notify: Arc::clone(&self.notify),
        }
    }

    pub(super) fn is_pending(&self) -> bool {
        self.count.load(Ordering::SeqCst) > 0
    }

    async fn wait_until_clear(&self) {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if !self.is_pending() {
                return;
            }
            notified.await;
        }
    }
}

pub(super) struct PendingTeardownGuard {
    count: Arc<AtomicUsize>,
    notify: Arc<Notify>,
}

impl Drop for PendingTeardownGuard {
    fn drop(&mut self) {
        if self.count.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.notify.notify_waiters();
        }
    }
}

async fn finish_failed_capture_start<F>(
    generation: &CaptureGeneration,
    started: &AtomicBool,
    transition: &Mutex<()>,
    generation_token: u64,
    teardown: F,
) where
    F: std::future::Future<Output = ()>,
{
    {
        let _transition = transition.lock().unwrap();
        if generation.invalidate_if_current(generation_token) {
            started.store(false, Ordering::SeqCst);
        }
    }
    // Phase 2 can install and start the stream immediately before the setup
    // acknowledgement loses a timeout race. The failing start must therefore
    // await token-scoped teardown instead of relying on a later `stop()` call.
    teardown.await;
}

#[derive(Clone)]
pub struct MacSystemAudioCapture {
    dispatcher: MainThreadDispatcher,
    started: Arc<AtomicBool>,
    transition: Arc<Mutex<()>>,
    generation: CaptureGeneration,
    pending_teardown: PendingTeardown,
    #[cfg(feature = "development-debugger")]
    smoke_backend: Option<bool>,
}

impl MacSystemAudioCapture {
    pub fn new(dispatcher: MainThreadDispatcher) -> Self {
        Self {
            dispatcher,
            started: Arc::new(AtomicBool::new(false)),
            transition: Arc::new(Mutex::new(())),
            generation: CaptureGeneration::default(),
            pending_teardown: PendingTeardown::default(),
            #[cfg(feature = "development-debugger")]
            smoke_backend: None,
        }
    }

    /// Only the explicit, local native-smoke entry point may force a backend.
    #[cfg(feature = "development-debugger")]
    pub(super) fn for_smoke(dispatcher: MainThreadDispatcher, tap: Option<bool>) -> Self {
        Self {
            smoke_backend: tap,
            ..Self::new(dispatcher)
        }
    }

    /// AppKit lists application metadata without requesting screen contents or
    /// recording authorization. Only starting capture may access ScreenCaptureKit.
    pub async fn audio_applications(&self) -> Result<ApplicationSnapshot, SystemAudioCaptureError> {
        let (tx, rx) = oneshot::channel();
        (self.dispatcher)(Box::new(move || {
            let _ = tx.send(list_audio_applications());
        }));
        tokio::time::timeout(CAPTURE_START_TIMEOUT, rx)
            .await
            .map_err(|_| SystemAudioCaptureError::StartTimedOut)?
            .map_err(|_| SystemAudioCaptureError::ApplicationListFailed)?
    }

    pub async fn start(
        &self,
        audio_ingress: AudioIngress,
        failure_tx: CaptureFailureSender,
        format: AudioCaptureFormat,
        target: SystemAudioTarget,
    ) -> Result<(), SystemAudioCaptureError> {
        // Claiming active state and assigning its token must be one transition.
        // A concurrent stop between them could otherwise invalidate the old
        // token, after which this stopped start would create a fresh generation.
        let generation_token = {
            let _transition = self.transition.lock().unwrap();
            if self.started.swap(true, Ordering::SeqCst) {
                return Err(SystemAudioCaptureError::AlreadyRunning);
            }
            self.generation.begin()
        };
        if tokio::time::timeout(
            CAPTURE_START_TIMEOUT,
            self.pending_teardown.wait_until_clear(),
        )
        .await
        .is_err()
        {
            self.finish_failed_start(generation_token).await;
            return Err(SystemAudioCaptureError::PreviousCaptureStopping);
        }
        if !self.generation.is_current(generation_token) {
            self.finish_failed_start(generation_token).await;
            return Err(SystemAudioCaptureError::StartCancelled);
        }

        let use_tap = super::macos_tap::use_audio_tap(
            super::macos_tap::is_available(),
            core_graphics2::window::preflight_screen_capture_access(),
        );
        #[cfg(feature = "development-debugger")]
        let use_tap = self.smoke_backend.unwrap_or(use_tap);
        pipeline_log!(
            "capture selected backend={}",
            if use_tap {
                "core_audio_tap"
            } else {
                "screen_capture_kit"
            }
        );
        if use_tap {
            // Reserve teardown before spawning. A timed-out native permission
            // request must keep subsequent sources closed until it really exits.
            let result = super::macos_tap::start(
                audio_ingress,
                failure_tx,
                format,
                target,
                self.generation.clone(),
                generation_token,
                self.pending_teardown.begin(),
            )
            .await;
            if result.is_err() {
                self.finish_failed_start(generation_token).await;
            }
            return result;
        }

        let application_target = target.application_id().is_some();
        let monitor_failure = failure_tx.clone();
        // Phase 1 (main thread): ask ScreenCaptureKit for shareable content.
        let dispatcher = Arc::clone(&self.dispatcher);
        let phase2_dispatcher = Arc::clone(&self.dispatcher);
        let generation = self.generation.clone();
        let pending_teardown = self.pending_teardown.clone();
        let (ready_tx, ready_rx) = oneshot::channel();
        let start_barrier = CaptureStartBarrier::new(ready_tx);
        let phase1_barrier = start_barrier.clone();
        dispatcher(Box::new(move || {
            if !generation.is_current(generation_token) {
                phase1_barrier.fail(SystemAudioCaptureError::StartCancelled);
                return;
            }
            // ScreenCaptureKit calls can raise Objective-C exceptions; catch
            // them here so they surface as errors instead of aborting.
            let barrier_for_callback = phase1_barrier.clone();
            let generation_for_callback = generation.clone();
            let pending_teardown_for_callback = pending_teardown.clone();
            let request = objc2::exception::catch(AssertUnwindSafe(|| {
                SCShareableContent::get_shareable_content_excluding_desktop_windows(
                    false,
                    false,
                    move |content, error| {
                        if !generation_for_callback.is_current(generation_token) {
                            barrier_for_callback.fail(SystemAudioCaptureError::StartCancelled);
                            return;
                        }
                        // This completion fires on an arbitrary queue; re-dispatch
                        // phase 2 to the main thread with the content.
                        let content = match (content, error) {
                            (Some(content), _) => content,
                            (None, Some(error)) => {
                                barrier_for_callback.fail(classify_native_start_error(&error));
                                return;
                            }
                            (None, None) => {
                                barrier_for_callback.fail(SystemAudioCaptureError::NoDisplay);
                                return;
                            }
                        };
                        let ptr = MainThreadPtr(Box::into_raw(Box::new(content)) as *mut ());
                        let audio_ingress = audio_ingress.clone();
                        let target = target.clone();
                        let failure_tx = failure_tx.clone();
                        let dispatcher = Arc::clone(&phase2_dispatcher);
                        let barrier = barrier_for_callback.clone();
                        let generation = generation_for_callback.clone();
                        let pending_teardown = pending_teardown_for_callback.clone();
                        dispatcher(Box::new(move || {
                            let raw = ptr.into_inner();
                            let content =
                                unsafe { *Box::from_raw(raw as *mut Retained<SCShareableContent>) };
                            if !generation.is_current(generation_token) {
                                barrier.fail(SystemAudioCaptureError::StartCancelled);
                                return;
                            }
                            let result = objc2::exception::catch(AssertUnwindSafe(|| {
                                start_capture_on_main(
                                    content,
                                    CaptureStartRequest {
                                        audio_ingress,
                                        failure_tx,
                                        format,
                                        generation,
                                        generation_token,
                                        pending_teardown,
                                        start_barrier: barrier.clone(),
                                        target,
                                    },
                                )
                            }));
                            match result {
                                Ok(Ok(())) => {
                                    // The native completion owns the success
                                    // acknowledgement through `barrier`.
                                }
                                Ok(Err(error)) => barrier.fail(error),
                                Err(_) => barrier.fail(SystemAudioCaptureError::NativeStartFailed),
                            }
                        }));
                    },
                );
            }));
            if request.is_err() {
                phase1_barrier.fail(SystemAudioCaptureError::NativeStartFailed);
            }
        }));

        let result = match tokio::time::timeout(CAPTURE_START_TIMEOUT, ready_rx).await {
            Ok(Ok(Ok(()))) if self.generation.is_current(generation_token) => Ok(()),
            Ok(Ok(Ok(()))) => Err(SystemAudioCaptureError::StartCancelled),
            Ok(Ok(Err(error))) => Err(error),
            Ok(Err(_)) => Err(SystemAudioCaptureError::NativeStartFailed),
            Err(_) => Err(SystemAudioCaptureError::StartTimedOut),
        };
        if result.is_err() {
            self.finish_failed_start(generation_token).await;
        }
        if result.is_ok() && application_target {
            let generation = self.generation.clone();
            let dispatcher = Arc::clone(&self.dispatcher);
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    if !generation.is_current(generation_token) || monitor_failure.has_reported() {
                        break;
                    }
                    let generation = generation.clone();
                    let failure = monitor_failure.clone();
                    let (checked_tx, checked_rx) = oneshot::channel();
                    dispatcher(Box::new(move || {
                        if MAIN_GENERATION.get() == Some(generation_token)
                            && generation.is_current(generation_token)
                            && MAIN_TARGET_APPS
                                .with(|apps| apps.borrow().iter().all(|app| app.isTerminated()))
                        {
                            failure.report(SystemAudioCaptureFailure::ApplicationUnavailable);
                        }
                        let _ = checked_tx.send(());
                    }));
                    if tokio::time::timeout(Duration::from_secs(2), checked_rx)
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            });
        }
        result
    }

    pub async fn stop(&self) {
        let generation_token = {
            let _transition = self.transition.lock().unwrap();
            if !self.started.swap(false, Ordering::SeqCst) {
                return;
            }
            self.generation.invalidate()
        };
        pipeline_log!("capture stop requested");
        self.teardown_generation(generation_token).await;
    }

    /// Source changes must wait for actual native teardown, not just the
    /// bounded stop acknowledgement. Timeouts keep the new source closed.
    pub async fn wait_until_idle(&self) -> Result<(), SystemAudioCaptureError> {
        tokio::time::timeout(
            CAPTURE_STOP_TIMEOUT,
            self.pending_teardown.wait_until_clear(),
        )
        .await
        .map_err(|_| SystemAudioCaptureError::PreviousCaptureStopping)
    }

    async fn teardown_generation(&self, generation_token: u64) {
        let (done_tx, done_rx) = oneshot::channel::<()>();
        let dispatcher = Arc::clone(&self.dispatcher);
        // Reserve before dispatch: a busy main thread must not leave an idle
        // gap where another source opens while this SCStream is still live.
        let teardown_guard = self.pending_teardown.begin();
        dispatcher(Box::new(move || {
            if MAIN_GENERATION.get() != Some(generation_token) {
                let _ = done_tx.send(());
                return;
            }
            MAIN_GENERATION.set(None);
            MAIN_TARGET_APPS.with(|apps| apps.borrow_mut().clear());
            let stream = MAIN_STREAM.take();
            let handler = MAIN_HANDLER.take();
            let Some(stream) = stream else {
                drop(handler);
                let _ = done_tx.send(());
                return;
            };
            // ScreenCaptureKit requires the stream and its output to stay
            // retained until the stop completion fires. `handler` owns its
            // `AudioHandlerState` directly, so any in-flight Objective-C
            // callback retains valid state until the final reference drops.
            // (The completion is typed `Fn`, so the values are released
            // through `Cell::take`, which is callable on a shared reference.)
            pipeline_log!("capture stop: stop_capture called");
            let completion_stream = std::cell::Cell::new(Some(stream.clone()));
            let completion_handler = std::cell::Cell::new(handler);
            let completion_teardown = std::cell::Cell::new(Some(teardown_guard));
            let completion_done = std::cell::Cell::new(Some(done_tx));
            stream.stop_capture(move |error| {
                if error.is_some() {
                    pipeline_log!("capture stop completed label=capture.native_stop_failed");
                } else {
                    pipeline_log!("capture stop completed");
                }
                drop(completion_stream.take());
                drop(completion_handler.take());
                drop(completion_teardown.take());
                if let Some(done_tx) = completion_done.take() {
                    let _ = done_tx.send(());
                }
            });
        }));
        match tokio::time::timeout(CAPTURE_STOP_TIMEOUT, done_rx).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {
                pipeline_log!("capture stop cancelled label=capture.native_stop_cancelled");
            }
            Err(_) => {
                pipeline_log!("capture stop timed out label=capture.native_stop_timed_out");
            }
        }
        // The tap worker owns native resources until IOProc removal completes.
        // Generation invalidation above tells it to stop, including mid-setup.
        let _ = tokio::time::timeout(
            CAPTURE_STOP_TIMEOUT,
            self.pending_teardown.wait_until_clear(),
        )
        .await;
    }

    async fn finish_failed_start(&self, generation_token: u64) {
        finish_failed_capture_start(
            &self.generation,
            &self.started,
            &self.transition,
            generation_token,
            self.teardown_generation(generation_token),
        )
        .await;
    }
}

/// Runs entirely on the main thread: builds the filter/configuration/handler,
/// installs the stream, and requests capture. Readiness is delivered later by
/// `CaptureStartBarrier`; this function never blocks the main thread.
struct CaptureStartRequest {
    audio_ingress: AudioIngress,
    failure_tx: CaptureFailureSender,
    format: AudioCaptureFormat,
    generation: CaptureGeneration,
    generation_token: u64,
    pending_teardown: PendingTeardown,
    start_barrier: CaptureStartBarrier,
    target: SystemAudioTarget,
}

fn start_capture_on_main(
    content: Retained<SCShareableContent>,
    request: CaptureStartRequest,
) -> Result<(), SystemAudioCaptureError> {
    let CaptureStartRequest {
        audio_ingress,
        failure_tx,
        format,
        generation,
        generation_token,
        pending_teardown,
        start_barrier,
        target,
    } = request;
    if !generation.is_current(generation_token) {
        return Err(SystemAudioCaptureError::StartCancelled);
    }
    if MAIN_STREAM.with(|stream| stream.borrow().is_some()) {
        return Err(SystemAudioCaptureError::AlreadyRunning);
    }
    if pending_teardown.is_pending() {
        return Err(SystemAudioCaptureError::PreviousCaptureStopping);
    }
    pipeline_log!("system audio capture phase2 start");
    // Pick the main display, falling back to the first one.
    let displays = content.displays();
    let main_display_id = unsafe { core_graphics2::display::CGMainDisplayID() };
    let mut chosen_display: Option<Retained<SCDisplay>> = None;
    for index in 0..displays.len() {
        let display = displays.objectAtIndex(index);
        if display.display_id() == main_display_id {
            chosen_display = Some(display);
            break;
        }
    }
    let Some(display) = chosen_display.or_else(|| displays.firstObject()) else {
        return Err(SystemAudioCaptureError::NoDisplay);
    };

    let own_bundle_id = own_bundle_identifier();
    let applications = content.applications();
    let mut selected = Vec::new();
    for app in applications.iter() {
        let bundle = app.bundle_identifier().to_string();
        let own = own_bundle_id.as_ref().is_some_and(|own| bundle == *own);
        if match target.application_id() {
            Some(id) => !own && bundle == id,
            None => own,
        } {
            selected.push(app);
        }
    }
    if target.application_id().is_some() && selected.is_empty() {
        return Err(SystemAudioCaptureError::ApplicationUnavailable);
    }
    let target_apps: Vec<_> = if target.application_id().is_some() {
        selected
            .iter()
            .filter_map(|app| {
                objc2_app_kit::NSRunningApplication::runningApplicationWithProcessIdentifier(
                    app.process_id(),
                )
            })
            .filter(|app| {
                !app.isTerminated()
                    && app.bundleIdentifier().is_some_and(|bundle| {
                        Some(bundle.to_string().as_str()) == target.application_id()
                    })
            })
            .collect()
    } else {
        Vec::new()
    };
    if target.application_id().is_some() && target_apps.is_empty() {
        return Err(SystemAudioCaptureError::ApplicationUnavailable);
    }
    let selected = NSArray::from_retained_slice(&selected);
    let no_windows = NSArray::new();
    let filter = if target.application_id().is_some() {
        SCContentFilter::init_with_display_include_applications(
            SCContentFilter::alloc(),
            &display,
            &selected,
            &no_windows,
        )
    } else {
        SCContentFilter::init_with_display_exclude_applications(
            SCContentFilter::alloc(),
            &display,
            &selected,
            &no_windows,
        )
    };
    pipeline_log!("system audio capture filter built");

    // Audio-only stream configuration at the provider rate, own audio excluded.
    let configuration = SCStreamConfiguration::new();
    configuration.set_captures_audio(true);
    configuration.set_excludes_current_process_audio(true);
    set_capture_sample_rate(&configuration, format.sample_rate_hz);
    configuration.set_channel_count(1);
    configuration.set_width(2);
    configuration.set_height(2);
    configuration.set_minimum_frame_interval(CMTime {
        value: 1,
        timescale: 1,
        flags: 0,
        epoch: 0,
    });
    configuration.set_queue_depth(3);
    // The `screen-capture-kit` crate's `set_show_cursor` sends the wrong
    // selector (`setShowCursor:`); the property is `showsCursor` →
    // `setShowsCursor:`.
    let _: () = unsafe { msg_send![&*configuration, setShowsCursor: false] };

    let state = AudioHandlerState {
        audio_ingress,
        failure_tx,
        generation: generation.clone(),
        generation_token,
        last_audio_buffer_at: Mutex::new(None),
        decoded_buffers: Mutex::new(0),
        resampler: Mutex::new(None),
        pending_frames: Mutex::new(Vec::new()),
        target_sample_rate_hz: format.sample_rate_hz,
    };
    let this = MimiAudioStreamHandler::alloc().set_ivars(MimiAudioStreamHandlerIvars { state });
    let handler: Retained<MimiAudioStreamHandler> = unsafe { msg_send![super(this), init] };
    let output = ProtocolObject::from_ref(&*handler);
    let delegate = ProtocolObject::from_ref(&*handler);

    let stream = SCStream::init_with_filter(SCStream::alloc(), &filter, &configuration, delegate);
    pipeline_log!("system audio capture stream created");

    // Deliver audio samples on a dedicated serial queue.
    let queue = DispatchQueue::new("app.yuxino.mimi.system-audio", DispatchQueueAttr::SERIAL);
    if stream
        .add_stream_output(output, SCStreamOutputType::Audio, &queue)
        .is_err()
    {
        return Err(SystemAudioCaptureError::NativeStartFailed);
    }
    pipeline_log!("system audio capture output attached");

    // Start capture. The completion can take hundreds of milliseconds on the
    // first stream (ScreenCaptureKit enumerates windows and apps while
    // building the session), so it must NOT be waited on synchronously — this
    // function runs on the main thread and blocking it here freezes every
    // window's rendering right at session start. The Tokio caller awaits the
    // asynchronous completion through `start_barrier` instead.
    let generation_for_completion = generation.clone();
    let completion_barrier = start_barrier.clone();
    stream.start_capture(move |error| {
        let result = if !generation_for_completion.is_current(generation_token) {
            Err(SystemAudioCaptureError::StartCancelled)
        } else if let Some(error) = error {
            Err(classify_native_start_error(&error))
        } else {
            Ok(())
        };
        completion_barrier.did_complete(result);
    });
    pipeline_log!("system audio capture start requested (async)");
    if !generation.is_current(generation_token) {
        stop_uninstalled_capture(stream, Some(handler), pending_teardown.clone());
        return Err(SystemAudioCaptureError::StartCancelled);
    }
    MAIN_GENERATION.set(Some(generation_token));
    MAIN_TARGET_APPS.with(|apps| *apps.borrow_mut() = target_apps);
    MAIN_STREAM.set(Some(stream));
    MAIN_HANDLER.set(Some(handler));
    if !generation.is_current(generation_token) {
        MAIN_GENERATION.set(None);
        MAIN_TARGET_APPS.with(|apps| apps.borrow_mut().clear());
        let stream = MAIN_STREAM
            .take()
            .expect("capture stream was just installed");
        let handler = MAIN_HANDLER.take();
        stop_uninstalled_capture(stream, handler, pending_teardown);
        return Err(SystemAudioCaptureError::StartCancelled);
    }
    start_barrier.did_install();
    Ok(())
}

/// The crate binds this property as f64, but ScreenCaptureKit expects NSInteger.
fn set_capture_sample_rate(configuration: &SCStreamConfiguration, sample_rate_hz: u32) {
    // SAFETY: The native setter takes NSInteger, represented by isize on macOS.
    let _: () = unsafe { msg_send![configuration, setSampleRate: sample_rate_hz as isize] };
}

/// Stops a stream that lost its generation race before it could become the
/// installed capture. Resources stay alive until ScreenCaptureKit confirms
/// the stop, and `handler` owns `AudioHandlerState` directly until its last
/// Objective-C reference is released.
fn stop_uninstalled_capture(
    stream: Retained<SCStream>,
    handler: Option<Retained<MimiAudioStreamHandler>>,
    pending_teardown: PendingTeardown,
) {
    let teardown_guard = pending_teardown.begin();
    let completion_stream = Cell::new(Some(stream.clone()));
    let completion_handler = Cell::new(handler);
    let completion_teardown = Cell::new(Some(teardown_guard));
    stream.stop_capture(move |error| {
        if error.is_some() {
            pipeline_log!("capture cleanup stop failed label=capture.native_stop_failed");
        }
        drop(completion_stream.take());
        drop(completion_handler.take());
        drop(completion_teardown.take());
    });
}

fn native_stream_error_code(error: &objc2_foundation::NSError) -> Option<SCStreamErrorCode> {
    // Numeric NSError codes are scoped to their domain. An unrelated -3801
    // must not be presented as a recording permission failure.
    (&*error.domain() == unsafe { SCStreamErrorDomain }).then(|| SCStreamErrorCode(error.code()))
}

fn classify_native_start_error(error: &objc2_foundation::NSError) -> SystemAudioCaptureError {
    match native_stream_error_code(error) {
        Some(SCStreamErrorCode::UserDeclined) => SystemAudioCaptureError::PermissionDenied,
        Some(SCStreamErrorCode::UserStopped) => SystemAudioCaptureError::UserStopped,
        _ => SystemAudioCaptureError::NativeStartFailed,
    }
}

fn classify_native_stop_error(error: &objc2_foundation::NSError) -> SystemAudioCaptureFailure {
    match native_stream_error_code(error) {
        Some(SCStreamErrorCode::UserDeclined) => SystemAudioCaptureFailure::PermissionDenied,
        Some(SCStreamErrorCode::UserStopped) => SystemAudioCaptureFailure::UserStopped,
        _ => SystemAudioCaptureFailure::NativeStopped,
    }
}

#[cfg(test)]
mod tests;
