use super::*;
use crate::audio::send_pipeline::AudioSendPipeline;

#[derive(Default)]
struct Calls {
    instances: AtomicUsize,
    render: Mutex<Vec<Vec<i16>>>,
    capture: Mutex<Vec<Vec<i16>>>,
}

struct TestProcessor {
    calls: Arc<Calls>,
    silence: bool,
}

impl EchoProcessor for TestProcessor {
    fn process(&mut self, render: &[i16], capture: &[i16]) -> Result<Vec<i16>, ()> {
        self.calls.render.lock().unwrap().push(render.to_vec());
        self.calls.capture.lock().unwrap().push(capture.to_vec());
        Ok(if self.silence {
            vec![0; capture.len()]
        } else {
            capture.to_vec()
        })
    }
}

fn echo(rate: u32, silence: bool) -> (EchoPipeline, Arc<Calls>) {
    let calls = Arc::new(Calls::default());
    let factory_calls = Arc::clone(&calls);
    let (failure, _) = CaptureFailureSender::channel();
    let pipeline = EchoPipeline::spawn_with_factory(
        rate,
        failure,
        Arc::new(move || {
            factory_calls.instances.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(TestProcessor {
                calls: Arc::clone(&factory_calls),
                silence,
            }))
        }),
    )
    .unwrap();
    (pipeline, calls)
}

fn collect() -> (AudioSendPipeline, Arc<Mutex<Vec<u8>>>) {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let output = Arc::clone(&bytes);
    let pipeline = AudioSendPipeline::spawn(
        move |data| {
            output.lock().unwrap().extend(data);
            async { Ok::<_, ()>(()) }
        },
        |_| {},
    );
    (pipeline, bytes)
}

async fn until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while !condition() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn echo_pipeline_keeps_arbitrary_chunks_and_final_partial_frame_at_both_rates() {
    for rate in [16_000, 24_000] {
        let (echo, calls) = echo(rate, false);
        let (network, received) = collect();
        let direct = network.ingress().unwrap();
        let microphone = echo.ingress(AudioSource::Microphone, direct.clone());
        let input: Vec<i16> = (0..1003).map(|sample| sample as i16 - 500).collect();
        let mut offset = 0;
        for size in [1, 159, 7, 201, 635] {
            microphone
                .try_send(encode(&input[offset..offset + size]))
                .unwrap();
            offset += size;
        }
        assert!(echo.finish(Duration::from_secs(1)).await);
        // A retained native callback cannot close the provider queue while
        // the last accepted DSP buffers are still draining.
        assert_eq!(
            microphone.try_send(vec![0, 0]),
            Err(AudioIngressError::Closed)
        );
        assert!(network.finish(Duration::from_secs(1)).await);
        assert_eq!(*received.lock().unwrap(), encode(&input));
        assert!(!network.pending_pcm_gate().has_pending());
        assert!(calls
            .capture
            .lock()
            .unwrap()
            .iter()
            .all(|frame| frame.len() == rate as usize / 100));
    }
}

#[tokio::test]
async fn echo_pipeline_observes_raw_microphone_even_when_processed_output_is_silent() {
    let (echo, _) = echo(16_000, true);
    let (system_network, system_bytes) = collect();
    let (mic_network, mic_bytes) = collect();
    let system = echo.ingress(AudioSource::System, system_network.ingress().unwrap());
    let mic = echo.ingress(AudioSource::Microphone, mic_network.ingress().unwrap());
    let input = encode(&vec![4_000; 160]);
    system.try_send(input.clone()).unwrap();
    mic.try_send(input.clone()).unwrap();
    assert!(echo.finish(Duration::from_secs(1)).await);
    assert!(system_network.finish(Duration::from_secs(1)).await);
    assert!(mic_network.finish(Duration::from_secs(1)).await);
    assert_eq!(*system_bytes.lock().unwrap(), input);
    assert_eq!(*mic_bytes.lock().unwrap(), vec![0; 320]);
    assert_eq!(mic_network.input_activity(), (true, true));
    assert_eq!(system_network.input_activity(), (true, true));
}

fn work_at(source: AudioSource, samples: &[i16], at: Instant, output: &AudioIngress) -> Work {
    let (failure, _) = CaptureFailureSender::channel();
    let shared = Arc::new(Shared {
        accepting: AtomicBool::new(true),
        queued_bytes: Default::default(),
        byte_limit: usize::MAX,
        failure,
    });
    Work {
        source,
        packet: PendingAudio {
            data: encode(samples),
            pending: Some(output.acquire_pending()),
            captured_at: at,
        },
        output: output.clone(),
        _bytes: shared.reserve(source, samples.len() * 2).unwrap(),
    }
}

fn mock_state(rate: usize) -> (ProcessorState, Arc<Calls>) {
    let calls = Arc::new(Calls::default());
    let factory_calls = Arc::clone(&calls);
    let factory: ProcessorFactory = Arc::new(move || {
        factory_calls.instances.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(TestProcessor {
            calls: Arc::clone(&factory_calls),
            silence: false,
        }))
    });
    (
        ProcessorState::new(rate / 100, factory().unwrap(), factory),
        calls,
    )
}

#[tokio::test]
async fn echo_pipeline_pairs_recent_reference_after_startup_and_obeys_capture_deadline() {
    let (mut state, calls) = mock_state(16_000);
    let (network, received) = collect();
    let output = network.ingress().unwrap();
    let start = Instant::now();
    for index in 1..=200_u64 {
        state
            .push(work_at(
                AudioSource::System,
                &vec![index as i16; 160],
                start + Duration::from_millis(index * 10),
                &output,
            ))
            .unwrap();
    }
    assert_eq!(state.reference.len(), REFERENCE_FRAMES);
    assert!(calls.render.lock().unwrap().is_empty());
    let mic_at = start + Duration::from_secs(2);
    state
        .push(work_at(
            AudioSource::Microphone,
            &vec![2_000; 160],
            mic_at,
            &output,
        ))
        .unwrap();
    // The first microphone frame aligns to current system time + the
    // bounded lookahead; no startup sample-index zero pairing is possible.
    state
        .push(work_at(
            AudioSource::System,
            &vec![999; 160],
            mic_at + REFERENCE_LOOKAHEAD,
            &output,
        ))
        .unwrap();
    state
        .process_ready(
            mic_at + CAPTURE_DELAY + OUTPUT_BATCH_WAIT - Duration::from_millis(1),
            false,
        )
        .unwrap();
    assert!(calls.capture.lock().unwrap().is_empty());
    assert!(network.pending_pcm_gate().has_pending());
    state
        .process_ready(mic_at + CAPTURE_DELAY + OUTPUT_BATCH_WAIT, false)
        .unwrap();
    assert_eq!(*calls.render.lock().unwrap(), vec![vec![999; 160]]);
    assert!(network.finish(Duration::from_secs(1)).await);
    assert_eq!(*received.lock().unwrap(), encode(&vec![2_000; 160]));
    assert!(!network.pending_pcm_gate().has_pending());
}

#[tokio::test]
async fn echo_pipeline_long_callbacks_are_continuous_and_true_gaps_reset_after_older_frames() {
    let (mut state, calls) = mock_state(16_000);
    let (network, received) = collect();
    let output = network.ingress().unwrap();
    let start = Instant::now();
    for index in 1..=3_u64 {
        state
            .push(work_at(
                AudioSource::Microphone,
                &vec![index as i16; 1600],
                start + Duration::from_millis(index * 100),
                &output,
            ))
            .unwrap();
    }
    state
        .process_ready(start + Duration::from_secs(1), false)
        .unwrap();
    assert_eq!(calls.instances.load(Ordering::SeqCst), 1);
    state
        .push(work_at(
            AudioSource::Microphone,
            &[77],
            start + Duration::from_secs(2),
            &output,
        ))
        .unwrap();
    state.finish().unwrap();
    assert_eq!(calls.instances.load(Ordering::SeqCst), 2);
    assert!(network.finish(Duration::from_secs(1)).await);
    assert_eq!(received.lock().unwrap().len(), (3 * 1600 + 1) * 2);
    assert!(!network.pending_pcm_gate().has_pending());
}

#[tokio::test]
async fn echo_pipeline_render_gap_discards_partial_reference_and_missing_reference_resets_tail() {
    let (mut state, calls) = mock_state(16_000);
    let (network, _) = collect();
    let output = network.ingress().unwrap();
    let start = Instant::now() + Duration::from_secs(1);
    state
        .push(work_at(
            AudioSource::System,
            &vec![111; 160],
            start,
            &output,
        ))
        .unwrap();
    state
        .push(work_at(
            AudioSource::Microphone,
            &vec![900; 160],
            start - REFERENCE_LOOKAHEAD,
            &output,
        ))
        .unwrap();
    state.process_ready(start + CAPTURE_DELAY, false).unwrap();
    assert_eq!(state.applied_render_epoch, Some(0));
    state
        .push(work_at(
            AudioSource::System,
            &[222; 80],
            start + Duration::from_millis(5),
            &output,
        ))
        .unwrap();
    state
        .push(work_at(
            AudioSource::Microphone,
            &vec![900; 160],
            start - REFERENCE_LOOKAHEAD + FRAME_DURATION,
            &output,
        ))
        .unwrap();
    state
        .process_ready(start + CAPTURE_DELAY + FRAME_DURATION, false)
        .unwrap();
    assert_eq!(calls.instances.load(Ordering::SeqCst), 2);
    assert_eq!(calls.render.lock().unwrap().last().unwrap(), &vec![0; 160]);
    // A long source gap must not combine 222 from the old half frame with
    // the resumed reference. The retained complete old frames stay tagged.
    state
        .push(work_at(
            AudioSource::System,
            &[333; 80],
            start + Duration::from_secs(1),
            &output,
        ))
        .unwrap();
    assert_eq!(state.render.pending.len(), 80);
    state
        .push(work_at(
            AudioSource::System,
            &[444; 80],
            start + Duration::from_millis(1005),
            &output,
        ))
        .unwrap();
    let resumed = state.reference.back().unwrap();
    assert_eq!(resumed.epoch, 1);
    assert_eq!(resumed.samples, [vec![333; 80], vec![444; 80]].concat());
    assert!(network.finish(Duration::from_secs(1)).await);
}

#[tokio::test]
async fn echo_pipeline_incomplete_capture_is_pending_until_finish_or_cancellation() {
    let (echo, _) = echo(16_000, false);
    let (network, received) = collect();
    let gate = network.pending_pcm_gate();
    let mic = echo.ingress(AudioSource::Microphone, network.ingress().unwrap());
    mic.try_send(encode(&[4_000])).unwrap();
    until(|| echo.shared.queued_bytes[1].load(Ordering::SeqCst) == 0).await;
    assert!(gate.has_pending());
    assert!(received.lock().unwrap().is_empty());
    echo.stop();
    until(|| !gate.has_pending()).await;
    assert_eq!(mic.try_send(encode(&[1])), Err(AudioIngressError::Closed));
    assert!(received.lock().unwrap().is_empty());
}

#[tokio::test]
async fn echo_pipeline_stalled_system_transport_does_not_block_mic_processing() {
    let (echo, calls) = echo(16_000, false);
    let (release, wait) = oneshot::channel();
    let wait = Arc::new(Mutex::new(Some(wait)));
    let system_network = AudioSendPipeline::spawn(
        move |_| {
            let wait = wait.lock().unwrap().take();
            async move {
                if let Some(wait) = wait {
                    let _ = wait.await;
                }
                Ok::<_, ()>(())
            }
        },
        |_| {},
    );
    let (mic_network, mic_bytes) = collect();
    let system = echo.ingress(AudioSource::System, system_network.ingress().unwrap());
    let mic = echo.ingress(AudioSource::Microphone, mic_network.ingress().unwrap());
    system.try_send(encode(&vec![1_000; 160])).unwrap();
    mic.try_send(encode(&vec![2_000; 160])).unwrap();
    until(|| mic_bytes.lock().unwrap().len() == 320).await;
    assert_eq!(calls.render.lock().unwrap().len(), 1);
    assert!(system_network.pending_pcm_gate().has_pending());
    assert!(!mic_network.pending_pcm_gate().has_pending());
    release.send(()).unwrap();
    assert!(echo.finish(Duration::from_secs(1)).await);
    assert!(system_network.finish(Duration::from_secs(1)).await);
    assert!(mic_network.finish(Duration::from_secs(1)).await);
}

#[tokio::test]
async fn dual_input_absorbs_same_short_send_stall_as_microphone_only() {
    for dual in [false, true] {
        let sends = Arc::new(AtomicUsize::new(0));
        let sent = Arc::clone(&sends);
        let received = Arc::new(Mutex::new(Vec::new()));
        let output = Arc::clone(&received);
        let network = AudioSendPipeline::spawn(
            move |pcm| {
                let first = sent.fetch_add(1, Ordering::SeqCst) == 0;
                let output = Arc::clone(&output);
                async move {
                    if first {
                        tokio::time::sleep(Duration::from_millis(300)).await;
                    }
                    output.lock().unwrap().extend(pcm);
                    Ok::<_, ()>(())
                }
            },
            |_| {},
        );
        let (echo, _) = echo(16_000, false);
        let ingress = if dual {
            echo.ingress(AudioSource::Microphone, network.ingress().unwrap())
        } else {
            network.ingress().unwrap()
        };
        let mut tick = tokio::time::interval(Duration::from_millis(20));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut accepted = Vec::new();
        let mut error = None;
        for block in 0..50_i16 {
            tick.tick().await;
            let pcm = encode(&vec![block; 320]);
            if let Err(failure) = ingress.try_send(pcm.clone()) {
                error = Some(failure);
                break;
            }
            accepted.extend(pcm);
        }
        let echo_drained = echo.finish(Duration::from_secs(1)).await;
        let network_drained = network.finish(Duration::from_secs(1)).await;
        assert!(error.is_none(), "dual={dual} ingress={error:?}");
        assert!(echo_drained, "dual={dual} echo drain");
        assert!(network_drained, "dual={dual} network drain");
        assert_eq!(
            *received.lock().unwrap(),
            accepted,
            "dual={dual} PCM sequence"
        );
    }
}

#[tokio::test]
async fn echo_pipeline_rejects_oversize_and_malformed_audio_before_copying() {
    for malformed in [false, true] {
        let (failure, mut failures) = CaptureFailureSender::channel();
        let calls = Arc::new(Calls::default());
        let echo = EchoPipeline::spawn_with_factory(
            16_000,
            failure,
            Arc::new(move || {
                Ok(Box::new(TestProcessor {
                    calls: Arc::clone(&calls),
                    silence: false,
                }))
            }),
        )
        .unwrap();
        let (network, received) = collect();
        let source = echo.ingress(AudioSource::System, network.ingress().unwrap());
        let data = if malformed {
            vec![1]
        } else {
            vec![0; echo.shared.byte_limit + 2]
        };
        assert!(source.try_send(data).is_err());
        assert_eq!(
            failures.recv().await,
            Some(if malformed {
                SystemAudioCaptureFailure::AudioProcessingFailed
            } else {
                SystemAudioCaptureFailure::Backpressure
            })
        );
        assert_eq!(source.try_send(vec![0, 0]), Err(AudioIngressError::Closed));
        assert!(received.lock().unwrap().is_empty());
        assert_eq!(echo.shared.queued_bytes[0].load(Ordering::SeqCst), 0);
        assert!(!network.pending_pcm_gate().has_pending());
    }
}

#[tokio::test]
async fn echo_pipeline_retired_ingress_cannot_feed_a_restarted_attempt() {
    let (old, _) = echo(16_000, false);
    let (old_network, old_bytes) = collect();
    let old_mic = old.ingress(AudioSource::Microphone, old_network.ingress().unwrap());
    old_mic.try_send(encode(&[999])).unwrap();
    old.stop();
    let (new, new_calls) = echo(16_000, false);
    let (new_network, new_bytes) = collect();
    let new_mic = new.ingress(AudioSource::Microphone, new_network.ingress().unwrap());
    assert_eq!(
        old_mic.try_send(encode(&vec![999; 160])),
        Err(AudioIngressError::Closed)
    );
    new_mic.try_send(encode(&vec![123; 160])).unwrap();
    assert!(new.finish(Duration::from_secs(1)).await);
    assert!(new_network.finish(Duration::from_secs(1)).await);
    assert_eq!(*new_bytes.lock().unwrap(), encode(&vec![123; 160]));
    assert_eq!(*new_calls.capture.lock().unwrap(), vec![vec![123; 160]]);
    assert!(old_bytes.lock().unwrap().is_empty());
    until(|| !old_network.pending_pcm_gate().has_pending()).await;
}

fn band_limited_noise(rate: usize, mut seed: u64, rms: f64) -> Vec<i16> {
    let mut lowpass = 0.0;
    let mut baseline = 0.0;
    let low = 1.0 - (-std::f64::consts::TAU * 3400.0 / rate as f64).exp();
    let high = 1.0 - (-std::f64::consts::TAU * 250.0 / rate as f64).exp();
    let signal: Vec<f64> = (0..rate * 16)
        .map(|index| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let white = (seed >> 11) as f64 / (1_u64 << 53) as f64 * 2.0 - 1.0;
            lowpass += low * (white - lowpass);
            baseline += high * (lowpass - baseline);
            let envelope = 0.35
                + 0.65
                    * (0.5
                        + 0.5 * (std::f64::consts::TAU * 3.1 * index as f64 / rate as f64).sin())
                    .powi(2);
            (lowpass - baseline) * envelope
        })
        .collect();
    let scale = rms
        / (signal.iter().map(|sample| sample * sample).sum::<f64>() / signal.len() as f64).sqrt();
    signal
        .into_iter()
        .map(|sample| (sample * scale).round() as i16)
        .collect()
}

struct Delivery {
    source: AudioSource,
    start: usize,
    end: usize,
    at: Instant,
}

async fn simulated_capture(
    rate: usize,
    system: &[i16],
    microphone: &[i16],
    batches_ms: (usize, usize),
    offset_ms: i32,
) -> Vec<i16> {
    let factory: ProcessorFactory = Arc::new(move || {
        EchoCanceller::new(rate as u32)
            .map(|processor| Box::new(processor) as Box<dyn EchoProcessor>)
            .map_err(|_| ())
    });
    let mut state = ProcessorState::new(rate / 100, factory().unwrap(), factory);
    let (network, received) = collect();
    let output = network.ingress().unwrap();
    let start = Instant::now();
    let mut deliveries = Vec::new();
    for (source, samples, batch_ms, delay_ms) in [
        (
            AudioSource::System,
            system,
            batches_ms.0,
            offset_ms.max(0) as usize,
        ),
        (
            AudioSource::Microphone,
            microphone,
            batches_ms.1,
            (-offset_ms).max(0) as usize,
        ),
    ] {
        let chunk = rate * batch_ms / 1000;
        for from in (0..samples.len()).step_by(chunk) {
            let end = (from + chunk).min(samples.len());
            deliveries.push(Delivery {
                source,
                start: from,
                end,
                at: start
                    + sample_duration(end, rate / 100)
                    + Duration::from_millis(delay_ms as u64),
            });
        }
    }
    // Microphone wins ties deliberately; the delayed processing must still
    // find render callbacks that arrive later, including 250ms batches.
    deliveries.sort_by_key(|delivery| (delivery.at, delivery.source == AudioSource::System));
    for delivery in deliveries {
        let input = match delivery.source {
            AudioSource::System => system,
            AudioSource::Microphone => microphone,
        };
        state
            .push(work_at(
                delivery.source,
                &input[delivery.start..delivery.end],
                delivery.at,
                &output,
            ))
            .unwrap();
        state.process_ready(delivery.at, false).unwrap();
        assert!(state.reference.len() <= REFERENCE_FRAMES);
        assert!(state.queued_capture.len() <= CAPTURE_BUFFER_MS / 10);
        tokio::task::yield_now().await;
    }
    state.finish().unwrap();
    assert!(network.finish(Duration::from_secs(1)).await);
    assert!(!network.pending_pcm_gate().has_pending());
    let samples: Vec<_> = decode(&received.lock().unwrap()).collect();
    assert_eq!(samples.len(), microphone.len());
    samples
}

#[tokio::test]
async fn echo_pipeline_real_dsp_survives_callback_batching_order_and_clock_offsets() {
    for rate in [16_000, 24_000] {
        let far = band_limited_noise(rate, 0x1234_5678_9abc_def1, 5500.0);
        let near = band_limited_noise(rate, 0xfedc_ba98_7654_3211, 2400.0);
        let echo: Vec<i16> = (0..far.len())
            .map(|index| {
                [(50, 0.55), (63, 0.22), (97, -0.12), (130, 0.04)]
                    .into_iter()
                    .map(|(delay_ms, gain)| {
                        index
                            .checked_sub(rate * delay_ms / 1000)
                            .map_or(0.0, |source| f64::from(far[source]) * gain)
                    })
                    .sum::<f64>()
                    .round() as i16
            })
            .collect();
        for (batches, offset) in [
            ((20, 20), 0),
            ((64, 20), 100),
            ((100, 64), -100),
            ((250, 100), 0),
        ] {
            for double_talk in [false, true] {
                let capture: Vec<i16> = echo
                    .iter()
                    .enumerate()
                    .map(|(index, &echo)| {
                        let near = if double_talk && index >= rate * 4 {
                            near[index]
                        } else {
                            0
                        };
                        (i32::from(echo) + i32::from(near)).clamp(i16::MIN as i32, i16::MAX as i32)
                            as i16
                    })
                    .collect();
                let output = simulated_capture(rate, &far, &capture, batches, offset).await;
                let range = rate * 8..rate * 14;
                if double_talk {
                    let reference = &near[range.clone()];
                    let actual = &output[range];
                    let energy = reference
                        .iter()
                        .map(|&sample| f64::from(sample).powi(2))
                        .sum::<f64>();
                    let gain = reference
                        .iter()
                        .zip(actual)
                        .map(|(&a, &b)| f64::from(a) * f64::from(b))
                        .sum::<f64>()
                        / energy;
                    let error = reference
                        .iter()
                        .zip(actual)
                        .map(|(&a, &b)| (f64::from(a) - f64::from(b)).powi(2))
                        .sum::<f64>();
                    let gain_db = 20.0 * gain.max(1e-30).log10();
                    let sdr = 10.0 * (energy / error.max(1e-30)).log10();
                    eprintln!("AEC actor rate={rate} batches={batches:?} offset={offset} gainDb={gain_db:.2} sdrDb={sdr:.2}");
                    assert!(
                        gain_db > -1.0 && gain_db < 1.0,
                        "near speech changed: {gain_db:.2} dB"
                    );
                    assert!(sdr > 9.0, "double-talk fidelity: {sdr:.2} dB");
                } else {
                    let before = capture[range.clone()]
                        .iter()
                        .map(|&sample| f64::from(sample).powi(2))
                        .sum::<f64>();
                    let after = output[range]
                        .iter()
                        .map(|&sample| f64::from(sample).powi(2))
                        .sum::<f64>();
                    let reduction = 10.0 * (before / after.max(1e-30)).log10();
                    eprintln!("AEC actor rate={rate} batches={batches:?} offset={offset} erleDb={reduction:.2}");
                    assert!(reduction > 15.0, "echo reduction: {reduction:.2} dB");
                }
            }
        }
        let output = simulated_capture(rate, &[], &near, (250, 64), 0).await;
        let reference = &near[rate..rate * 15];
        let actual = &output[rate..rate * 15];
        let energy = reference
            .iter()
            .map(|&sample| f64::from(sample).powi(2))
            .sum::<f64>();
        let gain = reference
            .iter()
            .zip(actual)
            .map(|(&a, &b)| f64::from(a) * f64::from(b))
            .sum::<f64>()
            / energy;
        assert!(
            20.0 * gain.log10() > -1.0,
            "missing reference must preserve near speech"
        );

        // Drop a previously valid reference exactly when near-end speech
        // begins. Check the first 200ms, not only after the learned tail
        // has expired, so stale cancellation cannot mask the onset.
        let mut after_dropout = echo.clone();
        after_dropout[rate * 8..].copy_from_slice(&near[rate * 8..]);
        let output = simulated_capture(rate, &far[..rate * 8], &after_dropout, (100, 64), 0).await;
        let range = rate * 8..rate * 8 + rate / 5;
        let reference = &near[range.clone()];
        let actual = &output[range];
        let energy = reference
            .iter()
            .map(|&sample| f64::from(sample).powi(2))
            .sum::<f64>();
        let gain = reference
            .iter()
            .zip(actual)
            .map(|(&a, &b)| f64::from(a) * f64::from(b))
            .sum::<f64>()
            / energy;
        let error = reference
            .iter()
            .zip(actual)
            .map(|(&a, &b)| (f64::from(a) - f64::from(b)).powi(2))
            .sum::<f64>();
        assert!(
            20.0 * gain.log10() > -1.0,
            "reference dropout suppressed near onset"
        );
        assert!(
            10.0 * (energy / error).log10() > 12.0,
            "old reference tail damaged near onset"
        );
    }
}
