//! CoreMedia sample decoding, multi-channel mono downmixing, and rubato resampling.

use super::AudioHandlerState;
use crate::audio::{macos_block_buffer, SystemAudioCaptureError};
use crate::core::audio::pcm16::PCM16Encoder;
use crate::pipeline_log;
use core_media::format_description::CMAudioFormatDescriptionGetStreamBasicDescription;
use core_media::sample_buffer::{
    CMSampleBufferGetDataBuffer, CMSampleBufferGetFormatDescription, CMSampleBufferRef,
};
use objc2_core_audio_types::{
    kAudioFormatFlagIsFloat, kAudioFormatFlagIsNonInterleaved, kAudioFormatFlagIsSignedInteger,
    AudioStreamBasicDescription,
};
use rubato::audioadapter_buffers::direct::SequentialSlice;
use rubato::Resampler;

/// Tracks the last reported stream format so the capture diagnostics only
/// print when the format changes (avoids log spam at 50 buffers/second).
static FORMAT_SIGNATURE: std::sync::Mutex<Option<(u32, u32, u32)>> = std::sync::Mutex::new(None);

const RESAMPLE_CHUNK_FRAMES: usize = 1024;

/// Extracts the sample buffer as f32 mono samples (honoring the stream's
/// format description), resamples them to the provider rate with `rubato` when needed,
/// and quantizes to PCM16 for the ASR pipeline.
pub(super) fn capture_to_pcm16(
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

    // Per-buffer diagnostics are intentionally absent here: this runs ~48x/s
    // and the throttled `capture decode buffers=` stats plus the send
    // pipeline's peakDbFS already cover silence/decode diagnosis.
    resample_to_pcm16(
        &state.resampler,
        &state.pending_frames,
        state.target_sample_rate_hz,
        &samples,
        sample_rate,
    )
}

pub(super) fn resample_to_pcm16(
    resampler: &std::sync::Mutex<Option<(u32, rubato::Fft<f32>)>>,
    pending_frames: &std::sync::Mutex<Vec<f32>>,
    target_sample_rate_hz: u32,
    samples: &[f32],
    sample_rate: f64,
) -> Result<Option<Vec<u8>>, SystemAudioCaptureError> {
    if samples.is_empty() {
        return Ok(None);
    }
    let rounded_rate = sample_rate.round();
    if !sample_rate.is_finite() || rounded_rate < 1.0 || rounded_rate > u32::MAX as f64 {
        return Err(SystemAudioCaptureError::UnsupportedAudioFormat);
    }
    let input_sample_rate_hz = rounded_rate as u32;
    let target_sample_rate = target_sample_rate_hz as f64;
    // Resample when the device rate differs from the selected provider.
    let pcm = if (sample_rate - target_sample_rate).abs() < 0.5 {
        {
            let mut pending = pending_frames.lock().unwrap();
            let mut resampler_guard = resampler.lock().unwrap();
            pending.clear();
            *resampler_guard = None;
        }
        PCM16Encoder::encode(samples)
    } else {
        let mut pending = pending_frames.lock().unwrap();
        let mut resampler_guard = resampler.lock().unwrap();
        let needs_rebuild = match resampler_guard.as_ref() {
            Some((rate, _)) => *rate != input_sample_rate_hz,
            None => true,
        };
        if needs_rebuild {
            pending.clear();
            *resampler_guard = None;
            let rebuilt = rubato::Fft::<f32>::new(
                input_sample_rate_hz as usize,
                target_sample_rate_hz as usize,
                RESAMPLE_CHUNK_FRAMES,
                1,
                rubato::FixedSync::Input,
            )
            .map_err(|_| SystemAudioCaptureError::AudioProcessingFailed)?;
            *resampler_guard = Some((input_sample_rate_hz, rebuilt));
        }
        pending.extend_from_slice(samples);
        let mut out = Vec::new();
        let (_, active_resampler) = resampler_guard.as_mut().unwrap();
        while pending.len() >= RESAMPLE_CHUNK_FRAMES {
            let output = match SequentialSlice::new(
                &pending[..RESAMPLE_CHUNK_FRAMES],
                1,
                RESAMPLE_CHUNK_FRAMES,
            ) {
                Ok(input) => active_resampler
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
        PCM16Encoder::encode(&out)
    };

    Ok(Some(pcm))
}

/// Decodes interleaved/planar linear PCM into mono f32 samples. Both the
/// float and signed-integer branches average multi-channel buffers here,
/// before resampling and PCM16 encoding.
pub(in crate::audio) fn decode_to_f32_mono(
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
