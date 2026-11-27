use super::applications::{picker_application, picker_icon_png};
use super::*;
use crate::audio::applications::MAX_APPLICATION_ICON_PNG_BYTES;
use objc2_core_audio_types::{
    kAudioFormatFlagIsFloat, kAudioFormatFlagIsNonInterleaved, kAudioFormatFlagIsSignedInteger,
    AudioStreamBasicDescription,
};
use rubato::audioadapter::Adapter;
use rubato::audioadapter_buffers::direct::SequentialSliceOfVecs;
use rubato::{FixedSync, Resampler};

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

fn peak(data: &[f32]) -> f32 {
    data.iter().fold(0.0f32, |acc, s| acc.max(s.abs()))
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
        mFormatFlags: kAudioFormatFlagIsFloat | objc2_core_audio_types::kAudioFormatFlagIsPacked,
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
        objc2_foundation::NSData::with_bytes(include_bytes!("../../../icons/128x128@2x.png"));
    let image = NSImage::initWithData(NSImage::alloc(), &data).unwrap();
    let previous = NSGraphicsContext::currentContext();
    let png = picker_icon_png(&image).unwrap();
    assert!(png.len() <= MAX_APPLICATION_ICON_PNG_BYTES);
    let decoded = tauri::image::Image::from_bytes(&png).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (64, 64));
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
        let error =
            unsafe { objc2_foundation::NSError::errorWithDomain_code_userInfo(domain, code, None) };
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
    finish_failed_capture_start(
        &generation,
        &started,
        &Mutex::new(()),
        timed_out_token,
        async move {
            let mut slot = teardown_slot.lock().unwrap();
            if *slot == Some(timed_out_token) {
                *slot = None;
            }
        },
    )
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
async fn stale_start_failure_preserves_a_new_active_generation() {
    let generation = CaptureGeneration::default();
    let started = AtomicBool::new(true);
    let transition = Mutex::new(());
    let old = generation.begin();
    generation.invalidate();
    let current = generation.begin();
    finish_failed_capture_start(&generation, &started, &transition, old, async {}).await;
    assert!(started.load(Ordering::SeqCst));
    assert!(generation.is_current(current));
    assert!(!generation.is_current(old));
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
        tokio::time::timeout(Duration::from_millis(10), pending.wait_until_clear())
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

fn sine_samples(sample_rate_hz: f64, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| {
            (i as f64 * 2.0 * std::f64::consts::PI * 1000.0 / sample_rate_hz).sin() as f32 * 0.5
        })
        .collect()
}

fn pcm16_peak(pcm: &[u8]) -> i16 {
    pcm.as_chunks::<2>()
        .0
        .iter()
        .map(|chunk| i16::from_le_bytes(*chunk).saturating_abs())
        .max()
        .unwrap_or(0)
}

#[test]
fn resample_to_pcm16_rebuilds_and_clears_pending_across_sample_rate_changes() {
    let resampler = Mutex::new(None);
    let pending_frames = Mutex::new(Vec::new());
    let target_rate = 16_000;

    // 1. Start at 48 kHz with a partial chunk (< 1024 frames) left in pending_frames.
    let partial_48k = sine_samples(48_000.0, 512);
    assert_eq!(
        decode::resample_to_pcm16(
            &resampler,
            &pending_frames,
            target_rate,
            &partial_48k,
            48_000.0,
        )
        .unwrap(),
        None
    );
    assert_eq!(pending_frames.lock().unwrap().len(), 512);
    assert_eq!(
        resampler.lock().unwrap().as_ref().map(|(rate, _)| *rate),
        Some(48_000)
    );

    // 2. Switch to 44.1 kHz: old 48 kHz pending samples must be discarded and resampler rebuilt.
    // Feed two full chunks so output clears any initial FFT warmup delay.
    let chunk_44k = sine_samples(44_100.0, 2048);
    let out_44k = decode::resample_to_pcm16(
        &resampler,
        &pending_frames,
        target_rate,
        &chunk_44k,
        44_100.0,
    )
    .unwrap()
    .expect("44.1 kHz chunks should emit resampled PCM");
    assert_eq!(pending_frames.lock().unwrap().len(), 0);
    assert_eq!(
        resampler.lock().unwrap().as_ref().map(|(rate, _)| *rate),
        Some(44_100)
    );
    let frames_44k = out_44k.len() / 2;
    assert!(pcm16_peak(&out_44k) > 100);

    // Leave partial 44.1 kHz frames buffered before switching to pass-through 16 kHz.
    let partial_44k = sine_samples(44_100.0, 300);
    assert_eq!(
        decode::resample_to_pcm16(
            &resampler,
            &pending_frames,
            target_rate,
            &partial_44k,
            44_100.0,
        )
        .unwrap(),
        None
    );
    assert_eq!(pending_frames.lock().unwrap().len(), 300);

    // 3. Switch to 16 kHz pass-through: pending_frames cleared and resampler reset to None.
    let direct_16k = sine_samples(16_000.0, 320);
    let out_16k = decode::resample_to_pcm16(
        &resampler,
        &pending_frames,
        target_rate,
        &direct_16k,
        16_000.0,
    )
    .unwrap()
    .expect("16 kHz pass-through should emit PCM directly");
    assert_eq!(out_16k.len(), 320 * 2);
    assert!(pending_frames.lock().unwrap().is_empty());
    assert!(resampler.lock().unwrap().is_none());
    assert!(pcm16_peak(&out_16k) > 100);

    // 4. Switch back to 48 kHz: resampler must be created fresh for 48 kHz.
    let chunk_48k = sine_samples(48_000.0, 2048);
    let out_48k = decode::resample_to_pcm16(
        &resampler,
        &pending_frames,
        target_rate,
        &chunk_48k,
        48_000.0,
    )
    .unwrap()
    .expect("48 kHz chunks should emit resampled PCM");
    assert_eq!(pending_frames.lock().unwrap().len(), 0);
    assert_eq!(
        resampler.lock().unwrap().as_ref().map(|(rate, _)| *rate),
        Some(48_000)
    );
    let frames_48k = out_48k.len() / 2;
    assert!(pcm16_peak(&out_48k) > 100);
    // For the same 2048 input frames, 44.1 kHz -> 16 kHz must yield more output
    // frames than 48 kHz -> 16 kHz, confirming the resampler tracked the rate change.
    assert!(
        frames_44k > frames_48k,
        "expected 44.1kHz->16kHz ({frames_44k}) > 48kHz->16kHz ({frames_48k})"
    );

    // Invalid sample rate is rejected cleanly.
    assert_eq!(
        decode::resample_to_pcm16(
            &resampler,
            &pending_frames,
            target_rate,
            &chunk_48k,
            f64::NAN
        ),
        Err(SystemAudioCaptureError::UnsupportedAudioFormat)
    );
}

#[tokio::test]
async fn stream_handler_owns_audio_state_until_last_objc_reference_drops() {
    use crate::audio::send_pipeline::AudioSendPipeline;

    let pipeline = AudioSendPipeline::spawn(|_| async { Ok::<(), ()>(()) }, |_| {});
    let ingress = pipeline.ingress().expect("ingress available");
    let (failure_tx, mut failure_rx) = CaptureFailureSender::channel();
    let generation = CaptureGeneration::default();
    let generation_token = generation.begin();

    let state = AudioHandlerState {
        audio_ingress: ingress,
        failure_tx,
        generation: generation.clone(),
        generation_token,
        last_audio_buffer_at: Mutex::new(None),
        decoded_buffers: Mutex::new(0),
        resampler: Mutex::new(None),
        pending_frames: Mutex::new(Vec::new()),
        target_sample_rate_hz: 16_000,
    };
    let this = MimiAudioStreamHandler::alloc().set_ivars(MimiAudioStreamHandlerIvars { state });
    let primary: Retained<MimiAudioStreamHandler> = unsafe { msg_send![super(this), init] };
    // Simulate ScreenCaptureKit retaining an extra Objective-C reference on a callback queue.
    let retained_by_callback = primary.clone();

    drop(primary);
    // Dropping the primary owner must not drop AudioHandlerState while another ObjC reference exists.
    assert!(matches!(
        failure_rx.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
    assert!(retained_by_callback
        .ivars()
        .state
        .generation
        .is_current(generation_token));

    // Releasing the final Objective-C reference invokes objc2's synthesized dealloc and drops state.
    drop(retained_by_callback);
    assert!(matches!(
        failure_rx.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Disconnected)
    ));
}
