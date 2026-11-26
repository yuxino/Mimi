//! Audio-only system and selected-application capture via Core Audio process taps (macOS 14.2+).
//!
//! A worker owns all native resources. The IOProc only copies a bounded mono
//! buffer into a bounded queue; decoding and resampling happen off the audio
//! thread. No ScreenCaptureKit enumeration or microphone device is involved.

use super::macos::{decode_to_f32_mono, CaptureGeneration, PendingTeardownGuard};
use super::macos_tap_target::ApplicationTapTarget;
use super::send_pipeline::{AudioIngress, AudioIngressError};
use super::streaming_resampler::StreamingPcm16Resampler;
use super::{
    AudioCaptureFormat, CaptureFailureSender, SystemAudioCaptureError, SystemAudioCaptureFailure,
};
use crate::core::audio::system_audio_target::SystemAudioTarget;
use crate::pipeline_log;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::AnyThread;
use objc2_core_audio::*;
use objc2_core_audio_types::{
    kAudioFormatFlagIsAlignedHigh, kAudioFormatFlagIsBigEndian, kAudioFormatFlagIsFloat,
    kAudioFormatFlagIsSignedInteger, kAudioFormatLinearPCM, AudioBufferList,
    AudioStreamBasicDescription, AudioTimeStamp,
};
use objc2_core_foundation::CFDictionary;
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString, NSUUID};
use std::ffi::{c_void, CStr};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

const MAX_CALLBACK_BYTES: usize = 64 * 1024;
// Absorb short worker scheduling pauses without restarting capture. The byte
// budget separately limits queued + processing PCM to one second at the
// negotiated format; the slot limit also bounds tiny callback allocations.
const CALLBACK_QUEUE_CAPACITY: usize = 256;
const POLL_INTERVAL: Duration = Duration::from_millis(100);
const HEALTH_INTERVAL: Duration = Duration::from_millis(500);

type CreateTap = unsafe extern "C-unwind" fn(Option<&CATapDescription>, *mut AudioObjectID) -> i32;
type DestroyTap = unsafe extern "C-unwind" fn(AudioObjectID) -> i32;

#[derive(Clone, Copy)]
struct TapApi {
    create: CreateTap,
    destroy: DestroyTap,
}

impl TapApi {
    fn load() -> Option<Self> {
        static API: OnceLock<Option<TapApi>> = OnceLock::new();
        *API.get_or_init(|| {
            AnyClass::get(c"CATapDescription")?;
            // SAFETY: CoreAudio is linked by the existing audio backend. Resolve
            // only these 14.2+ entry points dynamically so macOS 13 can still
            // load the executable. The signatures match AudioHardwareTapping.h.
            unsafe {
                let create = libc::dlsym(
                    libc::RTLD_DEFAULT,
                    c"AudioHardwareCreateProcessTap".as_ptr(),
                );
                let destroy = libc::dlsym(
                    libc::RTLD_DEFAULT,
                    c"AudioHardwareDestroyProcessTap".as_ptr(),
                );
                if create.is_null() || destroy.is_null() {
                    return None;
                }
                Some(Self {
                    create: std::mem::transmute::<*mut c_void, CreateTap>(create),
                    destroy: std::mem::transmute::<*mut c_void, DestroyTap>(destroy),
                })
            }
        })
    }
}

pub(super) fn is_available() -> bool {
    TapApi::load().is_some()
}

pub(super) fn use_audio_tap(available: bool, screen_capture_authorized: bool) -> bool {
    // Preserve existing authorization rather than moving an already working
    // installation to a separately permissioned API. Preflight never prompts.
    available && !screen_capture_authorized
}

pub(super) async fn start(
    ingress: AudioIngress,
    failure: CaptureFailureSender,
    format: AudioCaptureFormat,
    target: SystemAudioTarget,
    generation: CaptureGeneration,
    token: u64,
    teardown: PendingTeardownGuard,
) -> Result<(), SystemAudioCaptureError> {
    let api = TapApi::load().ok_or(SystemAudioCaptureError::NativeStartFailed)?;
    let (ready_tx, ready_rx) = oneshot::channel();
    std::thread::Builder::new()
        .name("mimi-system-audio-tap".into())
        .spawn(move || {
            objc2::rc::autoreleasepool(|_| {
                let mut native = NativeTap::new(api, teardown);
                let setup = objc2::exception::catch(std::panic::AssertUnwindSafe(|| {
                    native.prepare(&generation, token, failure.clone(), &target)
                }))
                .unwrap_or(Err(SystemAudioCaptureError::NativeStartFailed));
                let (asbd, receiver) = match setup {
                    Ok(value) => value,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                let resampler =
                    StreamingPcm16Resampler::new(asbd.mSampleRate as u32, format.sample_rate_hz, 1);
                let mut resampler = match resampler {
                    Ok(resampler) => resampler,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                if !generation.is_current(token) {
                    let _ = ready_tx.send(Err(SystemAudioCaptureError::StartCancelled));
                    return;
                }
                // The context and teardown reservation already exist before
                // starting IO. A late success after cancellation is destroyed
                // by this worker before another source can start.
                pipeline_log!("capture tap setup stage=start_device");
                let result = status(unsafe { AudioDeviceStart(native.device, native.io_proc) });
                if let Err(error) = result {
                    let _ = ready_tx.send(Err(error));
                    return;
                }
                native.started = true;
                if !generation.is_current(token) {
                    let _ = ready_tx.send(Err(SystemAudioCaptureError::StartCancelled));
                    return;
                }
                if ready_tx.send(Ok(())).is_err() {
                    return;
                }
                pipeline_log!(
                    "capture started backend=core_audio_tap sample_rate={}",
                    asbd.mSampleRate
                );
                let mut health_at = Instant::now();
                let mut target_at = Instant::now();
                while generation.is_current(token) && !failure.has_reported() {
                    match receiver.recv_timeout(POLL_INTERVAL) {
                        Ok(packet) => {
                            if !generation.is_current(token) {
                                break;
                            }
                            if !native.format_matches(&asbd) {
                                failure.report(SystemAudioCaptureFailure::NativeStopped);
                                break;
                            }
                            let result = decode_to_f32_mono(&packet.bytes, &asbd)
                                .and_then(|samples| resampler.push_interleaved(&samples));
                            match result {
                                Ok(buffers) => {
                                    for pcm in buffers {
                                        if !generation.is_current(token) {
                                            break;
                                        }
                                        if let Err(error) = ingress.try_send(pcm) {
                                            failure.report(match error {
                                                AudioIngressError::Backpressure => {
                                                    SystemAudioCaptureFailure::Backpressure
                                                }
                                                AudioIngressError::Closed => {
                                                    SystemAudioCaptureFailure::NativeStopped
                                                }
                                            });
                                            break;
                                        }
                                    }
                                }
                                Err(_) => {
                                    failure
                                        .report(SystemAudioCaptureFailure::AudioProcessingFailed);
                                }
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                    if target_at.elapsed() >= POLL_INTERVAL {
                        if let Err(error) = native.refresh_target() {
                            failure.report(
                                if error == SystemAudioCaptureError::ApplicationUnavailable {
                                    SystemAudioCaptureFailure::ApplicationUnavailable
                                } else {
                                    SystemAudioCaptureFailure::NativeStopped
                                },
                            );
                            break;
                        }
                        target_at = Instant::now();
                    }
                    if health_at.elapsed() >= HEALTH_INTERVAL {
                        // A changed route/format must reconnect rather than
                        // interpreting new buffers with the previous format.
                        if !native.is_healthy(&asbd) {
                            failure.report(SystemAudioCaptureFailure::NativeStopped);
                        }
                        health_at = Instant::now();
                    }
                }
            });
        })
        .map_err(|_| SystemAudioCaptureError::NativeStartFailed)?;
    tokio::time::timeout(Duration::from_secs(10), ready_rx)
        .await
        .map_err(|_| SystemAudioCaptureError::StartTimedOut)?
        .map_err(|_| SystemAudioCaptureError::NativeStartFailed)?
}

struct CallbackState {
    sender: SyncSender<CallbackPacket>,
    queued_bytes: Arc<AtomicUsize>,
    byte_limit: usize,
    failure: CaptureFailureSender,
    generation: CaptureGeneration,
    token: u64,
    bytes_per_frame: usize,
}

struct CallbackPacket {
    bytes: Vec<u8>,
    queued_bytes: Arc<AtomicUsize>,
}

impl Drop for CallbackPacket {
    fn drop(&mut self) {
        self.queued_bytes
            .fetch_sub(self.bytes.len(), Ordering::Relaxed);
    }
}

impl CallbackState {
    fn reserve_bytes(&self, byte_count: usize) -> bool {
        let mut queued = self.queued_bytes.load(Ordering::Relaxed);
        loop {
            let Some(total) = queued
                .checked_add(byte_count)
                .filter(|total| *total <= self.byte_limit)
            else {
                return false;
            };
            match self.queued_bytes.compare_exchange_weak(
                queued,
                total,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(current) => queued = current,
            }
        }
    }

    fn report_backpressure(&self, limit: &str) {
        pipeline_log!(
            "capture tap callback queue full limit={} queued_bytes={} byte_limit={} slots={}",
            limit,
            self.queued_bytes.load(Ordering::Relaxed),
            self.byte_limit,
            CALLBACK_QUEUE_CAPACITY
        );
        self.failure.report(SystemAudioCaptureFailure::Backpressure);
    }
}

unsafe extern "C-unwind" fn audio_callback(
    _device: AudioObjectID,
    _now: NonNull<AudioTimeStamp>,
    input: NonNull<AudioBufferList>,
    _input_time: NonNull<AudioTimeStamp>,
    _output: NonNull<AudioBufferList>,
    _output_time: NonNull<AudioTimeStamp>,
    context: *mut c_void,
) -> i32 {
    // SAFETY: NativeTap owns the stable Box until DestroyIOProcID succeeds.
    // Core Audio owns input for this call only; no native pointer is queued.
    let state = unsafe { &*context.cast::<CallbackState>() };
    if !state.generation.is_current(state.token) || state.failure.has_reported() {
        return 0;
    }
    let input = unsafe { input.as_ref() };
    if input.mNumberBuffers == 0 {
        return 0;
    }
    // The private mono mixdown has exactly one input buffer. Fail closed if
    // the aggregate unexpectedly exposes another stream (e.g. a microphone).
    if input.mNumberBuffers != 1 {
        state
            .failure
            .report(SystemAudioCaptureFailure::AudioProcessingFailed);
        return 0;
    }
    let buffer = &input.mBuffers[0];
    let byte_count = buffer.mDataByteSize as usize;
    if buffer.mData.is_null() || byte_count == 0 {
        return 0;
    }
    if !valid_buffer(buffer.mNumberChannels, byte_count, state.bytes_per_frame) {
        state
            .failure
            .report(SystemAudioCaptureFailure::AudioProcessingFailed);
        return 0;
    }
    if !state.reserve_bytes(byte_count) {
        state.report_backpressure("bytes");
        return 0;
    }
    let bytes =
        unsafe { std::slice::from_raw_parts(buffer.mData.cast::<u8>(), byte_count) }.to_vec();
    let packet = CallbackPacket {
        bytes,
        queued_bytes: Arc::clone(&state.queued_bytes),
    };
    if let Err(TrySendError::Full(packet)) = state.sender.try_send(packet) {
        drop(packet);
        state.report_backpressure("slots");
    }
    0
}

fn valid_buffer(channels: u32, bytes: usize, bytes_per_frame: usize) -> bool {
    channels == 1
        && bytes <= MAX_CALLBACK_BYTES
        && bytes_per_frame > 0
        && bytes.is_multiple_of(bytes_per_frame)
}

struct NativeTap {
    api: TapApi,
    tap: AudioObjectID,
    device: AudioObjectID,
    output_device: AudioObjectID,
    io_proc: AudioDeviceIOProcID,
    started: bool,
    context: Option<Box<CallbackState>>,
    teardown: Option<PendingTeardownGuard>,
    application: Option<ApplicationTapTarget>,
    description: Option<Retained<CATapDescription>>,
    process_ids: Vec<AudioObjectID>,
}

impl NativeTap {
    fn new(api: TapApi, teardown: PendingTeardownGuard) -> Self {
        Self {
            api,
            tap: 0,
            device: 0,
            output_device: 0,
            io_proc: None,
            started: false,
            context: None,
            teardown: Some(teardown),
            application: None,
            description: None,
            process_ids: Vec::new(),
        }
    }

    fn prepare(
        &mut self,
        generation: &CaptureGeneration,
        token: u64,
        failure: CaptureFailureSender,
        target: &SystemAudioTarget,
    ) -> Result<(AudioStreamBasicDescription, Receiver<CallbackPacket>), SystemAudioCaptureError>
    {
        let check_current = || {
            if generation.is_current(token) {
                Ok(())
            } else {
                Err(SystemAudioCaptureError::StartCancelled)
            }
        };
        check_current()?;
        let pid = std::process::id() as libc::pid_t;
        pipeline_log!("capture tap setup stage=translate_process");
        let own_process: AudioObjectID = property(
            kAudioObjectSystemObject as AudioObjectID,
            kAudioHardwarePropertyTranslatePIDToProcessObject,
            kAudioObjectPropertyScopeGlobal,
            Some(&pid),
        )?;
        if own_process == kAudioObjectUnknown as AudioObjectID {
            // Never substitute a PID for an AudioObjectID or include our own
            // playback merely because process translation was unavailable.
            return Err(SystemAudioCaptureError::NativeStartFailed);
        }
        self.application = target
            .application_id()
            .map(ApplicationTapTarget::new)
            .transpose()?;
        self.process_ids = match &self.application {
            Some(application) => application.processes()?,
            None => vec![own_process],
        };
        let numbers: Vec<_> = self
            .process_ids
            .iter()
            .map(|id| NSNumber::new_u32(*id))
            .collect();
        let processes = NSArray::from_retained_slice(&numbers);
        pipeline_log!("capture tap setup stage=create_description");
        // SAFETY: This is called only after both 14.2 tap symbols are found.
        let description = unsafe {
            let description = if self.application.is_some() {
                CATapDescription::initMonoMixdownOfProcesses(CATapDescription::alloc(), &processes)
            } else {
                CATapDescription::initMonoGlobalTapButExcludeProcesses(
                    CATapDescription::alloc(),
                    &processes,
                )
            };
            description.setPrivate(true);
            description.setMuteBehavior(CATapMuteBehavior::Unmuted);
            description.setName(&NSString::from_str("Mimi system audio"));
            description
        };
        check_current()?;
        pipeline_log!("capture tap setup stage=create_tap");
        status(unsafe { (self.api.create)(Some(&description), &mut self.tap) })?;
        self.description = Some(description.clone());
        check_current()?;
        pipeline_log!("capture tap setup stage=read_tap_format");
        let asbd: AudioStreamBasicDescription = property(
            self.tap,
            kAudioTapPropertyFormat,
            kAudioObjectPropertyScopeGlobal,
            None,
        )?;
        validate_format(&asbd)?;
        pipeline_log!("capture tap setup stage=read_output_device");
        self.output_device = property(
            kAudioObjectSystemObject as AudioObjectID,
            kAudioHardwarePropertyDefaultOutputDevice,
            kAudioObjectPropertyScopeGlobal,
            None,
        )?;
        let uid = unsafe { description.UUID().UUIDString() };
        let enabled = NSNumber::new_bool(true);
        let disabled = NSNumber::new_bool(false);
        let tap = dictionary(&[
            (kAudioSubTapUIDKey, &uid),
            (kAudioSubTapDriftCompensationKey, &enabled),
        ]);
        let taps = NSArray::from_retained_slice(&[tap]);
        let aggregate_uid = NSUUID::new().UUIDString();
        let name = NSString::from_str("Mimi private audio tap");
        let aggregate = dictionary(&[
            (kAudioAggregateDeviceNameKey, &name),
            (kAudioAggregateDeviceUIDKey, &aggregate_uid),
            (kAudioAggregateDeviceIsPrivateKey, &enabled),
            (kAudioAggregateDeviceIsStackedKey, &disabled),
            // The SDK documents that true waits for the first tapped audio.
            // Starting subtitles while the computer is quiet must not block
            // native setup or consume its timeout before playback begins.
            (kAudioAggregateDeviceTapAutoStartKey, &disabled),
            (kAudioAggregateDeviceTapListKey, &taps),
        ]);
        // SAFETY: NSDictionary is toll-free bridged to CFDictionary. The keys
        // and values above match Core Audio's aggregate/tap dictionary schema.
        let aggregate_cf = unsafe {
            &*((&*aggregate as *const NSDictionary<NSString, AnyObject>).cast::<CFDictionary>())
        };
        pipeline_log!("capture tap setup stage=create_aggregate");
        status(unsafe {
            AudioHardwareCreateAggregateDevice(aggregate_cf, NonNull::from(&mut self.device))
        })?;
        check_current()?;
        pipeline_log!("capture tap setup stage=read_aggregate_format");
        let device_format: AudioStreamBasicDescription = property(
            self.device,
            kAudioDevicePropertyStreamFormat,
            kAudioObjectPropertyScopeInput,
            None,
        )?;
        if !same_format(&asbd, &device_format) {
            return Err(SystemAudioCaptureError::UnsupportedAudioFormat);
        }
        let (sender, receiver) = mpsc::sync_channel(CALLBACK_QUEUE_CAPACITY);
        self.context = Some(Box::new(CallbackState {
            sender,
            queued_bytes: Arc::new(AtomicUsize::new(0)),
            byte_limit: asbd.mSampleRate as usize * asbd.mBytesPerFrame as usize,
            failure,
            generation: generation.clone(),
            token,
            bytes_per_frame: asbd.mBytesPerFrame as usize,
        }));
        let context = self.context.as_deref_mut().unwrap() as *mut CallbackState;
        pipeline_log!("capture tap setup stage=create_ioproc");
        status(unsafe {
            AudioDeviceCreateIOProcID(
                self.device,
                Some(audio_callback),
                context.cast(),
                NonNull::from(&mut self.io_proc),
            )
        })?;
        check_current()?;
        Ok((asbd, receiver))
    }

    fn is_healthy(&self, expected: &AudioStreamBasicDescription) -> bool {
        let output: Result<AudioObjectID, _> = property(
            kAudioObjectSystemObject as AudioObjectID,
            kAudioHardwarePropertyDefaultOutputDevice,
            kAudioObjectPropertyScopeGlobal,
            None,
        );
        let running: Result<u32, _> = property(
            self.device,
            kAudioDevicePropertyDeviceIsRunning,
            kAudioObjectPropertyScopeGlobal,
            None,
        );
        output.is_ok_and(|output| output == self.output_device)
            && self.format_matches(expected)
            && running.is_ok_and(|running| running != 0)
    }

    fn refresh_target(&mut self) -> Result<(), SystemAudioCaptureError> {
        let Some(application) = &self.application else {
            return Ok(());
        };
        let scan_started = Instant::now();
        let processes = application.processes()?;
        let scan_ms = scan_started.elapsed().as_millis();
        if scan_ms >= 20 {
            pipeline_log!("capture tap application_scan slow_ms={scan_ms}");
        }
        if processes == self.process_ids {
            return Ok(());
        }
        pipeline_log!(
            "capture tap application_processes previous_count={} count={} scan_ms={scan_ms}",
            self.process_ids.len(),
            processes.len()
        );
        let update_started = Instant::now();
        let numbers: Vec<_> = processes.iter().map(|id| NSNumber::new_u32(*id)).collect();
        let description = self
            .description
            .as_ref()
            .ok_or(SystemAudioCaptureError::NativeStartFailed)?;
        unsafe {
            description.setProcesses(&NSArray::from_retained_slice(&numbers));
        }
        let mut raw = &**description as *const CATapDescription;
        let mut address = AudioObjectPropertyAddress {
            mSelector: kAudioTapPropertyDescription,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain,
        };
        // Core Audio copies the retained description. An empty inclusion list
        // remains empty; it must never expand to the system mix on a race.
        status(unsafe {
            AudioObjectSetPropertyData(
                self.tap,
                NonNull::from(&mut address),
                0,
                std::ptr::null(),
                std::mem::size_of_val(&raw) as u32,
                NonNull::from(&mut raw).cast(),
            )
        })?;
        pipeline_log!(
            "capture tap application_update duration_ms={}",
            update_started.elapsed().as_millis()
        );
        self.process_ids = processes;
        Ok(())
    }

    fn format_matches(&self, expected: &AudioStreamBasicDescription) -> bool {
        let format: Result<AudioStreamBasicDescription, _> = property(
            self.device,
            kAudioDevicePropertyStreamFormat,
            kAudioObjectPropertyScopeInput,
            None,
        );
        format.is_ok_and(|format| same_format(expected, &format))
    }
}

impl Drop for NativeTap {
    fn drop(&mut self) {
        let mut clean = true;
        if self.started {
            let code = unsafe { AudioDeviceStop(self.device, self.io_proc) };
            if code != 0 {
                pipeline_log!("capture tap stop status={code}");
            }
        }
        if self.io_proc.is_some() {
            let code = unsafe { AudioDeviceDestroyIOProcID(self.device, self.io_proc) };
            if code != 0 {
                pipeline_log!("capture tap destroy_ioproc status={code}");
                // A failed removal is not proof callbacks ceased. Preserve the
                // context and teardown gate rather than risking use-after-free
                // or opening another source alongside an uncertain old stream.
                if let Some(context) = self.context.take() {
                    std::mem::forget(context);
                }
                clean = false;
            }
        }
        if self.device != 0 {
            let code = unsafe { AudioHardwareDestroyAggregateDevice(self.device) };
            if code != 0 {
                pipeline_log!("capture tap destroy_aggregate status={code}");
                clean = false;
            }
        }
        if self.tap != 0 {
            let code = unsafe { (self.api.destroy)(self.tap) };
            if code != 0 {
                pipeline_log!("capture tap destroy status={code}");
                clean = false;
            }
        }
        if !clean {
            if let Some(teardown) = self.teardown.take() {
                std::mem::forget(teardown);
            }
        }
    }
}

fn dictionary(entries: &[(&CStr, &AnyObject)]) -> Retained<NSDictionary<NSString, AnyObject>> {
    let keys: Vec<_> = entries
        .iter()
        .map(|(key, _)| NSString::from_str(key.to_str().expect("Core Audio dictionary key")))
        .collect();
    let keys: Vec<_> = keys.iter().map(|key| &**key).collect();
    let values: Vec<_> = entries.iter().map(|(_, value)| *value).collect();
    NSDictionary::from_slices(&keys, &values)
}

pub(super) fn property<T: Copy>(
    object: AudioObjectID,
    selector: AudioObjectPropertySelector,
    scope: AudioObjectPropertyScope,
    qualifier: Option<&libc::pid_t>,
) -> Result<T, SystemAudioCaptureError> {
    let mut address = AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: scope,
        mElement: kAudioObjectPropertyElementMain,
    };
    let mut value = std::mem::MaybeUninit::<T>::uninit();
    let mut size = std::mem::size_of::<T>() as u32;
    // SAFETY: All call sites use Core Audio's POD property types. The mutable
    // output and size live through the call; a PID qualifier is read-only.
    status(unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&mut address),
            qualifier.map_or(0, |_| std::mem::size_of::<libc::pid_t>() as u32),
            qualifier.map_or(std::ptr::null(), |pid| (pid as *const libc::pid_t).cast()),
            NonNull::from(&mut size),
            NonNull::new(value.as_mut_ptr()).unwrap().cast(),
        )
    })?;
    if size as usize != std::mem::size_of::<T>() {
        return Err(SystemAudioCaptureError::UnsupportedAudioFormat);
    }
    // SAFETY: Success with exactly size_of::<T>() initialized the complete
    // POD output. Call sites request only integer or ASBD properties.
    Ok(unsafe { value.assume_init() })
}

pub(super) fn status(code: i32) -> Result<(), SystemAudioCaptureError> {
    if code == 0 {
        Ok(())
    } else if code == kAudioDevicePermissionsError {
        Err(SystemAudioCaptureError::PermissionDenied)
    } else {
        pipeline_log!("capture tap setup status={code}");
        Err(SystemAudioCaptureError::NativeStartFailed)
    }
}

fn validate_format(format: &AudioStreamBasicDescription) -> Result<(), SystemAudioCaptureError> {
    let supported_sample = (format.mFormatFlags & kAudioFormatFlagIsFloat != 0
        && format.mBitsPerChannel == 32)
        || (format.mFormatFlags & kAudioFormatFlagIsSignedInteger != 0
            && matches!(format.mBitsPerChannel, 16 | 32));
    if format.mFormatID != kAudioFormatLinearPCM
        || format.mChannelsPerFrame != 1
        || !supported_sample
        || !(8_000.0..=192_000.0).contains(&format.mSampleRate)
        || format.mSampleRate.fract() != 0.0
        || format.mBytesPerFrame != format.mBitsPerChannel / 8
        || format.mFramesPerPacket != 1
        || format.mBytesPerPacket != format.mBytesPerFrame
        || format.mFormatFlags & (kAudioFormatFlagIsBigEndian | kAudioFormatFlagIsAlignedHigh) != 0
    {
        return Err(SystemAudioCaptureError::UnsupportedAudioFormat);
    }
    Ok(())
}

fn same_format(left: &AudioStreamBasicDescription, right: &AudioStreamBasicDescription) -> bool {
    left.mSampleRate == right.mSampleRate
        && left.mFormatID == right.mFormatID
        && left.mFormatFlags == right.mFormatFlags
        && left.mBytesPerFrame == right.mBytesPerFrame
        && left.mChannelsPerFrame == right.mChannelsPerFrame
        && left.mBitsPerChannel == right.mBitsPerChannel
        && left.mBytesPerPacket == right.mBytesPerPacket
        && left.mFramesPerPacket == right.mFramesPerPacket
}

#[cfg(test)]
#[path = "tap/tests.rs"]
mod tests;
