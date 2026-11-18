//! Test-only, recognition-first denoising candidate. Never a production gate.
//!
//! Speex uses one frame of analysis overlap. Flush it and remove startup delay
//! for fixed-PCM comparison so the candidate cannot accidentally lose a tail.

use super::{Failure, MAX_PCM_BYTES};
use aec_rs_sys as ffi;
use serde::Serialize;
use std::ptr::NonNull;
use std::time::Instant;

const SAMPLE_RATE: usize = 16_000;
const FRAME_SAMPLES: usize = SAMPLE_RATE / 50;
const SUPPRESSION_DB: i32 = -12;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum Kind {
    #[default]
    Bypass,
    SpeexMild,
}

impl Kind {
    pub(super) fn parse(value: &str) -> Result<Self, Failure> {
        match value {
            "bypass" => Ok(Self::Bypass),
            "speex-mild" => Ok(Self::SpeexMild),
            _ => Err(Failure::ManifestInvalid),
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct Metrics {
    pub(super) kind: Kind,
    input_samples: usize,
    output_samples: usize,
    frames_including_flush: usize,
    /// Excludes frame assembly waiting and CPU processing time.
    analysis_overlap_ms: u64,
    frame_duration_ms: u64,
    frame_processing_p50_us: u64,
    frame_processing_p95_us: u64,
    frame_processing_max_us: u64,
    total_processing_us: u64,
    /// Includes decoding, padding, flush, delay compensation and encoding.
    real_time_factor: f64,
}

struct Processor(NonNull<ffi::SpeexPreprocessState>);

impl Processor {
    fn new(denoise: bool) -> Result<Self, Failure> {
        // SAFETY: Fixed positive frame/rate, with exclusive native ownership.
        let state = NonNull::new(unsafe {
            ffi::speex_preprocess_state_init(FRAME_SAMPLES as i32, SAMPLE_RATE as i32)
        })
        .ok_or(Failure::InputInvalid)?;
        let processor = Self(state);
        for (request, mut value) in [
            (ffi::SPEEX_PREPROCESS_SET_DENOISE, i32::from(denoise)),
            (ffi::SPEEX_PREPROCESS_SET_DEREVERB, 0),
            (ffi::SPEEX_PREPROCESS_SET_NOISE_SUPPRESS, SUPPRESSION_DB),
        ] {
            // SAFETY: ctl synchronously reads this initialized C int; state is
            // exclusively owned and remains alive for the complete operation.
            if unsafe {
                ffi::speex_preprocess_ctl(
                    processor.0.as_ptr(),
                    request as i32,
                    (&mut value as *mut i32).cast(),
                )
            } != 0
            {
                return Err(Failure::InputInvalid);
            }
        }
        // The pinned aec-rs-sys fixed-point build excludes AGC entirely. VAD
        // defaults off; verify it without invoking the deprecated SET_VAD API.
        let mut vad = 1i32;
        // SAFETY: GET_VAD synchronously writes one C int in valid owned state.
        if unsafe {
            ffi::speex_preprocess_ctl(
                processor.0.as_ptr(),
                ffi::SPEEX_PREPROCESS_GET_VAD as i32,
                (&mut vad as *mut i32).cast(),
            )
        } != 0
            || vad != 0
        {
            return Err(Failure::InputInvalid);
        }
        Ok(processor)
    }

    fn frame(&mut self, samples: &mut [i16; FRAME_SAMPLES]) {
        // SAFETY: An entire initialized native-sized PCM frame and exclusive
        // state. Ignore VAD's return value: no audio is ever gated or discarded.
        unsafe { ffi::speex_preprocess_run(self.0.as_ptr(), samples.as_mut_ptr()) };
    }
}

impl Drop for Processor {
    fn drop(&mut self) {
        // SAFETY: Exactly one owner; no pointer escapes or concurrent calls.
        unsafe { ffi::speex_preprocess_state_destroy(self.0.as_ptr()) };
    }
}

fn process(pcm: &[u8], denoise: bool) -> Result<(Vec<u8>, Metrics), Failure> {
    if pcm.is_empty() || !pcm.len().is_multiple_of(2) || pcm.len() > MAX_PCM_BYTES {
        return Err(Failure::InputInvalid);
    }
    let start = Instant::now();
    let mut processor = Processor::new(denoise)?;
    let input_samples = pcm.len() / 2;
    let frame_count = input_samples.div_ceil(FRAME_SAMPLES);
    let mut result = Vec::with_capacity((frame_count + 1) * FRAME_SAMPLES * 2);
    let mut durations = Vec::with_capacity(frame_count + 1);
    for bytes in pcm
        .chunks(FRAME_SAMPLES * 2)
        .chain(std::iter::once(&[][..]))
    {
        let mut frame = [0i16; FRAME_SAMPLES];
        for (sample, pair) in frame.iter_mut().zip(bytes.as_chunks::<2>().0) {
            *sample = i16::from_le_bytes(*pair);
        }
        let before = Instant::now();
        processor.frame(&mut frame);
        durations.push(before.elapsed().as_micros() as u64);
        result.extend(frame.into_iter().flat_map(i16::to_le_bytes));
    }
    // Compensate one frame of internal overlap after flushing it. Streaming
    // integration would need to retain this delay and drain on stop, not simply
    // trim each callback independently.
    result.drain(..FRAME_SAMPLES * 2);
    result.truncate(pcm.len());
    let total_processing_us = start.elapsed().as_micros() as u64;
    durations.sort_unstable();
    let percentile =
        |percent: usize| durations[(durations.len() * percent).div_ceil(100).saturating_sub(1)];
    let metrics = Metrics {
        kind: Kind::SpeexMild,
        input_samples,
        output_samples: result.len() / 2,
        frames_including_flush: durations.len(),
        analysis_overlap_ms: 20,
        frame_duration_ms: 20,
        frame_processing_p50_us: percentile(50),
        frame_processing_p95_us: percentile(95),
        frame_processing_max_us: *durations.last().unwrap(),
        total_processing_us,
        real_time_factor: total_processing_us as f64 * SAMPLE_RATE as f64
            / (1_000_000.0 * input_samples as f64),
    };
    Ok((result, metrics))
}

pub(super) fn apply(pcm: &[u8], kind: Kind) -> Result<Vec<u8>, Failure> {
    match kind {
        Kind::Bypass => Ok(pcm.to_vec()),
        Kind::SpeexMild => process(pcm, true).map(|(bytes, _)| bytes),
    }
}

#[test]
fn selection_is_explicit_and_unknown_names_fail_closed() {
    assert_eq!(Kind::default(), Kind::Bypass);
    assert_eq!(Kind::parse("speex-mild").unwrap(), Kind::SpeexMild);
    assert!(Kind::parse("aggressive-private-value").is_err());
}

#[test]
fn bypass_is_byte_exact_and_does_not_modify_the_source() {
    let input = vec![1, 0, 0xff, 0x7f, 0, 0x80];
    let copy = input.clone();
    assert_eq!(apply(&input, Kind::Bypass).unwrap(), input);
    assert_eq!(input, copy);
}

#[test]
fn invalid_or_oversized_pcm_is_rejected_before_native_processing() {
    for input in [vec![], vec![1], vec![0; MAX_PCM_BYTES + 2]] {
        assert!(process(&input, true).is_err());
    }
}

#[test]
fn partial_frames_and_quiet_audio_are_never_gated() {
    for count in [
        1,
        FRAME_SAMPLES - 1,
        FRAME_SAMPLES,
        FRAME_SAMPLES + 1,
        2_117,
    ] {
        let input: Vec<_> = (0..count).flat_map(|_| 3i16.to_le_bytes()).collect();
        let (output, metrics) = process(&input, true).unwrap();
        assert_eq!(output.len(), input.len());
        assert_eq!(metrics.output_samples, count);
        assert_eq!(
            metrics.frames_including_flush,
            count.div_ceil(FRAME_SAMPLES) + 1
        );
    }
}

#[test]
fn flush_and_delay_compensation_preserve_a_nonzero_last_sample() {
    for count in [1, FRAME_SAMPLES - 1, FRAME_SAMPLES, FRAME_SAMPLES + 7] {
        let mut input = vec![0u8; count * 2];
        input[(count - 1) * 2..].copy_from_slice(&8_000i16.to_le_bytes());
        let (output, _) = process(&input, false).unwrap();
        let last = i16::from_le_bytes(output[output.len() - 2..].try_into().unwrap());
        assert!((last as i32 - 8_000).abs() < 8, "count={count} last={last}");
    }
}

#[test]
fn stationary_noise_can_be_reduced_without_changing_its_duration() {
    let mut seed = 123u32;
    let input: Vec<_> = (0..SAMPLE_RATE * 3)
        .flat_map(|_| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (((seed >> 16) as i16) / 8).to_le_bytes()
        })
        .collect();
    let (output, _) = process(&input, true).unwrap();
    let energy = |pcm: &[u8]| {
        pcm[pcm.len() - SAMPLE_RATE * 2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| f64::from(i16::from_le_bytes(*pair)).powi(2))
            .sum::<f64>()
    };
    assert_eq!(output.len(), input.len());
    assert!(energy(&output) < energy(&input) * 0.8);
}

#[test]
fn numeric_metrics_contain_no_audio_or_reference_content() {
    let (_, metrics) = process(&[0; 640], true).unwrap();
    let json = serde_json::to_string(&metrics).unwrap();
    assert!(json.contains("analysis_overlap_ms"));
    for forbidden in ["pcm", "reference", "path", "credential"] {
        assert!(!json.contains(forbidden));
    }
}

#[test]
#[ignore = "manual offline DSP benchmark using an explicit public-fixture manifest; no credentials or network"]
fn manual_public_corpus_cpu_comparison() {
    let result = (|| {
        let path = std::env::var_os("MIMI_ASR_BENCH_MANIFEST")
            .map(std::path::PathBuf::from)
            .ok_or(Failure::ManifestInvalid)?;
        for clip in super::prepare_manifest(&path)? {
            let (_, metrics) = process(&clip.pcm, true)?;
            println!(
                "DENOISE_BENCH_JSON {}",
                serde_json::json!({"label": clip.label, "metrics": metrics})
            );
        }
        Ok::<_, Failure>(())
    })();
    if let Err(failure) = result {
        println!(
            "DENOISE_BENCH_JSON {}",
            serde_json::json!({"failure": failure})
        );
        panic!("Offline denoising benchmark failed; see fixed classification");
    }
}
