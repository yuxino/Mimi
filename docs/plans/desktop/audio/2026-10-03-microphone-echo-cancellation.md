# Microphone acoustic echo cancellation

## Requested behavior

When System audio and Microphone are enabled together, use captured system
audio as the render reference for acoustic echo cancellation (AEC). Send the
system source unchanged. Process microphone PCM before recognition and before
optional microphone recording, retaining the user's independent speech while
reducing speaker playback picked up by the microphone. This is audio processing,
not transcript deduplication or a rule that mutes the microphone whenever system
sound is present.

AEC is automatic for Both. Single-source modes bypass it, and microphone-only
mode must not silently open system capture or request its permission. The
existing compact help explains when AEC applies and its limits; no persistent
small-print paragraph or new row of controls is needed. Headphones remain useful
for difficult acoustic conditions. No claim of complete echo removal is made.

## Algorithm and dependency

Use SpeexDSP's linear adaptive echo canceller through pinned `aec-rs-sys`
1.0.0. Do not enable nonlinear residual suppression, noise suppression, or
automatic gain control. The checked core wrapper supports Mimi's mono 16 kHz
and 24 kHz PCM16 in exact 10 ms frames, sets the echo state's sampling rate,
and owns/destroys its native state. A 500 ms filter covers the tested acoustic
paths and the reference timing bias below; this is not a promise of arbitrary
room or device delay support. The state is movable between threads but is never
shared for concurrent calls.

This adds a local C library, CMake and libclang build requirements, but no runtime
service, model download, or credential. Rust's declared minimum remains 1.88.
SpeexDSP is linked statically; keep the wrapper's MIT and SpeexDSP's BSD notices
in `THIRD_PARTY_NOTICES.md`, included in application resources. Add Linux build
dependencies to the shared CI/release setup and verify Windows ARM64 explicitly.

The isolated selection probe compared SpeexDSP and two Rust AEC3 ports with the
same deterministic fixtures. Full nonlinear AEC3 suppressed independent near-end
speech substantially in these tests, including a separate generated-speech
probe. Linear SpeexDSP preserved near-end gain while reducing echo. AEC3's
linear-only output also performed better, but its public output is fixed at
16 kHz and would require another rate-conversion path for Mimi's 24 kHz services.
These are observations on the tested configurations, not general claims about
all WebRTC implementations.

Sources: [Speex echo API](https://www.speex.org/docs/api/speex-api-reference/group__SpeexEchoState.html),
[pinned bindings](https://docs.rs/aec-rs-sys/1.0.0/aec_rs_sys/).

## Audio and lifecycle boundaries

Place a shared processing worker after native platform resampling and before
provider send queues. Native callbacks only perform bounded, nonblocking enqueue
work. System upload and render-reference delivery must not wait for microphone
network sends. Keep raw capture activity for the UI; cancelling echo must not
make an active microphone appear to have stopped capturing.

Reassemble arbitrary callback buffers into exact 10 ms frames. Preserve sample
order and duration, including a bounded partial frame at finish. Reference and
microphone buffering must be limited by audio duration as well as message count;
overflow closes the attempt with a sanitized error instead of accumulating
latency or joining disconnected portions of audio. Pending-PCM guards include
processing and partial-frame work until it reaches the provider queue.

System capture may begin seconds before the microphone connection. Do not align
the first frame from each source as if they were simultaneous. Place packets on
a monotonic timeline using callback time and sample count. This is an estimate,
not native hardware clock synchronization. Keep 1 second of bounded reference
history. Process microphone frames after a 350 ms jitter window and select a
reference 100 ms ahead on that timeline, covering reference callbacks batching
up to 250 ms and keeping the tested capture-time offsets causal for the filter.
Wait at most one additional 10 ms frame before publishing matured DSP output,
coalescing normal output into 20 ms packets. A lone tail still has this finite
deadline and explicit finish bypasses the wait. This preserves PCM order and
keeps the existing 20-message provider queue from shrinking to roughly 200 ms
merely because DSP uses 10 ms frames. A fixed 300 ms send-stall regression
previously passed for 20 ms microphone-only capture and failed for dual input;
both must now preserve all accepted PCM through the same stall.
This adds about 350–360 ms to the microphone path when both sources are enabled;
system forwarding and single-source modes do not acquire that fixed delay.

Keep input queues bounded to 500 ms per source and the microphone processing
buffer to 700 ms, including its jitter window and incoming batches. Missing
reference resets the learned filter and uses a zero reference, retaining
microphone speech rather than subtracting an old tail or repeating stale playback.
Reference discontinuities have separate epochs. Test positive and negative
callback-time offsets, reversed arrival order, and 64/100/250 ms batches;
unknown device routing, long-running hardware-clock drift, and delays outside
these bounds still need real-device acceptance. The 100 ms bias is a processing
alignment choice, not a measured room delay.

Each connection attempt owns its processor, including retries within the same
generation. Source changes, pause/resume, reconnect, stop, permission failure,
and startup rollback retire its ingress and processing state. A late callback
cannot modify a later processor. Audio failures carry an attempt nonce checked
atomically with generation ownership, so errors delayed across a retry within
the same generation cannot tear down its replacement. Graceful stop seals input,
stops capture,
finishes bounded DSP work, then drains existing provider queues. Cancellation
aborts all queued work and releases its guards. Clearing subtitles alone does
not erase the learned acoustic path. All reference audio stays in bounded
working memory and is never written to diagnostics or session files.

## Verification

Use deterministic generated signals, not user recordings: delayed and filtered
echo, near-end speech-like signals, independent simultaneous speech, changed
echo paths, render-only startup, missing reference, varied callback sizes and
arrival order, and 16 kHz/24 kHz. Measure echo reduction together with near-end
gain and error against the clean near-end reference. Demonstrate that passthrough,
all-silence, and uniform attenuation cannot satisfy the complete quality checks.
Keep the algorithm's fixed processing delay explicit in those measurements.

The checked 500 ms core filter is tested at both rates with independent voiced
and broadband fixtures, delays of 40/80/180/280 ms, and a reflection tail up to
130 ms. The 10–18 second scoring window measured 24.62–32.14 dB echo reduction.
Independent near-end gain during double talk stayed within 0.18 dB of the dry
signal and within 0.022 dB of the processed near-only baseline. The latter
baseline accounts for SpeexDSP's fixed DC filter: it must not be confused with
signal-dependent suppression. The weakest dry-reference SDR was 11.88 dB; it
is retained in test diagnostics, while the corresponding processed-baseline
SDR was 14.50 dB and improvement over the raw microphone mixture was 11.17 dB.

Regression gates require echo reduction above 20 dB, double-talk gain within
0.5 dB of dry speech, improvement above 10 dB, and processed-baseline SDR above
14 dB with gain within 0.2 dB, including the first half second of near speech.
Negative controls reject silence, uniform attenuation, and passthrough. These
are reproducible fixture gates, not guaranteed scores for arbitrary speech:
longer isolated voiced probes showed residual-error variation, even while
near-end gain remained stable. Additional generated-speech probes supported
near-end preservation; no private or physical microphone recordings were used.

Integration tests cover bounded queues, pending-PCM lifetime, unchanged system
output, network independence, partial-frame finishing, cancellation, and stale
attempt isolation. Run the repository checks and native-platform CI, including
Windows ARM64 and the existing minimum Rust version. Synthetic tests prove only
their tested conditions; physical speaker/microphone and cloud recognition
acceptance must be reported separately.

Deliver with PR #108 for review. Do not merge main or publish a release.
