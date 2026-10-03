# Subtitle completeness investigation

The user reports apparently missing content across potentially clean dialogue,
mixed dialogue and music. A specific failing excerpt and source/translation
comparison are not yet available. This investigation does not establish a root
cause for that session. It fixes two independently reproduced capture defects;
it does not install speech enhancement.

## Initial observations

The signed development app was idle when initially inspected. Saved preferences
cannot establish the route of an earlier session, and an idle diagnostic page
has no capture observation or request timing. Neither proves whether the
reported session lost audio. Later configuration changes likewise do not
identify an earlier session's recognition or translation route.

## Capture fixes and enhancement results

The macOS callback could decode only the first contiguous prefix of a segmented
CoreMedia buffer. It also confused the non-interleaved and nonmixable format
flags. The [capture design and checks](../plans/2026-10-03-capture-completeness.md)
document full bounded buffer reads and corrected multichannel frame ordering.
Native fixtures reproduce these defects without retaining user recordings.
Requested mono capture is unaffected by the channel-layout distinction.

The [fixed-input denoising evaluation](2026-10-03-asr-denoising-results.md)
completed 32 provider runs. Both tested enhancement parameters caused recognition
regressions and were not promoted. These results are independent of capture
completeness: directly supplied benchmark PCM bypasses native capture.

## Boundaries audited

- System capture decodes/downmixes PCM and resamples for the provider. No local
  voice-activity gate or speech/music separator trims system audio. This source
  audit is not proof of native callback continuity or successful capture of a
  particular app/output route.
- The bounded send queue reports backpressure and retires the failing ingress;
  transport failure is also explicit. Accepted queued buffers drain during
  graceful shutdown, subject to its finite deadline. These guarantees do not
  rule out lost material during a failure, reconnection or forced cancellation.
- Existing dual-input echo cancellation modifies microphone PCM only; system
  PCM continues unchanged. Removing speaker echo from the microphone is not
  extracting dialogue from music in the system soundtrack.
- HQ finals retain source/translation pairs and order; preview cancellation
  replaces drafts. Queue overload, expiry and failed final requests report
  failure rather than certify a complete translation. Compatible HTTP decoding
  rejects explicit unfinished `finish_reason`, including `length`; it cannot
  detect a semantically incomplete result marked complete by a service.
- Following-mode subtitle lanes intentionally clip older visual lines in long
  sentences. Upward reading intent reveals the full retained sentence. Compact
  repetition folding is presentation-only. A short visible tail is therefore
  not sufficient evidence of capture or recognition loss.

Prior [controlled real-audio work](2026-10-02-real-audio-comparison.md) observed
recognition mistakes and revising drafts with identical directly supplied PCM.
That establishes an ASR quality limitation for those samples, not the cause of
this new report.

## Real-time enhancement feasibility

[DeepFilterNet](https://github.com/Rikorose/DeepFilterNet) supplies real-time
speech enhancement and a Rust implementation. The
[DeepFilterNet2 paper](https://arxiv.org/abs/2205.05474) reports CPU real-time
factor 0.04 on its notebook hardware; this is processing throughput, not a
40 ms end-to-end latency guarantee. The
[causal denoiser benchmark](https://github.com/facebookresearch/denoiser) also
demonstrates a low-latency streaming speech-enhancement path. Their results do
not establish Mimi device performance, improved ASR accuracy, reliable singing
recognition or separation of multiple simultaneous speakers.

Speech activity detection marks speech intervals and does not extract a vocal
track. A new gate must not discard quiet speech, initial consonants or sentence
tails. General music-stem separation and low-latency dialogue enhancement need
separate accuracy and latency evaluation; do not promote one based on the other.

## Acceptance needed before integration

Replay fixed public human recordings covering clean speech, speech with music
and singing. Compare identical input through bypass and an enhancement candidate
with the same recognition/translation configuration. Measure:

- sample continuity and capture/send failures;
- reference WER/CER, deletions and sentence-tail retention;
- enhancement processing time, algorithmic buffering, peak memory and CPU;
- speech-onset to first source/translation, final confirmation and queue age.

A candidate must preserve clean-speech quality, improve the affected material,
and meet an explicitly measured additional-delay budget. Faster than real time
alone is insufficient. No default model, prompt, routing, recording preference
or credential path was changed for this investigation.

## Local verification

Existing audio regressions: 53 passed. Long-caption layout, reading, source
blocks and repetition display regressions: 57 passed. HQ recognition/translation
pairing, ordering, retries, cancellation and final-queue regressions: 57 passed.
The subsequent canonical check passed 970 Rust tests (2 manual tests ignored)
and 946 frontend tests across 87 files, plus strict compilation, formatting,
lint and production build. These checks include native CoreMedia buffer fixtures
but do not establish the user's original-text error rate or rule out other
capture, transport, model or presentation causes.
