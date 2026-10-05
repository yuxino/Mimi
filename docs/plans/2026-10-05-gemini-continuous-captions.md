# Gemini continuous captions and planned connection rotation

The current overlay projection already selects realtime drafts after a retained
final pair (see `2026-10-05-gemini-subtitle-progress.md`). Keep that behavior and
its identity checks. This change addresses the remaining Gemini-specific text
assembly, presentation cadence, caption checkpoints and planned GoAway handling.

## Transcript assembly and presentation

Keep Gemini assembly in shared Rust, used directly by desktop and through the
actual Android JNI adapter. Each nonempty changing chunk publishes its draft
before any local confirmation. A strict extension of the entire pending lane
is a cumulative snapshot; replace the lane. Otherwise append the fragment
verbatim. Ignore an equal whole snapshot only after observing cumulative
extensions in that lane. Matching tails alone do not prove duplication; other
fragments retain their repetition, subword boundaries and provider whitespace.
Source and translation keep independent buffers. Without delta/snapshot metadata,
a genuine fragment that strictly extends the whole pending text is also ambiguous
and this heuristic can treat it as a cumulative extension. It does not guarantee
all repetitions or reconstruct arbitrary non-prefix revisions.

Two seconds without changing text can checkpoint meaningful paired lanes even
without sentence punctuation. Unmatched text remains live. This is a local
caption boundary, not provider-final or reliable utterance alignment. A local
checkpoint retains the prefix of an observed cumulative lane so a later snapshot
only publishes and confirms its new suffix; equal resends remain ignored. The
retained prefix and pending text share the existing 5,120-character bound per
lane. Explicit turns keep the existing 500 ms late-tail grace, then clear this
baseline even when no text remains; finish, interruptions and reset also clear
it, allowing genuine repeated captions in a new turn. OpenAI delta behavior
remains unchanged.

Gemini translation presentation coalesces for 100 ms with a non-resetting
250 ms maximum; source keeps 180/750 ms. Other providers keep their existing
preview clocks and projection. Sentence breaks are display-only and apply to
Gemini live blocks; raw buffers and committed history never receive inserted
newlines. Preserve existing clipping, source identity and history behavior.

## Desktop planned GoAway rotation

Prepare one replacement WebSocket with the existing configuration while the
old connection continues audio and transcript delivery. Preparation is bounded
at 10 seconds. Flush/pad the old partial 100 ms frame and send audioStreamEnd,
then stage arriving PCM by bytes, at most two seconds at 16 kHz mono PCM16.
The old transcript drain ends after 500 ms quiet or a 1,500 ms deadline. Finish
the old local paired buffer, install the replacement, and send staged frames
in order. Only one connection receives input audio at a time. No resumption
handle or replay is introduced; native capture and the provider generation
continue. Failed rotation falls back to existing bounded SessionManager
transport recovery, and upstream authentication classification remains intact.

Stop during preparation retains the old closing path; Stop during handoff
waits for the staged audio switch before ending the replacement input. Clear
and retired generations cannot confirm old text. Cancellation releases handoff
waiters. A byte cap is required because actual native callbacks can be 20 ms;
a fixed count of callbacks would not represent two seconds of PCM.

A finite old-tail drain may omit later semantic output. Replacement readiness
and first subsequent transcript must be measured separately. This does not
promise lossless, seamless translation. Android shares transcript semantics,
but its native planned WebSocket handoff is outside this change.

## Verification

Use synthetic shared fixtures to assert drafts as well as finals through Rust
and actual JNI. Exercise the real overlay selector, stabilizer and Timeline
across post-final drafts, continuous updates, input sources, hidden previews,
Clear and durable history. Desktop loopback tests cover PCM byte order through
AudioSendPipeline, late old transcripts, Stop during both phases, Clear,
retired-generation cancellation and staging limits. Keep real-provider evidence
and verification limits in the concise integration run ledger; private audio,
transcripts, credentials and benchmark harnesses stay outside Git.
