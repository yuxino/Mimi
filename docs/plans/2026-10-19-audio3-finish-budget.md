# Audio3 bounded finish and tail drain

## Scope

Baseline: clean main `632166a5c37152f9d8a19ab6f22fbedb851ab089`.
Issue #280 motivates checking the recognition stop boundary, but its external
worker measurements do not establish a current Mimi regression. A recognizer
reply arriving after an intentional deadline alone is not a bug.

Use synthetic loopback traffic only. Do not launch or restart installed apps,
capture or play audio, read private configuration, or call real providers.
The user questioned the complexity of additional silence notifications. Keep
the existing UI: silence cannot reliably distinguish paused playback, quiet
speech and a failed endpoint. Known capture errors retain their existing path.

## Boundary to verify

Audio3 currently starts its recognition deadline after sending `finish-task`.
That send has a separate five-second budget. A missing `task-finished` then
leaves the high-quality ASR bridge waiting for an acknowledgement that cannot
arrive after transport teardown. This can consume time intended for final
translation and tail fallback under the outer provider deadline.

Keep the existing recognition and provider policy constants. Charge the finish
send and provider wait to one recognition budget. On a normal local timeout,
stop the old producer before enqueueing a local finish barrier, preserving the
receiver's existing latest-draft/final ordering. Use the original connection's
generation and sender. Do not issue a successful finish after a known error,
failed finish send, or an already-disconnected call. Ordinary disconnect and
cancellation remain separate; do not affect a replacement connection.
Socket-close I/O retains its separate bound.

Prefer these contained changes to extending every service timeout or creating
another status/event system. Keep portable behavior in `mimi-runtime` so the
desktop and Android production engine use the same implementation.

## Verification

First demonstrate the unwanted extra wait and finish-send budget overrun with
controlled tests on the original implementation, then rerun the same cases.
Cover normal acknowledgement, no acknowledgement, delayed tail, authoritative
final versus draft, empty-result compatibility, errors, cancellation and connection
replacement. Reuse existing Clear, cooldown and bridge-ordering regressions.

`audio3FinishDrain` in `shared/translation-contracts.json` provides synthetic
tail expectations for direct Rust and actual JNI loopback tests. JNI checks
exercise the production protocol and state adapter; controlled Rust checks
establish scheduling/deadline behavior. Neither proves acoustic completeness
or acceptable latency for the private local worker described in #280.
Audio3 ignores empty recognition results; preserve this existing protocol rule
instead of importing OpenAI transcription's authoritative-empty semantics.

Run focused shared checks and cross-platform/Android CI before integration.
Record exact revisions, results and untested provider/native behavior in the
integration ledger. No new dependency, model, setting or interface is required.
