//! Linear acoustic echo cancellation for explicitly selected dual inputs.
//!
//! The caller supplies corresponding 10 ms render and microphone frames in
//! chronological order. This module owns no queue or clock: missing references,
//! capture discontinuities, and generation changes are handled by its caller.
//! SpeexDSP's nonlinear preprocessor is deliberately not enabled, so near-end
//! speech is not gated merely because the system input is also active.

use std::cell::Cell;
use std::marker::PhantomData;
use std::ptr::NonNull;

use aec_rs_sys as ffi;

// Covers the tested 180 ms playback delay, 100 ms reference lookahead and
// a 130 ms reflection tail, with margin. This finite filter is not a guarantee
// for arbitrary devices.
const FILTER_LENGTH_MS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EchoCancellationError {
    #[error("Unsupported echo cancellation sample rate.")]
    UnsupportedSampleRate,
    #[error("Echo cancellation requires one complete 10 ms frame per input.")]
    InvalidFrameLength,
    #[error("Echo cancellation could not be initialized.")]
    InitializationFailed,
}

/// One independently owned SpeexDSP state. Recreate it after a discontinuity;
/// never continue a learned filter with a reference from another generation.
#[derive(Debug)]
pub struct EchoCanceller {
    state: NonNull<ffi::SpeexEchoState>,
    frame_samples: usize,
    // DSP calls mutate native state. Moving its sole owner is safe, sharing it
    // across concurrent callers is not, even if the pointer type changes later.
    not_sync: PhantomData<Cell<()>>,
}

// SAFETY: The allocation belongs exclusively to this instance, has no thread
// affinity, and every processing call requires &mut self. Drop is likewise
// exclusive. No raw state pointer is exposed to callers.
unsafe impl Send for EchoCanceller {}

impl EchoCanceller {
    pub fn new(sample_rate: u32) -> Result<Self, EchoCancellationError> {
        if !matches!(sample_rate, 16_000 | 24_000) {
            return Err(EchoCancellationError::UnsupportedSampleRate);
        }
        let frame_samples = sample_rate as usize / 100;
        let filter_samples = sample_rate as usize * FILTER_LENGTH_MS / 1000;
        // SAFETY: Both lengths are positive, bounded by the validated rates,
        // and fit in the C API's signed integer parameters.
        let state = NonNull::new(unsafe {
            ffi::speex_echo_state_init(frame_samples as i32, filter_samples as i32)
        })
        .ok_or(EchoCancellationError::InitializationFailed)?;
        let canceller = Self {
            state,
            frame_samples,
            not_sync: PhantomData,
        };
        let mut rate = sample_rate as i32;
        // SAFETY: SET_SAMPLING_RATE reads one initialized C int synchronously.
        // The owned state and rate pointer remain valid throughout the call.
        let result = unsafe {
            ffi::speex_echo_ctl(
                canceller.state.as_ptr(),
                ffi::SPEEX_ECHO_SET_SAMPLING_RATE as i32,
                (&mut rate as *mut i32).cast(),
            )
        };
        if result != 0 {
            // The already-owned state is released by Drop on this error path.
            return Err(EchoCancellationError::InitializationFailed);
        }
        Ok(canceller)
    }

    /// Removes the supplied render reference from a microphone frame.
    ///
    /// Exact lengths are checked before entering C. Neither input is modified;
    /// the system-audio lane must continue sending its original input separately.
    pub fn process_pair(
        &mut self,
        render: &[i16],
        capture: &[i16],
    ) -> Result<Vec<i16>, EchoCancellationError> {
        if render.len() != self.frame_samples || capture.len() != self.frame_samples {
            return Err(EchoCancellationError::InvalidFrameLength);
        }
        let mut output = vec![0; self.frame_samples];
        // SAFETY: The native state was initialized for exactly these lengths.
        // Input buffers remain valid, output has its own non-aliasing allocation,
        // and &mut self guarantees exclusive access to mutable DSP state.
        unsafe {
            ffi::speex_echo_cancellation(
                self.state.as_ptr(),
                capture.as_ptr(),
                render.as_ptr(),
                output.as_mut_ptr(),
            );
        }
        Ok(output)
    }
}

impl Drop for EchoCanceller {
    fn drop(&mut self) {
        // SAFETY: This is the sole owner of the non-null allocation, and no
        // native call can outlive a method's exclusive borrow.
        unsafe { ffi::speex_echo_state_destroy(self.state.as_ptr()) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    const SECONDS: usize = 20;
    const NEAR_SEED: u64 = 0xfedc_ba98_7654_3211;
    const FAR_SEED: u64 = 0x1234_5678_9abc_def1;

    fn energy(samples: &[f32]) -> f64 {
        samples
            .iter()
            .map(|&sample| f64::from(sample).powi(2))
            .sum()
    }

    fn dot(left: &[f32], right: &[f32]) -> f64 {
        left.iter()
            .zip(right)
            .map(|(&a, &b)| f64::from(a) * f64::from(b))
            .sum()
    }

    fn decibels(numerator: f64, denominator: f64) -> f64 {
        10.0 * (numerator.max(1e-30) / denominator.max(1e-30)).log10()
    }

    // Independent, reproducible, non-sensitive signals. The broadband variant
    // excites many filter coefficients; the voiced variant adds independently
    // drifting fundamentals, formants and syllabic envelopes. Neither is a
    // recorded voice, an ASR fixture, or a substitute for device validation.
    fn signal(rate: usize, mut seed: u64, rms: f64, voiced: bool) -> Vec<f32> {
        let near = seed == NEAR_SEED;
        let mut phase = 0.0;
        let mut lowpass = 0.0;
        let mut baseline = 0.0;
        let low_coefficient = 1.0 - (-2.0 * PI * 3400.0 / rate as f64).exp();
        let high_coefficient = 1.0 - (-2.0 * PI * 250.0 / rate as f64).exp();
        let mut output = Vec::with_capacity(rate * SECONDS);
        for index in 0..rate * SECONDS {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let white = (seed >> 11) as f64 / (1_u64 << 53) as f64 * 2.0 - 1.0;
            lowpass += low_coefficient * (white - lowpass);
            baseline += high_coefficient * (lowpass - baseline);
            let time = index as f64 / rate as f64;
            let sample = if voiced {
                let fundamental = if near { 177.0 } else { 123.0 };
                phase +=
                    2.0 * PI * (fundamental + 8.0 * (2.0 * PI * 0.7 * time).sin()) / rate as f64;
                let harmonic = (1..=24)
                    .map(|harmonic| {
                        let frequency = f64::from(harmonic) * fundamental;
                        let formant = 0.2
                            + (-((frequency - 650.0) / 250.0).powi(2)).exp()
                            + 0.7 * (-((frequency - 1400.0) / 350.0).powi(2)).exp()
                            + 0.4 * (-((frequency - 2600.0) / 450.0).powi(2)).exp();
                        (phase * f64::from(harmonic)).sin() * formant / f64::from(harmonic).sqrt()
                    })
                    .sum::<f64>();
                let envelope = 0.15
                    + 0.85
                        * (0.5 + 0.5 * (2.0 * PI * if near { 3.7 } else { 2.3 } * time).sin())
                            .powi(2);
                (0.8 * harmonic + 0.1 * (lowpass - baseline)) * envelope
            } else {
                let envelope = 0.35 + 0.65 * (0.5 + 0.5 * (2.0 * PI * 3.1 * time).sin()).powi(2);
                (lowpass - baseline) * envelope
            };
            output.push(sample as f32);
        }
        let scale = rms / (energy(&output) / output.len() as f64).sqrt();
        for sample in &mut output {
            *sample *= scale as f32;
        }
        assert!(output.iter().all(|sample| sample.abs() < 0.95));
        output
    }

    fn reflected(render: &[f32], rate: usize, delay_ms: usize, room: bool) -> Vec<f32> {
        let taps: &[(usize, f32)] = if room {
            &[
                (0, 0.55),
                (13, 0.22),
                (29, -0.16),
                (47, 0.09),
                (78, 0.04),
                (130, -0.02),
            ]
        } else {
            &[(0, 0.55)]
        };
        let mut echo = vec![0.0; render.len()];
        for &(offset, gain) in taps {
            let delay = rate * (delay_ms + offset) / 1000;
            for index in delay..render.len() {
                echo[index] += render[index - delay] * gain;
            }
        }
        echo
    }

    fn quantize(input: &[f32]) -> Vec<i16> {
        input
            .iter()
            .map(|sample| (sample * 32767.0).round() as i16)
            .collect()
    }

    fn process(canceller: &mut EchoCanceller, render: &[f32], capture: &[f32]) -> Vec<f32> {
        assert_eq!(render.len(), capture.len());
        let frame = canceller.frame_samples;
        assert_eq!(capture.len() % frame, 0);
        let mut output = Vec::with_capacity(capture.len());
        for (render, capture) in render.chunks_exact(frame).zip(capture.chunks_exact(frame)) {
            let render = quantize(render);
            let capture = quantize(capture);
            output.extend(
                canceller
                    .process_pair(&render, &capture)
                    .unwrap()
                    .into_iter()
                    .map(|sample| f32::from(sample) / 32767.0),
            );
        }
        assert_eq!(output.len(), capture.len());
        output
    }

    fn run(rate: usize, render: &[f32], capture: &[f32]) -> Vec<f32> {
        process(
            &mut EchoCanceller::new(rate as u32).unwrap(),
            render,
            capture,
        )
    }

    #[derive(Debug)]
    struct Fidelity {
        gain_db: f64,
        sdr_db: f64,
    }

    fn fidelity(reference: &[f32], output: &[f32]) -> Fidelity {
        let reference_energy = energy(reference);
        assert!(reference_energy > 1e-6);
        let gain = dot(reference, output) / reference_energy;
        let error = reference
            .iter()
            .zip(output)
            .map(|(&a, &b)| f64::from(a - b).powi(2))
            .sum();
        Fidelity {
            gain_db: 20.0 * gain.max(1e-30).log10(),
            sdr_db: decibels(reference_energy, error),
        }
    }

    // Score against the near-only DSP output: Speex's fixed DC notch changes
    // waveform phase even without echo. Gain remains unnormalized, so silence
    // or broad attenuation cannot pass. The matrix separately requires dry
    // gain within 0.5 dB, >=10 dB mixture improvement, and >=20 dB echo rejection.
    fn retains_near(metrics: &Fidelity) -> bool {
        metrics.gain_db.abs() < 0.2 && metrics.sdr_db > 14.0
    }

    #[test]
    fn validates_rates_and_exact_frames_before_ffi() {
        for rate in [0, 8_000, 44_100, 48_000, u32::MAX] {
            assert_eq!(
                EchoCanceller::new(rate).unwrap_err(),
                EchoCancellationError::UnsupportedSampleRate
            );
        }
        for rate in [16_000, 24_000] {
            let mut canceller = EchoCanceller::new(rate).unwrap();
            let frame = rate as usize / 100;
            let valid = vec![0; frame];
            for wrong in [0, 1, frame - 1, frame + 1, frame * 2] {
                assert_eq!(
                    canceller.process_pair(&vec![0; wrong], &valid),
                    Err(EchoCancellationError::InvalidFrameLength)
                );
                assert_eq!(
                    canceller.process_pair(&valid, &vec![0; wrong]),
                    Err(EchoCancellationError::InvalidFrameLength)
                );
            }
            assert_eq!(canceller.process_pair(&valid, &valid).unwrap(), valid);
        }
    }

    #[test]
    fn can_move_its_exclusive_state_to_the_audio_worker() {
        let mut canceller = EchoCanceller::new(16_000).unwrap();
        let output =
            std::thread::spawn(move || canceller.process_pair(&[0; 160], &[0; 160]).unwrap())
                .join()
                .unwrap();
        assert_eq!(output, vec![0; 160]);
    }

    #[test]
    fn quantitative_echo_rejection_and_double_talk_preservation() {
        for rate in [16_000, 24_000] {
            for voiced in [false, true] {
                let render = signal(rate, FAR_SEED, 0.12, voiced);
                let near = signal(rate, NEAR_SEED, 0.08, voiced);
                let silence = vec![0.0; near.len()];
                let baseline = run(rate, &silence, &near);
                let window = rate * 10..rate * 18;
                // The built-in DC notch has a fixed frequency response, so also
                // score against its near-only output; do not normalize gain or
                // move the scoring window to hide missing near-end speech.
                let clean = fidelity(&near[window.clone()], &baseline[window.clone()]);
                assert!(
                    clean.gain_db.abs() < 0.3 && clean.sdr_db > 14.0,
                    "near-only rate={rate} voiced={voiced} metrics={clean:?}"
                );
                for (delay, room) in [(40, false), (80, true), (180, true), (280, true)] {
                    let echo = reflected(&render, rate, delay, room);
                    let echo_output = run(rate, &render, &echo);
                    let erle = decibels(
                        energy(&echo[window.clone()]),
                        energy(&echo_output[window.clone()]),
                    );
                    assert!(
                        erle > 20.0,
                        "echo rate={rate} voiced={voiced} delay={delay} erleDb={erle}"
                    );
                    let mut speaking = near.clone();
                    speaking[..rate * 8].fill(0.0);
                    let mixture: Vec<f32> =
                        echo.iter().zip(&speaking).map(|(&a, &b)| a + b).collect();
                    assert!(mixture.iter().all(|sample| sample.abs() < 0.95));
                    let output = run(rate, &render, &mixture);
                    let metrics = fidelity(&speaking[window.clone()], &output[window.clone()]);
                    let baseline_metrics =
                        fidelity(&baseline[window.clone()], &output[window.clone()]);
                    let raw_sdr = decibels(
                        energy(&speaking[window.clone()]),
                        energy(&echo[window.clone()]),
                    );
                    assert!(
                        metrics.gain_db.abs() < 0.5,
                        "double-talk rate={rate} voiced={voiced} delay={delay} metrics={metrics:?}"
                    );
                    assert!(retains_near(&baseline_metrics),
                        "double-talk baseline rate={rate} voiced={voiced} delay={delay} metrics={baseline_metrics:?}");
                    assert!(
                        metrics.sdr_db - raw_sdr > 10.0,
                        "double-talk improvement rate={rate} voiced={voiced} delay={delay}"
                    );
                    let onset = rate * 8..rate * 8 + rate / 2;
                    let onset_metrics = fidelity(&baseline[onset.clone()], &output[onset]);
                    assert!(retains_near(&onset_metrics), "near onset rate={rate} voiced={voiced} delay={delay} metrics={onset_metrics:?}");
                    eprintln!("AEC rate={rate} voiced={voiced} delayMs={delay} erleDb={erle:.2} nearGainDb={:.3} dryNearSdrDb={:.2} improvementDb={:.2} baselineSdrDb={:.2} baselineGainDb={:.3}", metrics.gain_db, metrics.sdr_db, metrics.sdr_db - raw_sdr, baseline_metrics.sdr_db, baseline_metrics.gain_db);
                }
            }
        }
    }

    #[test]
    fn follows_a_changed_room_path_without_muting_the_microphone() {
        for rate in [16_000, 24_000] {
            let render = signal(rate, FAR_SEED, 0.12, false);
            let first = reflected(&render, rate, 40, false);
            let second = reflected(&render, rate, 180, true);
            let mut echo = first;
            echo[rate * 10..].copy_from_slice(&second[rate * 10..]);
            let output = run(rate, &render, &echo);
            let window = rate * 16..rate * 19;
            let erle = decibels(energy(&echo[window.clone()]), energy(&output[window]));
            assert!(erle > 18.0, "changed path rate={rate} erleDb={erle}");
        }
    }

    #[test]
    fn absent_reference_preserves_near_and_recreated_state_has_no_old_tail() {
        for rate in [16_000, 24_000] {
            let near = signal(rate, NEAR_SEED, 0.08, false);
            let render = signal(rate, FAR_SEED, 0.12, false);
            let echo = reflected(&render, rate, 80, true);
            let silence = vec![0.0; near.len()];
            let baseline = run(rate, &silence, &near);
            let mut learned = EchoCanceller::new(rate as u32).unwrap();
            process(&mut learned, &render[..rate * 8], &echo[..rate * 8]);
            let without_reference = process(&mut learned, &silence[..rate * 4], &near[..rate * 4]);
            // Skip only the bounded old filter tail after the reference ends;
            // an application discontinuity instead recreates immediately below.
            let metrics = fidelity(
                &baseline[rate..rate * 4],
                &without_reference[rate..rate * 4],
            );
            assert!(
                retains_near(&metrics),
                "missing reference rate={rate} metrics={metrics:?}"
            );
            drop(learned);
            let after_reset = run(rate, &silence[..rate], &near[..rate]);
            assert_eq!(
                after_reset,
                baseline[..rate],
                "fresh generation must match a fresh filter from its first sample"
            );
        }
    }

    #[test]
    fn muted_attenuated_or_passthrough_controls_cannot_pass_quality_gates() {
        let rate = 16_000;
        let near = signal(rate, NEAR_SEED, 0.08, false);
        let render = signal(rate, FAR_SEED, 0.12, false);
        let echo = reflected(&render, rate, 80, true);
        let zero = vec![0.0; near.len()];
        let attenuated: Vec<_> = near.iter().map(|sample| sample * 0.1).collect();
        assert!(!retains_near(&fidelity(&near, &zero)));
        assert!(!retains_near(&fidelity(&near, &attenuated)));
        assert_eq!(decibels(energy(&echo), energy(&echo)), 0.0);
        let passthrough: Vec<_> = near
            .iter()
            .zip(&echo)
            .map(|(near, echo)| near + echo)
            .collect();
        assert!(!retains_near(&fidelity(&near, &passthrough)));
    }
}
