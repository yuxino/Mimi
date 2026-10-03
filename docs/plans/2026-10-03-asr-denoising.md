# Recognition-first denoising evaluation

The user explicitly wants noise reduction before recognition to improve the
original text, independently of translation. General vocal-stem separation is
not required. Prioritize recognition completeness over sounding cleaner.

## First candidate

Reuse the already linked SpeexDSP preprocessor in the test-only ASR benchmark.
Evaluate mild suppression (-12 dB), with VAD, AGC and dereverberation disabled.
Every input sample remains represented: no speech gate, silence dropping,
normalization or changed recognizer prompt/model. Use mono PCM16 / 16 kHz and
20 ms frames, with one owned native state per clip. Flush overlap at EOF and
compensate its startup delay for fixed-PCM comparisons, preserving exact sample
count and sentence tails. Report algorithmic delay separately from frame CPU
time and real ASR response timing.

This introduces no dependency, new service, production setting or credential
path. The candidate is not compiled into the application. The existing paid
benchmark's explicit local-dev-file credential gate remains unchanged.

The pinned fixed-point SpeexDSP build excludes AGC. The candidate checks that
native VAD is off and ignores its frame return value. Tail/impulse fixtures
verify delay compensation with denoising disabled so energy attenuation cannot
hide a sample-alignment error.

## Comparisons and promotion gate

Run local continuity and CPU checks first on the existing public human corpus.
Prepare bypass/candidate runs using identical source PCM, account, source hint,
recognizer configuration and two-second tail. Label processing in every ASR
report so filtered and raw audio cannot be conflated. Ordinary benchmark runs
remain bypass. Evaluate the winning comparison again in reversed order.

Acceptance requires reference WER/CER and deletions, quiet speech / initial and
final word retention, clean-speech non-regression, noisy-dialogue gains and
measured added latency. Energy reduction is not evidence of recognition gains.
Only after real ASR acceptance should the candidate be wired into bounded,
generation-owned capture processing and exposed as a reversible setting.

If this inexpensive candidate does not improve recognition, evaluate a causal
neural speech enhancer such as DeepFilterNet with documented model/runtime cost.
No denoiser is promoted solely because it is faster than real time. Android
runtime/UI parity must be documented when production integration is proposed;
this evaluation changes neither desktop nor Android provider behavior.

## Neural candidate and controls

Evaluate the official DeepFilterNet 0.5.6 Apple Silicon CLI with its embedded
default model outside the application. Keep post-filter disabled. Compare a
12 dB attenuation limit first and full suppression separately; stronger
suppression is not presumed to improve ASR. No model/runtime dependency is added
to Mimi during this evaluation.

The CLI requires 48 kHz PCM. Prepare a 16-to-48-to-16 kHz polyphase-resampling
control alongside each denoised fixture. Initialize the model independently for
each recording. Append a 100 ms zero drain tail before processing, compensate
the CLI's 30 ms algorithmic delay, and trim only after verifying that output
contains at least the original sample count. The original and processed inputs
must have identical lengths; the benchmark then adds its usual two-second ASR
tail. Report fixture preparation separately from benchmark processing and reject
combining prepared input with Speex to avoid accidental double processing.

Numerical edit reports now distinguish substitutions, insertions and deletions.
Where minimal reference alignments tie, choose fewer deletions, then insertions.
Those counts describe reference edits, not a semantic diagnosis of unheard
speech. Neither recognized text nor reference text is written into diagnostics.

Record real-provider results, CPU measurements and unresolved acceptance limits
in [the evaluation record](../development/2026-10-03-asr-denoising-results.md).
