use super::*;

#[test]
fn callback_rejects_extra_streams_oversized_and_partial_frames() {
    assert!(valid_buffer(1, 4096, 4));
    assert!(!valid_buffer(2, 4096, 4));
    assert!(!valid_buffer(1, MAX_CALLBACK_BYTES + 4, 4));
    assert!(!valid_buffer(1, 4095, 4));
    assert!(!valid_buffer(1, 4096, 0));
}

#[test]
fn taps_cover_both_targets_but_preserve_existing_screen_grants() {
    assert!(use_audio_tap(true, false));
    assert!(!use_audio_tap(false, false));
    assert!(!use_audio_tap(true, true));
    assert!(!use_audio_tap(false, true));
}

#[test]
fn changed_sample_rate_or_layout_requires_reconnect() {
    let format = AudioStreamBasicDescription {
        mSampleRate: 48_000.0,
        mFormatID: kAudioFormatLinearPCM,
        mFormatFlags: kAudioFormatFlagIsFloat,
        mBitsPerChannel: 32,
        mBytesPerFrame: 4,
        mChannelsPerFrame: 1,
        mBytesPerPacket: 4,
        mFramesPerPacket: 1,
        mReserved: 0,
    };
    assert!(validate_format(&format).is_ok());
    assert!(same_format(&format, &format));
    let mut changed = format;
    changed.mSampleRate = 44_100.0;
    assert!(!same_format(&format, &changed));
    changed.mChannelsPerFrame = 2;
    assert!(validate_format(&changed).is_err());
    changed = format;
    changed.mSampleRate = f64::NAN;
    assert!(validate_format(&changed).is_err());
}

fn deliver(state: &mut CallbackState, bytes: &mut [u8]) {
    deliver_with_buffer(state, bytes, |_| {});
}

fn deliver_with_buffer(
    state: &mut CallbackState,
    bytes: &mut [u8],
    configure: impl FnOnce(&mut AudioBufferList),
) {
    use objc2_core_audio_types::AudioBuffer;
    let mut input = AudioBufferList {
        mNumberBuffers: 1,
        mBuffers: [AudioBuffer {
            mNumberChannels: 1,
            mDataByteSize: bytes.len() as u32,
            mData: bytes.as_mut_ptr().cast(),
        }],
    };
    configure(&mut input);
    let mut output = AudioBufferList {
        mNumberBuffers: 0,
        mBuffers: [AudioBuffer {
            mNumberChannels: 0,
            mDataByteSize: 0,
            mData: std::ptr::null_mut(),
        }],
    };
    // SAFETY: Zero is a valid timestamp with no flags set. Every pointer
    // passed to the callback remains valid for this synchronous call.
    let mut timestamp: AudioTimeStamp = unsafe { std::mem::zeroed() };
    unsafe {
        audio_callback(
            0,
            NonNull::from(&mut timestamp),
            NonNull::from(&mut input),
            NonNull::from(&mut timestamp),
            NonNull::from(&mut output),
            NonNull::from(&mut timestamp),
            (state as *mut CallbackState).cast(),
        );
    }
}

#[test]
fn callback_ignores_empty_disabled_input_without_reading_or_failing() {
    let (sender, receiver) = mpsc::sync_channel(CALLBACK_QUEUE_CAPACITY);
    let (failure, mut failures) = CaptureFailureSender::channel();
    let mut state = CallbackState {
        sender,
        queued_bytes: Arc::new(AtomicUsize::new(0)),
        byte_limit: 48_000 * 4,
        failure,
        generation: CaptureGeneration::default(),
        token: 0,
        bytes_per_frame: 4,
    };
    let mut bytes = [1, 2, 3, 4];
    deliver_with_buffer(&mut state, &mut bytes, |input| input.mNumberBuffers = 0);
    deliver_with_buffer(&mut state, &mut bytes, |input| {
        input.mBuffers[0].mData = std::ptr::null_mut();
    });
    deliver(&mut state, &mut []);
    assert!(receiver.try_recv().is_err());
    assert!(failures.try_recv().is_err());
}

#[test]
fn malformed_native_callback_fails_closed_and_reports_only_once() {
    for invalid in 0..4 {
        let (sender, receiver) = mpsc::sync_channel(CALLBACK_QUEUE_CAPACITY);
        let (failure, mut failures) = CaptureFailureSender::channel();
        let mut state = CallbackState {
            sender,
            queued_bytes: Arc::new(AtomicUsize::new(0)),
            byte_limit: 48_000 * 4,
            failure,
            generation: CaptureGeneration::default(),
            token: 0,
            bytes_per_frame: 4,
        };
        let mut bytes = [1, 2, 3, 4];
        deliver_with_buffer(&mut state, &mut bytes, |input| match invalid {
            0 => input.mNumberBuffers = 2,
            1 => input.mBuffers[0].mNumberChannels = 2,
            2 => input.mBuffers[0].mDataByteSize = MAX_CALLBACK_BYTES as u32 + 4,
            _ => input.mBuffers[0].mDataByteSize = 3,
        });
        deliver(&mut state, &mut bytes);
        assert!(receiver.try_recv().is_err());
        assert_eq!(
            failures.try_recv().unwrap(),
            SystemAudioCaptureFailure::AudioProcessingFailed
        );
        assert!(failures.try_recv().is_err());
    }
}

#[test]
fn native_permission_error_is_distinct_from_other_setup_failures() {
    assert_eq!(status(0), Ok(()));
    assert_eq!(
        status(kAudioDevicePermissionsError),
        Err(SystemAudioCaptureError::PermissionDenied)
    );
    assert_eq!(status(-1), Err(SystemAudioCaptureError::NativeStartFailed));
}

#[test]
fn supported_tap_formats_decode_and_resample_at_both_provider_rates() {
    for (bits, flags) in [
        (32, kAudioFormatFlagIsFloat),
        (16, kAudioFormatFlagIsSignedInteger),
        (32, kAudioFormatFlagIsSignedInteger),
    ] {
        let asbd = AudioStreamBasicDescription {
            mSampleRate: 48_000.0,
            mFormatID: kAudioFormatLinearPCM,
            mFormatFlags: flags,
            mBitsPerChannel: bits,
            mBytesPerFrame: bits / 8,
            mChannelsPerFrame: 1,
            mBytesPerPacket: bits / 8,
            mFramesPerPacket: 1,
            mReserved: 0,
        };
        validate_format(&asbd).unwrap();
        let mut bytes = Vec::new();
        for frame in 0..4096 {
            let sample = (frame as f32 * std::f32::consts::TAU * 440.0 / 48_000.0).sin() * 0.5;
            if flags == kAudioFormatFlagIsFloat {
                bytes.extend_from_slice(&sample.to_le_bytes());
            } else if bits == 16 {
                bytes.extend_from_slice(&((sample * i16::MAX as f32) as i16).to_le_bytes());
            } else {
                bytes.extend_from_slice(&((sample * i32::MAX as f32) as i32).to_le_bytes());
            }
        }
        let samples = decode_to_f32_mono(&bytes, &asbd).unwrap();
        assert_eq!(samples.len(), 4096);
        for rate in [16_000, 24_000] {
            let mut resampler = StreamingPcm16Resampler::new(48_000, rate, 1).unwrap();
            let buffers = resampler.push_interleaved(&samples).unwrap();
            assert!(!buffers.is_empty());
            assert!(buffers
                .iter()
                .all(|pcm| !pcm.is_empty() && pcm.len().is_multiple_of(2)));
            assert!(buffers
                .iter()
                .flat_map(|pcm| pcm.as_chunks::<2>().0.iter())
                .any(|bytes| i32::from(i16::from_le_bytes(*bytes)).abs() > 100));
        }
        for rate in [0.0, -48_000.0, f64::NAN, f64::INFINITY, 48_000.5, 192_001.0] {
            assert!(validate_format(&AudioStreamBasicDescription {
                mSampleRate: rate,
                ..asbd
            })
            .is_err());
        }
        for flags in [
            asbd.mFormatFlags | kAudioFormatFlagIsBigEndian,
            asbd.mFormatFlags | kAudioFormatFlagIsAlignedHigh,
        ] {
            assert!(validate_format(&AudioStreamBasicDescription {
                mFormatFlags: flags,
                ..asbd
            })
            .is_err());
        }
        assert!(validate_format(&AudioStreamBasicDescription {
            mBytesPerPacket: 1,
            ..asbd
        })
        .is_err());
        assert!(validate_format(&AudioStreamBasicDescription {
            mFramesPerPacket: 2,
            ..asbd
        })
        .is_err());
    }
}

#[test]
fn callback_preserves_a_short_native_burst_until_worker_resumes() {
    let (sender, receiver) = mpsc::sync_channel(CALLBACK_QUEUE_CAPACITY);
    let (failure, mut failures) = CaptureFailureSender::channel();
    let mut state = CallbackState {
        sender,
        queued_bytes: Arc::new(AtomicUsize::new(0)),
        byte_limit: 48_000 * 4,
        failure,
        generation: CaptureGeneration::default(),
        token: 0,
        bytes_per_frame: 4,
    };
    // Six 1024-frame callbacks at 48 kHz: a 128 ms scheduling pause.
    for sequence in 0..6 {
        deliver(&mut state, &mut vec![sequence; 4096]);
    }
    assert!(failures.try_recv().is_err());
    assert_eq!(state.queued_bytes.load(Ordering::Relaxed), 6 * 4096);
    let packets: Vec<_> = receiver.try_iter().collect();
    assert_eq!(packets.len(), 6);
    for (sequence, packet) in packets.iter().enumerate() {
        assert!(packet.bytes.iter().all(|byte| *byte == sequence as u8));
    }
    drop(packets);
    assert_eq!(state.queued_bytes.load(Ordering::Relaxed), 0);
}

#[test]
fn callback_byte_budget_bounds_pcm_and_releases_received_and_disconnected_packets() {
    for (rate, frame_bytes) in [(8_000, 2), (48_000, 4), (192_000, 4)] {
        let (sender, receiver) = mpsc::sync_channel(CALLBACK_QUEUE_CAPACITY);
        let (failure, mut failures) = CaptureFailureSender::channel();
        let mut state = CallbackState {
            sender,
            queued_bytes: Arc::new(AtomicUsize::new(0)),
            byte_limit: rate * frame_bytes,
            failure,
            generation: CaptureGeneration::default(),
            token: 0,
            bytes_per_frame: frame_bytes,
        };
        let mut bytes = vec![0; 4096];
        deliver(&mut state, &mut bytes);
        let processing = receiver.try_recv().unwrap();
        assert_eq!(state.queued_bytes.load(Ordering::Relaxed), 4096);
        drop(processing);
        assert_eq!(state.queued_bytes.load(Ordering::Relaxed), 0);
        let capacity = state.byte_limit / bytes.len();
        for _ in 0..capacity + 2 {
            deliver(&mut state, &mut bytes);
        }
        assert_eq!(state.queued_bytes.load(Ordering::Relaxed), capacity * 4096);
        assert_eq!(
            failures.try_recv().unwrap(),
            SystemAudioCaptureFailure::Backpressure
        );
        assert!(failures.try_recv().is_err());
        drop(receiver);
        assert_eq!(state.queued_bytes.load(Ordering::Relaxed), 0);
    }
    let (sender, receiver) = mpsc::sync_channel(CALLBACK_QUEUE_CAPACITY);
    drop(receiver);
    let (failure, mut failures) = CaptureFailureSender::channel();
    let mut state = CallbackState {
        sender,
        queued_bytes: Arc::new(AtomicUsize::new(0)),
        byte_limit: 48_000 * 4,
        failure,
        generation: CaptureGeneration::default(),
        token: 0,
        bytes_per_frame: 4,
    };
    deliver(&mut state, &mut [0; 4096]);
    assert_eq!(state.queued_bytes.load(Ordering::Relaxed), 0);
    assert!(failures.try_recv().is_err());
}

#[test]
fn callback_owns_queued_bytes_discards_stale_generation_and_reports_overflow_once() {
    let (sender, receiver) = mpsc::sync_channel(CALLBACK_QUEUE_CAPACITY);
    let (failure, mut failures) = CaptureFailureSender::channel();
    let mut state = CallbackState {
        sender,
        queued_bytes: Arc::new(AtomicUsize::new(0)),
        byte_limit: 48_000 * 4,
        failure,
        generation: CaptureGeneration::default(),
        token: 1,
        bytes_per_frame: 4,
    };
    let mut bytes = vec![1, 2, 3, 4];
    deliver(&mut state, &mut bytes);
    assert!(receiver.try_recv().is_err());
    state.token = 0;
    deliver(&mut state, &mut bytes);
    bytes.fill(0);
    assert_eq!(receiver.try_recv().unwrap().bytes, [1, 2, 3, 4]);
    for _ in 0..CALLBACK_QUEUE_CAPACITY + 2 {
        deliver(&mut state, &mut bytes);
    }
    assert_eq!(receiver.try_iter().count(), CALLBACK_QUEUE_CAPACITY);
    assert_eq!(
        failures.try_recv().unwrap(),
        SystemAudioCaptureFailure::Backpressure
    );
    assert!(failures.try_recv().is_err());
}

#[test]
fn native_resource_cleanup_controls_the_source_exclusivity_gate() {
    use super::super::macos::PendingTeardown;
    unsafe extern "C-unwind" fn create(_: Option<&CATapDescription>, _: *mut AudioObjectID) -> i32 {
        0
    }
    unsafe extern "C-unwind" fn destroyed(_: AudioObjectID) -> i32 {
        0
    }
    unsafe extern "C-unwind" fn uncertain(_: AudioObjectID) -> i32 {
        -1
    }
    let pending = PendingTeardown::default();
    let mut native = NativeTap::new(
        TapApi {
            create,
            destroy: destroyed,
        },
        pending.begin(),
    );
    native.tap = 42;
    assert!(pending.is_pending());
    drop(native);
    assert!(!pending.is_pending());
    let mut native = NativeTap::new(
        TapApi {
            create,
            destroy: uncertain,
        },
        pending.begin(),
    );
    native.tap = 42;
    drop(native);
    assert!(
        pending.is_pending(),
        "uncertain native cleanup must keep the next source closed"
    );
}
