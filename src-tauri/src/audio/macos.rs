//! macOS system-audio capture (audio only, own process audio excluded,
//! provider-rate PCM16 mono). The system mix uses Core Audio taps on macOS
//! 14.2+; older systems and application capture use ScreenCaptureKit.
//!
//! ScreenCaptureKit types are main-thread-only (!Send). All stream, handler,
//! and content objects are therefore created, used, and destroyed inside
//! closures dispatched to the main thread; completions that fire on arbitrary
//! queues re-dispatch there before touching those objects.

use crate::audio::applications::{
    sort_applications, ApplicationIconBudget, ApplicationSnapshot, AudioApplication,
    MAX_APPLICATION_ICON_PNG_BYTES,
};
use crate::audio::macos_block_buffer;
use crate::audio::send_pipeline::{AudioIngress, AudioIngressError};
use crate::audio::{
    AudioCaptureFormat, CaptureFailureSender, SystemAudioCaptureError, SystemAudioCaptureFailure,
};
use crate::core::pcm16::PCM16Encoder;
use crate::core::system_audio_target::SystemAudioTarget;
use crate::pipeline_log;
use core_media::format_description::CMAudioFormatDescriptionGetStreamBasicDescription;
use core_media::sample_buffer::{
    CMSampleBufferGetDataBuffer, CMSampleBufferGetFormatDescription, CMSampleBufferRef,
};
use core_media::time::CMTime;
use dispatch2::{DispatchQueue, DispatchQueueAttr};
use objc2::rc::Retained;
use objc2::runtime::{NSObject, ProtocolObject};
use objc2::{define_class, msg_send, AnyThread, DefinedClass};
use objc2_core_audio_types::{
    kAudioFormatFlagIsFloat, kAudioFormatFlagIsNonInterleaved, kAudioFormatFlagIsSignedInteger,
    AudioStreamBasicDescription,
};
use objc2_foundation::{NSArray, NSObjectProtocol};
use rubato::audioadapter_buffers::direct::SequentialSlice;
use rubato::Resampler;
use screen_capture_kit::error::{SCStreamErrorCode, SCStreamErrorDomain};
use screen_capture_kit::shareable_content::{SCDisplay, SCShareableContent};
use screen_capture_kit::stream::{
    SCContentFilter, SCStream, SCStreamConfiguration, SCStreamDelegate, SCStreamOutput,
    SCStreamOutputType,
};
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
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
    /// Provider-rate resampler; rebuilt for every capture session.
    /// SCStream delivers the device rate (typically 48 kHz) regardless of the
    /// requested configuration, so the samples must be resampled before the
    /// selected provider's input rate (16 or 24 kHz PCM).
    resampler: Mutex<Option<rubato::Fft<f32>>>,
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
    state_ptr: *mut c_void,
}

define_class!(
    // SAFETY:
    // - The superclass NSObject has no subclassing requirements.
    // - MimiAudioStreamHandler does not implement Drop.
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
            let state = unsafe { &*(self.ivars().state_ptr as *const AudioHandlerState) };
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
                Ok(Some(data)) if !data.is_empty() => {
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
                Ok(None) | Ok(Some(_)) => {}
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
            let state = unsafe { &*(self.ivars().state_ptr as *const AudioHandlerState) };
            if state.generation.is_current(state.generation_token) {
                state.failure_tx.report(classify_native_stop_error(error));
            }
        }
    }
);

/// Tracks the last reported stream format so the capture diagnostics only
/// print when the format changes (avoids log spam at 50 buffers/second).
pub static FORMAT_SIGNATURE: std::sync::Mutex<Option<(u32, u32, u32)>> =
    std::sync::Mutex::new(None);

// Main-thread-only live capture state. Only ever accessed from main-thread
// closures (see `MainThreadDispatcher`).
thread_local! {
    static MAIN_STREAM: RefCell<Option<Retained<SCStream>>> = const { RefCell::new(None) };
    static MAIN_HANDLER: RefCell<Option<Retained<MimiAudioStreamHandler>>> =
        const { RefCell::new(None) };
    static MAIN_STATE: RefCell<Option<Box<AudioHandlerState>>> = const { RefCell::new(None) };
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
    generation_token: u64,
    teardown: F,
) where
    F: std::future::Future<Output = ()>,
{
    if generation.invalidate_if_current(generation_token) {
        started.store(false, Ordering::SeqCst);
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
    generation: CaptureGeneration,
    pending_teardown: PendingTeardown,
}

impl MacSystemAudioCapture {
    pub fn new(dispatcher: MainThreadDispatcher) -> Self {
        Self {
            dispatcher,
            started: Arc::new(AtomicBool::new(false)),
            generation: CaptureGeneration::default(),
            pending_teardown: PendingTeardown::default(),
        }
    }

    /// AppKit lists application metadata without requesting screen contents or
    /// recording authorization. Only starting capture may access ScreenCaptureKit.
    pub async fn audio_applications(&self) -> Result<ApplicationSnapshot, SystemAudioCaptureError> {
        let (tx, rx) = oneshot::channel();
        (self.dispatcher)(Box::new(move || {
            let result = objc2::exception::catch(AssertUnwindSafe(|| {
                let own = own_bundle_identifier();
                let mut running_apps = std::collections::HashMap::new();
                let mut applications = objc2_app_kit::NSWorkspace::sharedWorkspace()
                    .runningApplications()
                    .iter()
                    .filter_map(|app| {
                        let application = picker_application(
                            app.isTerminated(),
                            app.activationPolicy()
                                == objc2_app_kit::NSApplicationActivationPolicy::Regular,
                            app.bundleIdentifier().map(|id| id.to_string()),
                            app.localizedName().map(|name| name.to_string()),
                            own.as_deref(),
                        )?;
                        running_apps.entry(application.id.clone()).or_insert(app);
                        Some(application)
                    })
                    .collect();
                sort_applications(&mut applications);
                let mut icon_budget = ApplicationIconBudget::default();
                for application in &mut applications {
                    application.icon_data_url = running_apps
                        .get(&application.id)
                        .and_then(|app| {
                            objc2::exception::catch(AssertUnwindSafe(|| app.icon()))
                                .ok()
                                .flatten()
                        })
                        .and_then(|image| picker_icon_png(&image))
                        .and_then(|png| icon_budget.encode_png(&png));
                }
                ApplicationSnapshot {
                    supported: true,
                    applications,
                }
            }))
            .map_err(|_| SystemAudioCaptureError::ApplicationListFailed);
            let _ = tx.send(result);
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
        if self.started.swap(true, Ordering::SeqCst) {
            return Err(SystemAudioCaptureError::AlreadyRunning);
        }
        let generation_token = self.generation.begin();
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

        if super::macos_tap::use_audio_tap(&target, super::macos_tap::is_available()) {
            // Reserve teardown before spawning. A timed-out native permission
            // request must keep subsequent sources closed until it really exits.
            let result = super::macos_tap::start(
                audio_ingress,
                failure_tx,
                format,
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
        if !self.started.swap(false, Ordering::SeqCst) {
            return;
        }
        let generation_token = self.generation.invalidate();
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
            let state = MAIN_STATE.take();
            let handler = MAIN_HANDLER.take();
            let Some(stream) = stream else {
                drop(state);
                drop(handler);
                let _ = done_tx.send(());
                return;
            };
            // ScreenCaptureKit requires the stream and its output to stay
            // retained until the stop completion fires. Releasing them right
            // after calling stop_capture can leave the capture session
            // running while the handler reads freed state (use-after-free);
            // the stop completion is the only safe point to release them.
            // (The completion is typed `Fn`, so the values are released
            // through `Cell::take`, which is callable on a shared reference.)
            pipeline_log!("capture stop: stop_capture called");
            let completion_stream = std::cell::Cell::new(Some(stream.clone()));
            let completion_state = std::cell::Cell::new(state);
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
                drop(completion_state.take());
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

    let state = Box::new(AudioHandlerState {
        audio_ingress,
        failure_tx,
        generation: generation.clone(),
        generation_token,
        last_audio_buffer_at: Mutex::new(None),
        decoded_buffers: Mutex::new(0),
        resampler: Mutex::new(None),
        pending_frames: Mutex::new(Vec::new()),
        target_sample_rate_hz: format.sample_rate_hz,
    });
    let state_ptr: *mut c_void = (&*state as *const AudioHandlerState) as *mut c_void;
    let this = MimiAudioStreamHandler::alloc().set_ivars(MimiAudioStreamHandlerIvars { state_ptr });
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
        stop_uninstalled_capture(stream, Some(handler), Some(state), pending_teardown.clone());
        return Err(SystemAudioCaptureError::StartCancelled);
    }
    MAIN_GENERATION.set(Some(generation_token));
    MAIN_TARGET_APPS.with(|apps| *apps.borrow_mut() = target_apps);
    MAIN_STREAM.set(Some(stream));
    MAIN_HANDLER.set(Some(handler));
    MAIN_STATE.set(Some(state));
    if !generation.is_current(generation_token) {
        MAIN_GENERATION.set(None);
        MAIN_TARGET_APPS.with(|apps| apps.borrow_mut().clear());
        let stream = MAIN_STREAM
            .take()
            .expect("capture stream was just installed");
        let handler = MAIN_HANDLER.take();
        let state = MAIN_STATE.take();
        stop_uninstalled_capture(stream, handler, state, pending_teardown);
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
/// the stop, preventing the callback handler from observing freed state.
fn stop_uninstalled_capture(
    stream: Retained<SCStream>,
    handler: Option<Retained<MimiAudioStreamHandler>>,
    state: Option<Box<AudioHandlerState>>,
    pending_teardown: PendingTeardown,
) {
    let teardown_guard = pending_teardown.begin();
    let completion_stream = Cell::new(Some(stream.clone()));
    let completion_state = Cell::new(state);
    let completion_handler = Cell::new(handler);
    let completion_teardown = Cell::new(Some(teardown_guard));
    stream.stop_capture(move |error| {
        if error.is_some() {
            pipeline_log!("capture cleanup stop failed label=capture.native_stop_failed");
        }
        drop(completion_stream.take());
        drop(completion_state.take());
        drop(completion_handler.take());
        drop(completion_teardown.take());
    });
}

fn picker_application(
    terminated: bool,
    regular: bool,
    id: Option<String>,
    name: Option<String>,
    own: Option<&str>,
) -> Option<AudioApplication> {
    if terminated || !regular {
        return None;
    }
    let (id, name) = (id?, name?);
    let target = SystemAudioTarget::Application { id, name };
    if !target.validate() || target.application_id() == own {
        return None;
    }
    let SystemAudioTarget::Application { id, name } = target else {
        unreachable!()
    };
    Some(AudioApplication {
        id,
        name,
        icon_data_url: None,
    })
}

/// Render an existing AppKit icon into a fixed-size bitmap. This does not read
/// windows, enumerate screen contents, start capture or request a new permission.
/// Runtime callers execute on the main thread. Failures leave a text-only choice.
fn picker_icon_png(image: &objc2_app_kit::NSImage) -> Option<Vec<u8>> {
    use objc2_app_kit::{
        NSBitmapImageFileType, NSBitmapImageRep, NSCompositingOperation, NSDeviceRGBColorSpace,
        NSGraphicsContext,
    };
    use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize};
    const EDGE: isize = 32;
    let previous_context = NSGraphicsContext::currentContext();
    let result = objc2::exception::catch(AssertUnwindSafe(|| {
        // SAFETY: Null planes let AppKit own one 32x32 RGBA buffer with an
        // explicit 128-byte row. No caller-owned memory is retained by AppKit.
        let bitmap = unsafe {
            NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
                NSBitmapImageRep::alloc(), std::ptr::null_mut(), EDGE, EDGE, 8, 4,
                true, false, NSDeviceRGBColorSpace, EDGE * 4, 32,
            )
        }?;
        let pixels = bitmap.bitmapData();
        if pixels.is_null() || bitmap.bytesPerRow() != EDGE * 4 {
            return None;
        }
        // SAFETY: The bitmap owns this exact RGBA allocation. Clear alpha so
        // transparent padding never includes uninitialized bitmap bytes.
        unsafe { std::ptr::write_bytes(pixels, 0, (EDGE * EDGE * 4) as usize) };
        let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&bitmap)?;
        NSGraphicsContext::setCurrentContext(Some(&context));
        image.drawInRect_fromRect_operation_fraction(
            NSRect::new(NSPoint::ZERO, NSSize::new(EDGE as f64, EDGE as f64)),
            NSRect::ZERO, NSCompositingOperation::Copy, 1.0,
        );
        // SAFETY: An empty property dictionary supplies no incorrectly typed
        // codec options. The newly rendered representation contains no source metadata.
        let data = unsafe {
            bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
        }?;
        (data.len() <= MAX_APPLICATION_ICON_PNG_BYTES).then(|| data.to_vec())
    })).ok().flatten();
    // Restore even after an Objective-C drawing/encoding exception. A missing
    // app icon must not disturb the graphics context used by the next UI draw.
    NSGraphicsContext::setCurrentContext(previous_context.as_deref());
    result
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

fn own_bundle_identifier() -> Option<String> {
    use objc2_foundation::NSBundle;
    NSBundle::mainBundle()
        .bundleIdentifier()
        .map(|id| id.to_string())
}

/// Extracts the sample buffer as f32 mono samples (honoring the stream's
/// format description), resamples them to the provider rate with `rubato` when needed,
/// and quantizes to PCM16 for the ASR pipeline.
fn capture_to_pcm16(
    state: &AudioHandlerState,
    sample_buffer: CMSampleBufferRef,
) -> Result<Option<Vec<u8>>, SystemAudioCaptureError> {
    let (samples, sample_rate) = unsafe {
        let block_buffer = CMSampleBufferGetDataBuffer(sample_buffer);
        if block_buffer.is_null() {
            return Ok(None);
        }
        // The sample callback keeps this CoreMedia buffer and its format alive.
        // Decode its complete logical byte range, even across native blocks.
        // Empty buffers must return before requiring a format description.
        let Some(decoded) = macos_block_buffer::with_bytes(block_buffer, |bytes| {
            let format_description = CMSampleBufferGetFormatDescription(sample_buffer);
            if format_description.is_null() {
                return Err(SystemAudioCaptureError::UnsupportedAudioFormat);
            }
            let asbd = CMAudioFormatDescriptionGetStreamBasicDescription(format_description);
            if asbd.is_null() {
                return Err(SystemAudioCaptureError::UnsupportedAudioFormat);
            }
            let asbd = &*asbd;
            // Log the stream format once per format change.
            let signature = (
                asbd.mBitsPerChannel,
                asbd.mChannelsPerFrame,
                asbd.mFormatFlags as u32,
            );
            let mut last_format = FORMAT_SIGNATURE.lock().unwrap();
            if *last_format != Some(signature) {
                *last_format = Some(signature);
                let nonzero = bytes.iter().filter(|byte| **byte != 0).count();
                pipeline_log!(
                    "capture format bytes={} asbd={}Hz {}ch {}bit flags={:#x} nonzero={}/{}",
                    bytes.len(),
                    asbd.mSampleRate as u64,
                    asbd.mChannelsPerFrame,
                    asbd.mBitsPerChannel,
                    asbd.mFormatFlags,
                    nonzero,
                    bytes.len()
                );
            }

            decode_to_f32_mono(bytes, asbd).map(|samples| (samples, asbd.mSampleRate))
        })
        .map_err(|_| SystemAudioCaptureError::AudioProcessingFailed)?
        else {
            return Ok(None);
        };

        decoded?
    };

    if samples.is_empty() {
        return Ok(None);
    }
    // Per-buffer diagnostics are intentionally absent here: this runs ~48x/s
    // and the throttled `capture decode buffers=` stats plus the send
    // pipeline's peakDbFS already cover silence/decode diagnosis.

    let target_sample_rate = state.target_sample_rate_hz as f64;
    // Resample when the device rate differs from the selected provider.
    let pcm = if (sample_rate - target_sample_rate).abs() < 0.5 {
        PCM16Encoder::encode(&[samples])
    } else {
        let mut pending = state.pending_frames.lock().unwrap();
        pending.extend_from_slice(&samples);
        let mut resampler_guard = state.resampler.lock().unwrap();
        if resampler_guard.is_none() {
            let resampler = rubato::Fft::<f32>::new(
                sample_rate as usize,
                state.target_sample_rate_hz as usize,
                RESAMPLE_CHUNK_FRAMES,
                1,
                rubato::FixedSync::Input,
            )
            .map_err(|_| SystemAudioCaptureError::AudioProcessingFailed)?;
            *resampler_guard = Some(resampler);
        }
        let mut out = Vec::new();
        let resampler = resampler_guard.as_mut().unwrap();
        while pending.len() >= RESAMPLE_CHUNK_FRAMES {
            let output = match SequentialSlice::new(
                &pending[..RESAMPLE_CHUNK_FRAMES],
                1,
                RESAMPLE_CHUNK_FRAMES,
            ) {
                Ok(input) => resampler
                    .process(&input, None)
                    .map(|output| output.take_data())
                    .map_err(|_| SystemAudioCaptureError::AudioProcessingFailed),
                Err(_) => Err(SystemAudioCaptureError::AudioProcessingFailed),
            };
            // Match the previous drain-before-process behavior on both success
            // and failure while avoiding an owned input copy.
            pending.drain(..RESAMPLE_CHUNK_FRAMES);
            out.extend_from_slice(&output?);
        }
        if out.is_empty() {
            return Ok(None);
        }
        PCM16Encoder::encode(&[out])
    };

    Ok(Some(pcm))
}

const RESAMPLE_CHUNK_FRAMES: usize = 1024;

/// Decodes interleaved/planar linear PCM into mono f32 samples. Both the
/// float and signed-integer branches handle multi-channel buffers by
/// averaging channels, matching `PCM16Encoder::encode`.
pub(super) fn decode_to_f32_mono(
    bytes: &[u8],
    asbd: &AudioStreamBasicDescription,
) -> Result<Vec<f32>, SystemAudioCaptureError> {
    let channels = asbd.mChannelsPerFrame.max(1) as usize;
    let is_float = asbd.mFormatFlags & kAudioFormatFlagIsFloat != 0;
    let is_signed_int = asbd.mFormatFlags & kAudioFormatFlagIsSignedInteger != 0;
    let is_planar = asbd.mFormatFlags & kAudioFormatFlagIsNonInterleaved != 0;

    if is_planar {
        // Planar layout: channel data is contiguous per channel.
        let frames_per_channel = if is_float && asbd.mBitsPerChannel == 32 {
            bytes.len() / (4 * channels)
        } else if is_signed_int && asbd.mBitsPerChannel == 16 {
            bytes.len() / (2 * channels)
        } else if is_signed_int && asbd.mBitsPerChannel == 32 {
            bytes.len() / (4 * channels)
        } else {
            return Err(SystemAudioCaptureError::UnsupportedAudioFormat);
        };
        let bytes_per_sample = (asbd.mBitsPerChannel / 8) as usize;
        let mut mono = Vec::with_capacity(frames_per_channel);
        for frame in 0..frames_per_channel {
            let mut mixed = 0.0f64;
            for channel in 0..channels {
                let offset = (frame + channel * frames_per_channel) * bytes_per_sample;
                mixed += sample_to_f32(bytes, offset, is_float, asbd.mBitsPerChannel)? as f64;
            }
            mono.push((mixed / channels as f64) as f32);
        }
        Ok(mono)
    } else if is_float && asbd.mBitsPerChannel == 32 {
        let bytes_per_sample = 4;
        let frames = bytes.len() / (bytes_per_sample * channels);
        if frames == 0 {
            return Ok(Vec::new());
        }
        let mut mono = Vec::with_capacity(frames);
        for frame in 0..frames {
            let mut mixed = 0.0f64;
            for channel in 0..channels {
                let offset = (frame * channels + channel) * bytes_per_sample;
                let sample = f32::from_le_bytes([
                    bytes[offset],
                    bytes[offset + 1],
                    bytes[offset + 2],
                    bytes[offset + 3],
                ]);
                mixed += sample as f64;
            }
            mono.push((mixed / channels as f64) as f32);
        }
        Ok(mono)
    } else if is_signed_int && asbd.mBitsPerChannel == 16 {
        let bytes_per_sample = 2;
        let frames = bytes.len() / (bytes_per_sample * channels);
        if frames == 0 {
            return Ok(Vec::new());
        }
        let mut mono = Vec::with_capacity(frames);
        for frame in 0..frames {
            let mut mixed = 0.0f64;
            for channel in 0..channels {
                let offset = (frame * channels + channel) * bytes_per_sample;
                mixed += i16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as f64;
            }
            mono.push((mixed / channels as f64 / 32_768.0) as f32);
        }
        Ok(mono)
    } else if is_signed_int && asbd.mBitsPerChannel == 32 {
        let bytes_per_sample = 4;
        let frames = bytes.len() / (bytes_per_sample * channels);
        if frames == 0 {
            return Ok(Vec::new());
        }
        let mut mono = Vec::with_capacity(frames);
        for frame in 0..frames {
            let mut mixed = 0.0f64;
            for channel in 0..channels {
                let offset = (frame * channels + channel) * bytes_per_sample;
                let sample = i32::from_le_bytes([
                    bytes[offset],
                    bytes[offset + 1],
                    bytes[offset + 2],
                    bytes[offset + 3],
                ]);
                mixed += sample as f64;
            }
            mono.push((mixed / channels as f64 / 2_147_483_648.0) as f32);
        }
        Ok(mono)
    } else {
        Err(SystemAudioCaptureError::UnsupportedAudioFormat)
    }
}

fn sample_to_f32(
    bytes: &[u8],
    offset: usize,
    is_float: bool,
    bits: u32,
) -> Result<f32, SystemAudioCaptureError> {
    if is_float && bits == 32 {
        Ok(f32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]))
    } else if !is_float && bits == 16 {
        Ok(i16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as f32 / 32_768.0)
    } else if !is_float && bits == 32 {
        Ok(i32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as f32
            / 2_147_483_648.0)
    } else {
        Err(SystemAudioCaptureError::UnsupportedAudioFormat)
    }
}

#[cfg(test)]
mod resampler_tests {
    use objc2_core_audio_types::AudioStreamBasicDescription;

    fn asbd_48k_1ch_f32() -> AudioStreamBasicDescription {
        AudioStreamBasicDescription {
            mSampleRate: 48_000.0,
            mFormatID: 0x6c70636d, // kAudioFormatLinearPCM
            mFormatFlags: 0x29,    // float | packed | non-interleaved
            mBytesPerPacket: 4,
            mFramesPerPacket: 1,
            mBytesPerFrame: 4,
            mChannelsPerFrame: 1,
            mBitsPerChannel: 32,
            mReserved: 0,
        }
    }

    fn f32_bytes(samples: &[f32]) -> Vec<u8> {
        samples.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    #[test]
    fn decode_interleaved_f32_preserves_signal() {
        let samples = vec![0.5f32, -0.25, 1.0, -1.0, 0.0, 0.75];
        let bytes = f32_bytes(&samples);
        let mono = decode_to_f32_mono(&bytes, &asbd_48k_1ch_f32()).expect("decode ok");
        assert_eq!(mono.len(), samples.len());
        let peak = mono.iter().fold(0.0f32, |a, s| a.max(s.abs()));
        assert!(peak > 0.5, "decoded samples are near-silent (peak={peak})");
        assert!((mono[0] - 0.5).abs() < 1e-6);
        assert!((mono[3] - (-1.0)).abs() < 1e-6);
    }

    #[test]
    fn decode_handles_stereo_by_averaging_channels() {
        let asbd = AudioStreamBasicDescription {
            mChannelsPerFrame: 2,
            mFormatFlags: kAudioFormatFlagIsFloat
                | objc2_core_audio_types::kAudioFormatFlagIsPacked,
            ..asbd_48k_1ch_f32()
        };
        // Interleaved: L=0.5 R=0.5 -> mono 0.5; L=-1 R=1 -> mono 0.
        let bytes = f32_bytes(&[0.5, 0.5, -1.0, 1.0]);
        let mono = decode_to_f32_mono(&bytes, &asbd).expect("decode ok");
        assert_eq!(mono.len(), 2);
        assert!((mono[0] - 0.5).abs() < 1e-6);
        assert!(mono[1].abs() < 1e-6);
    }

    #[test]
    fn planar_stereo_f32_preserves_frame_order() {
        let asbd = AudioStreamBasicDescription {
            mChannelsPerFrame: 2,
            mFormatFlags: kAudioFormatFlagIsFloat | kAudioFormatFlagIsNonInterleaved,
            ..asbd_48k_1ch_f32()
        };
        // L and R are separate planes; matching frames always average to 0.5.
        let bytes = f32_bytes(&[0.0, 0.25, 0.5, 0.75, 1.0, 0.75, 0.5, 0.25]);
        let mono = decode_to_f32_mono(&bytes, &asbd).unwrap();
        assert_eq!(mono, [0.5; 4]);
    }

    #[test]
    fn planar_signed_integer_channels_keep_all_frames_in_order() {
        for bits in [16, 32] {
            let bytes = if bits == 16 {
                [16_384i16, -16_384, 0, 0]
                    .into_iter()
                    .flat_map(i16::to_le_bytes)
                    .collect::<Vec<_>>()
            } else {
                [1_073_741_824i32, -1_073_741_824, 0, 0]
                    .into_iter()
                    .flat_map(i32::to_le_bytes)
                    .collect::<Vec<_>>()
            };
            let asbd = AudioStreamBasicDescription {
                mChannelsPerFrame: 2,
                mBitsPerChannel: bits,
                mBytesPerFrame: bits / 8,
                mBytesPerPacket: bits / 8,
                mFormatFlags: kAudioFormatFlagIsSignedInteger | kAudioFormatFlagIsNonInterleaved,
                ..asbd_48k_1ch_f32()
            };
            assert_eq!(decode_to_f32_mono(&bytes, &asbd).unwrap(), [0.25, -0.25]);
        }
    }

    #[test]
    fn nonmixable_flag_is_not_a_planar_layout_flag() {
        let asbd = AudioStreamBasicDescription {
            mChannelsPerFrame: 2,
            mFormatFlags: kAudioFormatFlagIsFloat
                | objc2_core_audio_types::kAudioFormatFlagIsNonMixable,
            ..asbd_48k_1ch_f32()
        };
        let bytes = f32_bytes(&[0.0, 0.25, 0.5, 0.75]);
        assert_eq!(decode_to_f32_mono(&bytes, &asbd).unwrap(), [0.125, 0.625]);
    }

    use super::*;
    use rubato::audioadapter::Adapter;
    use rubato::audioadapter_buffers::direct::SequentialSliceOfVecs;
    use rubato::FixedSync;
    use rubato::Resampler;

    fn peak(data: &[f32]) -> f32 {
        data.iter().fold(0.0f32, |acc, s| acc.max(s.abs()))
    }

    #[test]
    fn application_picker_keeps_regular_apps_without_window_or_capture_metadata() {
        let own = Some("app.yuxino.mimi");
        let app = |terminated, regular, id: Option<&str>, name: Option<&str>| {
            picker_application(
                terminated,
                regular,
                id.map(str::to_owned),
                name.map(str::to_owned),
                own,
            )
        };
        let player = app(false, true, Some("test.player"), Some("Player")).unwrap();
        assert_eq!(player.id, "test.player");
        assert_eq!(player.name, "Player");
        // Windowless and hidden regular applications remain selectable; no
        // screen/window metadata or recording grant enters this decision.
        assert!(app(true, true, Some("test.player"), Some("Player")).is_none());
        assert!(app(false, false, Some("test.helper"), Some("Helper")).is_none());
        assert!(app(false, true, own, Some("Mimi")).is_none());
        assert!(app(false, true, None, Some("Player")).is_none());
        assert!(app(false, true, Some("test.player"), None).is_none());
        assert!(app(false, true, Some("test.player"), Some(" ")).is_none());
        assert!(app(false, true, Some("test.player\n"), Some("Player")).is_none());
    }

    #[test]
    fn application_picker_icon_is_a_small_png_and_restores_the_graphics_context() {
        use objc2_app_kit::{NSGraphicsContext, NSImage};
        let data =
            objc2_foundation::NSData::with_bytes(include_bytes!("../../icons/128x128@2x.png"));
        let image = NSImage::initWithData(NSImage::alloc(), &data).unwrap();
        let previous = NSGraphicsContext::currentContext();
        let png = picker_icon_png(&image).unwrap();
        assert!(png.len() <= MAX_APPLICATION_ICON_PNG_BYTES);
        let decoded = tauri::image::Image::from_bytes(&png).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (32, 32));
        assert!(decoded
            .rgba()
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[3] != 0));
        assert_eq!(NSGraphicsContext::currentContext(), previous);
    }

    #[test]
    fn native_permission_and_user_stop_errors_are_terminal_at_start_and_runtime() {
        for (code, start, runtime) in [
            (
                SCStreamErrorCode::UserDeclined,
                SystemAudioCaptureError::PermissionDenied,
                SystemAudioCaptureFailure::PermissionDenied,
            ),
            (
                SCStreamErrorCode::UserStopped,
                SystemAudioCaptureError::UserStopped,
                SystemAudioCaptureFailure::UserStopped,
            ),
        ] {
            // No native stream is created: only NSError classification is tested.
            let error = unsafe {
                objc2_foundation::NSError::errorWithDomain_code_userInfo(
                    SCStreamErrorDomain,
                    code.0,
                    None,
                )
            };
            assert_eq!(classify_native_start_error(&error), start);
            assert_eq!(classify_native_stop_error(&error), runtime);
            assert!(!runtime.is_recoverable());
        }
    }

    #[test]
    fn unrelated_native_errors_do_not_claim_a_permission_denial() {
        for (domain, code) in [
            (
                objc2_foundation::ns_string!("test.other"),
                SCStreamErrorCode::UserDeclined.0,
            ),
            (
                unsafe { SCStreamErrorDomain },
                SCStreamErrorCode::InternalError.0,
            ),
        ] {
            let error = unsafe {
                objc2_foundation::NSError::errorWithDomain_code_userInfo(domain, code, None)
            };
            assert_eq!(
                classify_native_start_error(&error),
                SystemAudioCaptureError::NativeStartFailed
            );
            let failure = classify_native_stop_error(&error);
            assert_eq!(failure, SystemAudioCaptureFailure::NativeStopped);
            assert!(failure.is_recoverable());
        }
    }

    #[test]
    fn stopping_invalidates_a_pending_capture_generation() {
        let generation = CaptureGeneration::default();
        let first = generation.begin();
        assert!(generation.is_current(first));

        assert_eq!(generation.invalidate(), first);
        assert!(!generation.is_current(first));

        let second = generation.begin();
        assert_ne!(second, first);
        assert!(generation.is_current(second));
        assert!(!generation.invalidate_if_current(first));
        assert!(generation.is_current(second));
    }

    #[tokio::test]
    async fn native_completion_does_not_acknowledge_before_stream_installation() {
        let (tx, mut rx) = oneshot::channel();
        let barrier = CaptureStartBarrier::new(tx);

        barrier.did_complete(Ok(()));
        assert!(rx.try_recv().is_err());
        barrier.did_install();

        assert_eq!(rx.await.unwrap(), Ok(()));
    }

    #[tokio::test]
    async fn stream_installation_does_not_acknowledge_before_native_completion() {
        let (tx, mut rx) = oneshot::channel();
        let barrier = CaptureStartBarrier::new(tx);

        barrier.did_install();
        assert!(rx.try_recv().is_err());
        barrier.did_complete(Ok(()));

        assert_eq!(rx.await.unwrap(), Ok(()));
    }

    #[tokio::test]
    async fn capture_start_barrier_delivers_failure_only_once() {
        let (tx, rx) = oneshot::channel();
        let barrier = CaptureStartBarrier::new(tx);

        barrier.did_complete(Ok(()));
        barrier.fail(SystemAudioCaptureError::StartCancelled);
        barrier.did_install();

        assert_eq!(
            rx.await.unwrap(),
            Err(SystemAudioCaptureError::StartCancelled)
        );
    }

    #[tokio::test]
    async fn timeout_after_phase_two_install_tears_down_token_and_allows_retry() {
        let generation = CaptureGeneration::default();
        let started = AtomicBool::new(true);
        let timed_out_token = generation.begin();
        let installed_stream = Arc::new(Mutex::new(Some(timed_out_token)));

        // This channel is the phase-2 barrier: the native stream is already
        // installed, but its start acknowledgement has not reached `start`.
        let (phase_two_installed_tx, phase_two_installed_rx) = oneshot::channel();
        phase_two_installed_tx.send(()).unwrap();
        phase_two_installed_rx.await.unwrap();

        let teardown_slot = Arc::clone(&installed_stream);
        finish_failed_capture_start(&generation, &started, timed_out_token, async move {
            let mut slot = teardown_slot.lock().unwrap();
            if *slot == Some(timed_out_token) {
                *slot = None;
            }
        })
        .await;

        assert!(!started.load(Ordering::SeqCst));
        assert_eq!(*installed_stream.lock().unwrap(), None);

        let retry_token = generation.begin();
        let mut slot = installed_stream.lock().unwrap();
        assert!(slot.is_none(), "stale stream would reject a retry");
        *slot = Some(retry_token);
        assert_eq!(*slot, Some(retry_token));
    }

    #[tokio::test]
    async fn source_switch_waits_for_queued_main_thread_teardown_after_stop_is_dropped() {
        let (dispatch_tx, dispatch_rx) = std::sync::mpsc::channel::<Box<dyn FnOnce() + Send>>();
        let capture = MacSystemAudioCapture::new(Arc::new(move |task| {
            dispatch_tx.send(task).unwrap();
        }));
        capture.started.store(true, Ordering::SeqCst);
        capture.generation.begin();
        assert!(
            tokio::time::timeout(Duration::from_millis(10), capture.stop())
                .await
                .is_err()
        );
        assert!(!capture.started.load(Ordering::SeqCst));
        // The second stop returns immediately, but it cannot imply native
        // release while the first teardown is still waiting for the main queue.
        capture.stop().await;
        assert!(
            tokio::time::timeout(Duration::from_millis(10), capture.wait_until_idle())
                .await
                .is_err()
        );
        dispatch_rx.try_recv().unwrap()();
        capture.wait_until_idle().await.unwrap();
    }

    #[tokio::test]
    async fn timed_out_native_stop_keeps_the_teardown_barrier_closed() {
        let pending = PendingTeardown::default();
        let native_completion = pending.begin();

        assert!(
            tokio::time::timeout(Duration::from_millis(10), pending.wait_until_clear(),)
                .await
                .is_err(),
            "a hung native completion must keep a retry waiting"
        );
        assert!(
            pending.is_pending(),
            "the caller timeout must not abandon native teardown ownership"
        );

        drop(native_completion);
        tokio::time::timeout(Duration::from_millis(100), pending.wait_until_clear())
            .await
            .expect("late completion releases the teardown barrier");
        assert!(!pending.is_pending());
    }

    #[tokio::test]
    async fn late_native_stop_completion_unblocks_every_waiting_retry() {
        let pending = PendingTeardown::default();
        let native_completion = pending.begin();
        let first_pending = pending.clone();
        let second_pending = pending.clone();
        let first_retry = tokio::spawn(async move {
            first_pending.wait_until_clear().await;
        });
        let second_retry = tokio::spawn(async move {
            second_pending.wait_until_clear().await;
        });

        tokio::task::yield_now().await;
        assert!(!first_retry.is_finished());
        assert!(!second_retry.is_finished());

        // Model ScreenCaptureKit invoking its completion after the caller's
        // bounded stop wait has already returned.
        drop(native_completion);
        tokio::time::timeout(Duration::from_millis(100), async {
            first_retry.await.unwrap();
            second_retry.await.unwrap();
        })
        .await
        .expect("all retries resume after the real native completion");
    }

    #[test]
    fn native_configuration_preserves_provider_sample_rate() {
        let configuration = SCStreamConfiguration::new();
        for sample_rate_hz in [16_000, 24_000] {
            set_capture_sample_rate(&configuration, sample_rate_hz);
            // SAFETY: The native getter returns NSInteger, rather than the crate's f64.
            let actual: isize = unsafe { msg_send![&*configuration, sampleRate] };
            assert_eq!(actual, sample_rate_hz as isize);
        }
    }

    #[test]
    fn fft_resampler_48000_to_16000_produces_non_silent_output() {
        let mut resampler = rubato::Fft::<f32>::new(48_000, 16_000, 1024, 1, FixedSync::Input)
            .expect("resampler builds");
        let input_frames = resampler.input_frames_next();
        // 1 kHz sine at 48 kHz.
        let samples: Vec<f32> = (0..input_frames)
            .map(|i| (i as f64 * 2.0 * std::f64::consts::PI * 1000.0 / 48_000.0).sin() as f32 * 0.5)
            .collect();
        let binding = [samples];
        let input = SequentialSliceOfVecs::new(&binding, 1, input_frames).expect("adapter builds");
        let output = resampler.process(&input, None).expect("process succeeds");
        let frames = output.frames();
        let data = output.take_data();

        assert!((200..=350).contains(&frames));
        assert!(!data.is_empty(), "resampler produced no output");
        assert!(
            peak(&data) > 0.01,
            "resampled output is silent (peak={})",
            peak(&data)
        );
    }

    #[test]
    fn fft_resampler_48000_to_openai_24000_produces_non_silent_output() {
        let mut resampler = rubato::Fft::<f32>::new(48_000, 24_000, 1024, 1, FixedSync::Input)
            .expect("resampler builds");
        let input_frames = resampler.input_frames_next();
        let samples: Vec<f32> = (0..input_frames)
            .map(|index| {
                (index as f64 * 2.0 * std::f64::consts::PI * 1_000.0 / 48_000.0).sin() as f32 * 0.5
            })
            .collect();
        let binding = [samples];
        let input = SequentialSliceOfVecs::new(&binding, 1, input_frames).expect("adapter builds");
        let output = resampler.process(&input, None).expect("process succeeds");
        let data = output.take_data();

        assert!(!data.is_empty());
        assert!(peak(&data) > 0.01);
    }

    #[test]
    fn fft_resampler_streams_multiple_chunks_without_silence() {
        let mut resampler = rubato::Fft::<f32>::new(48_000, 16_000, 1024, 1, FixedSync::Input)
            .expect("resampler builds");
        let input_frames = resampler.input_frames_next();
        let samples: Vec<f32> = (0..input_frames)
            .map(|i| (i as f64 * 2.0 * std::f64::consts::PI * 1000.0 / 48_000.0).sin() as f32 * 0.5)
            .collect();
        let binding = [samples.clone()];
        let input = SequentialSliceOfVecs::new(&binding, 1, input_frames).expect("adapter");

        let _warmup = resampler
            .process(&input, None)
            .expect("process 1")
            .take_data();
        let binding2 = [samples];
        let input2 = SequentialSliceOfVecs::new(&binding2, 1, input_frames).expect("adapter");
        let out2 = resampler
            .process(&input2, None)
            .expect("process 2")
            .take_data();

        assert!(
            peak(&out2) > 0.01,
            "streaming second chunk is silent (out2_peak={})",
            peak(&out2)
        );
    }

    #[test]
    fn fft_resampler_reports_expected_chunk_sizes() {
        let resampler = rubato::Fft::<f32>::new(48_000, 16_000, 1024, 1, FixedSync::Input)
            .expect("resampler builds");
        // The first block is a warm-up chunk (fewer frames); subsequent
        // blocks output ≈1024/3 frames. The streaming test above asserts
        // non-silent, correctly-sized output across multiple chunks.
        assert!(
            (200..=350).contains(&resampler.output_frames_next()),
            "unexpected output chunk size: {}",
            resampler.output_frames_next()
        );
    }
}
