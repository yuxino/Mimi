//! Stateful interleaved-audio downmixing and resampling shared by the
//! native capture adapters and platform-independent regression tests.

use crate::audio::SystemAudioCaptureError;
use crate::core::pcm16::PCM16Encoder;
use rubato::audioadapter_buffers::direct::SequentialSlice;
use rubato::{FixedSync, Resampler};

const RESAMPLE_CHUNK_FRAMES: usize = 1024;

pub(crate) struct StreamingPcm16Resampler {
    channel_count: usize,
    input_sample_rate_hz: u32,
    target_sample_rate_hz: u32,
    pending_interleaved: Vec<f32>,
    pending_mono: Vec<f32>,
    resampler: Option<rubato::Fft<f32>>,
    output_scratch: Vec<f32>,
}

impl StreamingPcm16Resampler {
    pub(crate) fn new(
        input_sample_rate_hz: u32,
        target_sample_rate_hz: u32,
        channel_count: usize,
    ) -> Result<Self, SystemAudioCaptureError> {
        if channel_count == 0 || input_sample_rate_hz == 0 || target_sample_rate_hz == 0 {
            return Err(SystemAudioCaptureError::UnsupportedAudioFormat);
        }
        let resampler = if input_sample_rate_hz == target_sample_rate_hz {
            None
        } else {
            Some(
                rubato::Fft::<f32>::new(
                    input_sample_rate_hz as usize,
                    target_sample_rate_hz as usize,
                    RESAMPLE_CHUNK_FRAMES,
                    1,
                    FixedSync::Input,
                )
                .map_err(|_| SystemAudioCaptureError::AudioProcessingFailed)?,
            )
        };
        let output_scratch = resampler
            .as_ref()
            .map(|resampler| Vec::with_capacity(resampler.output_frames_max()))
            .unwrap_or_default();
        Ok(Self {
            channel_count,
            input_sample_rate_hz,
            target_sample_rate_hz,
            pending_interleaved: Vec::new(),
            pending_mono: Vec::new(),
            resampler,
            output_scratch,
        })
    }

    /// Accepts any callback size. Incomplete interleaved frames and incomplete
    /// resampler blocks remain buffered for the next callback.
    pub(crate) fn push_interleaved(
        &mut self,
        data: &[f32],
    ) -> Result<Vec<Vec<u8>>, SystemAudioCaptureError> {
        self.pending_interleaved.extend_from_slice(data);
        let complete_sample_count =
            self.pending_interleaved.len() / self.channel_count * self.channel_count;
        if complete_sample_count > 0 {
            let channel_count = self.channel_count;
            self.pending_mono.extend(
                self.pending_interleaved[..complete_sample_count]
                    .chunks_exact(channel_count)
                    .map(|frame| frame.iter().copied().sum::<f32>() / channel_count as f32),
            );
            self.pending_interleaved.drain(..complete_sample_count);
        }

        if self.input_sample_rate_hz == self.target_sample_rate_hz {
            if self.pending_mono.is_empty() {
                return Ok(Vec::new());
            }
            let pcm = PCM16Encoder::encode(std::slice::from_ref(&self.pending_mono));
            self.pending_mono.clear();
            return Ok(vec![pcm]);
        }

        let Some(resampler) = self.resampler.as_mut() else {
            return Err(SystemAudioCaptureError::UnsupportedAudioFormat);
        };
        let mut buffers = Vec::new();
        while self.pending_mono.len() >= RESAMPLE_CHUNK_FRAMES {
            let output_frames = resampler.output_frames_next();
            self.output_scratch.resize(output_frames, 0.0);
            let output = match (
                SequentialSlice::new(
                    &self.pending_mono[..RESAMPLE_CHUNK_FRAMES],
                    1,
                    RESAMPLE_CHUNK_FRAMES,
                ),
                SequentialSlice::new_mut(&mut self.output_scratch, 1, output_frames),
            ) {
                (Ok(input), Ok(mut output)) => resampler
                    .process_into_buffer(&input, &mut output, None)
                    .map_err(|_| SystemAudioCaptureError::AudioProcessingFailed),
                _ => Err(SystemAudioCaptureError::AudioProcessingFailed),
            };
            // Match the previous drain-before-process behavior on both success
            // and failure while avoiding an owned input copy.
            self.pending_mono.drain(..RESAMPLE_CHUNK_FRAMES);
            let pcm = output.map(|(_, written)| {
                self.output_scratch.truncate(written);
                PCM16Encoder::encode(std::slice::from_ref(&self.output_scratch))
            });
            self.output_scratch.clear();
            let pcm = pcm?;
            if !pcm.is_empty() {
                buffers.push(pcm);
            }
        }
        Ok(buffers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stereo_sine(start_frame: usize, frame_count: usize) -> Vec<f32> {
        (start_frame..start_frame + frame_count)
            .flat_map(|frame| {
                let sample = (frame as f64 * 2.0 * std::f64::consts::PI * 1_000.0 / 48_000.0).sin()
                    as f32
                    * 0.5;
                [sample, sample]
            })
            .collect()
    }

    #[test]
    fn four_small_callbacks_fill_one_resampler_block() {
        let mut resampler = StreamingPcm16Resampler::new(48_000, 24_000, 2).unwrap();

        for callback in 0..3 {
            let output = resampler
                .push_interleaved(&stereo_sine(callback * 256, 256))
                .unwrap();
            assert!(output.is_empty());
        }
        let output = resampler
            .push_interleaved(&stereo_sine(3 * 256, 256))
            .unwrap();

        assert_eq!(output.len(), 1);
        assert!(!output[0].is_empty());
    }

    #[test]
    fn resampling_tracks_output_lengths_and_preserves_audible_signal() {
        for (input_rate, target_rate, frame_counts) in [
            (48_000, 24_000, [512, 512, 512, 512]),
            (48_000, 16_000, [258, 344, 344, 344]),
            (44_100, 24_000, [480, 480, 640, 480]),
            (44_100, 16_000, [320, 320, 320, 480]),
        ] {
            let mut resampler = StreamingPcm16Resampler::new(input_rate, target_rate, 2).unwrap();
            for (block, frames) in frame_counts.into_iter().enumerate() {
                let output = resampler
                    .push_interleaved(&stereo_sine(
                        block * RESAMPLE_CHUNK_FRAMES,
                        RESAMPLE_CHUNK_FRAMES,
                    ))
                    .unwrap();
                assert_eq!(output.len(), 1);
                assert_eq!(output[0].len(), frames * 2);
                let peak = output[0]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|sample| i32::from(i16::from_le_bytes(*sample)).abs())
                    .max()
                    .unwrap_or(0);

                assert!(peak > 100, "resampled PCM unexpectedly silent: peak={peak}");
            }
        }
    }

    #[test]
    fn incomplete_interleaved_frame_is_kept_for_the_next_callback() {
        let mut resampler = StreamingPcm16Resampler::new(24_000, 24_000, 2).unwrap();
        assert!(resampler.push_interleaved(&[0.5]).unwrap().is_empty());
        let output = resampler.push_interleaved(&[0.5]).unwrap();
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].len(), 2);
    }

    #[test]
    fn same_rate_callbacks_reuse_the_mono_buffer_allocation() {
        let mut resampler = StreamingPcm16Resampler::new(24_000, 24_000, 2).unwrap();
        let first = stereo_sine(0, 256);
        let output = resampler.push_interleaved(&first).unwrap();

        assert_eq!(output.len(), 1);
        assert!(resampler.pending_mono.is_empty());
        assert_eq!(resampler.output_scratch.capacity(), 0);
        assert!(resampler.pending_mono.capacity() >= 256);
        let allocation = resampler.pending_mono.as_ptr();
        let capacity = resampler.pending_mono.capacity();

        let second = stereo_sine(256, 256);
        let output = resampler.push_interleaved(&second).unwrap();

        assert_eq!(output.len(), 1);
        assert!(resampler.pending_mono.is_empty());
        assert_eq!(resampler.output_scratch.capacity(), 0);
        assert_eq!(resampler.pending_mono.capacity(), capacity);
        assert_eq!(resampler.pending_mono.as_ptr(), allocation);
    }
}
