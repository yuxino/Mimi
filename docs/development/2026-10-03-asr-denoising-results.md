# Recognition-first denoising: candidates not promoted

The user wants better original-language recognition before translation. This
round evaluates noise reduction, rather than requiring music-stem separation.
It does not identify the cause of the reported native-session omissions. Neither
candidate is enabled in Mimi: the measured recognition regressions fail the
promotion gate in [the design](../plans/2026-10-03-asr-denoising.md).

## Method

Use the five public human recordings described in the
[fixed-corpus record](2026-10-02-real-audio-comparison.md). Add two synthetic-noise
conditions, selected before looking at ASR results: the first English and
Mandarin recordings mixed with deterministic Gaussian white noise at nominal
5 dB global-RMS SNR, seed 20261003. Both mixtures have zero clipped samples.
Their speech is human, but their background noise is synthetic. They do not
substitute for naturally noisy dialogue, singing or the user's failing excerpt.

Send identical-duration mono PCM16 / 16 kHz recordings through the same existing
Alibaba Audio3 3.0/context configuration, explicit per-recording language hint,
account and endpoint. No translation is called. Each input is paced at 20 ms,
with the same two-second ASR tail. Preserve bounded, content-free diagnostics;
recognized text is never saved. Reference normalization and film-reference
limitations are inherited from the corpus record.

Compare:

- Raw PCM, with benchmark processing explicitly set to `bypass`.
- SpeexDSP from the existing pinned dependency: -12 dB suppression, 20 ms frames,
  no AGC, dereverberation or VAD gate. Flush one frame of overlap and compensate
  the startup delay, preserving every fixture's original sample count.
- Official DeepFilterNet 0.5.6 Apple Silicon CLI, embedded default model, no
  post-filter, 12 dB attenuation limit. Process each clip in a separate invocation
  to reset neural state. Resample 16-to-48-to-16 kHz, append 100 ms of drain zeros,
  compensate 30 ms delay, then verify sufficient length and trim to the original
  sample count. Final prepared PCM has zero clipped samples.
- A polyphase 16-to-48-to-16 kHz control with no denoiser. This isolates the
  neural filter from its necessary input conversion. Reports label fixture
  preparation separately from in-benchmark processing.

The neural CLI was obtained from the
[official 0.5.6 release](https://github.com/Rikorose/DeepFilterNet/releases/tag/v0.5.6).
The asset was 27,877,081 bytes, local SHA-256
`4601e7f4e4c03e59a4c5b5000216ef3add3e808799cfccd95e14e83ea4611081`.
The release API did not publish an asset digest; this local hash is a reproducible
identification, not verification against a separately signed checksum. The
model, tool, resampling runtime, audio and references remain outside the repository.
No production dependency, credential path or provider contract was added.

The manual benchmark reused the explicitly authorized, feature-gated private
development-file credentials in an isolated temporary configuration. It did not
fall back to Keychain or enable that file in the user's normal app configuration.
The new temporary credential copy was removed after all service calls completed.

## Reference edit results

Entries are Levenshtein edit counts divided by reference units. English uses
words; Japanese and Mandarin use characters. Do not sum these as one shared
accuracy score. Lower is better.

| Recording | Raw | Speex mild | Resample control | DeepFilterNet mild |
| --- | ---: | ---: | ---: | ---: |
| English 1527 | 5/28 | 5/28 | 5/28 | 6/28 |
| Japanese 1527 | 21/89 | 8/89 | 8/89 | 8/89 |
| Film dialogue | 14/79 | 14/79 | 13/79 | 14/79 |
| Mandarin 1579 | 0/44 | 0/44 | 0/44 | 0/44 |
| English 1620 | 4/33 | 6/33 | 4/33 | 8/33 |
| English 1527 + 5 dB noise | 10/28 | 15/28 | 10/28 | 11/28 |
| Mandarin 1579 + 5 dB noise | 0/44 | 1/44 | 0/44 | 3/44 |

Speex was repeated before raw on the Japanese, second English and two noisy
conditions. Raw and Speex retained exactly the same counts on these repeat
comparisons. This reproduces both Speex's apparent Japanese gain and its clear
English/noise regressions; it does not establish broad generalization.

The resampling control also reached 8/89 on Japanese. Therefore this apparent
neural-denoising improvement cannot be attributed to the neural filter. The
control itself was not repeated in reverse order; resampling and provider
variability remain possible contributors. The neural candidate loses against
its resampling control on both English recordings, the film and both added-noise
conditions, and ties on the other two.

The newer neural/control reports separate substitutions, insertions and deletions:

| Recording | Control S/I/D | Neural S/I/D |
| --- | ---: | ---: |
| English 1527 | 5/0/0 | 5/1/0 |
| Japanese 1527 | 5/0/3 | 5/0/3 |
| Film dialogue | 9/1/3 | 9/2/3 |
| Mandarin 1579 | 0/0/0 | 0/0/0 |
| English 1620 | 4/0/0 | 7/1/0 |
| Noisy English | 7/3/0 | 10/1/0 |
| Noisy Mandarin | 0/0/0 | 3/0/0 |

These are deterministic minimum-reference-alignment operations, not a semantic
diagnosis of missed speech. Older raw/Speex runs predate operation counters and
only provide total edits.

## Completion and timing limits

All 32 paid clip runs received genuine task completion with no failed reports:
five raw, five Speex, four reversed Speex, four reversed raw, seven neural and
seven resampling controls. Maximum sending lateness by run was respectively
66, 22, 23, 57, 150 and 176 ms. The later runs shared a machine with other builds,
so these measurements do not certify live buffering or capture-to-visible
latency. The development app was initially idle. Its system-audio session was
observed active late in the control round; further paid experiments were stopped
without interrupting the user's session. Stronger neural suppression has not
been ASR-tested in this round.

Separate offline Speex CPU measurements on the five public recordings gave
13–14 microsecond median frame processing, 14–15 microsecond p95, and 70–191
microsecond maxima. Complete processing was 28–70 ms per recording, real-time
factor approximately 0.00165–0.00179. Its 20 ms overlap and up to 20 ms frame
assembly wait are additional buffering costs, not included in those frame times.

Independent neural invocations took 40.849 seconds total for seven clips,
including repeated model initialization. CLI processing real-time factors were
approximately 0.13–0.44 across all seven clips (0.13–0.26 for the five originals);
the noisy English condition had the largest factor. An earlier multi-file run
carried model state between recordings and was discarded; its faster wall time
is not used as acceptance evidence. Streaming model startup, per-frame jitter,
resampling buffers, peak memory and native subtitle latency were not measured.

## Decision

Reject these parameters as a universal or default recognition preprocessor.
They fail clean-speech non-regression and do not improve the two added-noise
conditions. Faster-than-real-time CPU performance and audible noise attenuation
do not override these ASR results.

Remaining acceptance needs include the user's actual failure, naturally noisy
and music-backed dialogue, quiet speech, first/last-word retention, reversed
resampling/neural comparisons and native timing. A future candidate may use
stronger or adaptive enhancement, but it needs fresh fixed-input ASR evidence
and bounded, generation-owned streaming integration before a product switch.
The current app continues to send original system audio to recognition.
