# Runtime performance and resource ownership

Optimize the code used by the shipped application. Provider protocols, subtitle
ordering, translation scheduling and cleanup live in the shared Rust crates;
Android tests must exercise those crates through JNI. Retired Kotlin engines
are available in Git history, not maintained as alternate test implementations.

## Bounded state and work

- Keep only bounded display state in memory. Optional saved transcripts stream
  to private files; library listings read summaries and readers request pages.
  Do not load a complete session to display its count or export it.
- System and microphone lanes keep independent capture, PCM admission, provider
  connections and subtitle ownership. Queue overflow reports recovery to the
  connection that rejected the PCM; a retired connection cannot fail its replacement.
- Copy only the snapshot projection actually published. JNI compares the caller's
  revision under the same controller lock used to publish state and advance that
  revision. An unchanged poll still carries current terminal/error metadata;
  the first poll and changed revisions deliver full snapshots before termination.
- Stable archive metadata must not trigger repeated React commits. Language,
  local day and system time-zone changes still invalidate visible date formatting,
  and a successful unchanged response must clear a previous read error.
- Memoize subtitle lanes and embedded controls at their actual ownership
  boundaries. Preserve locale updates, current text, source identity, reading
  position and the last row's measurement ownership. Closed pickers need no
  option rows; caption-only mutations need no unrelated popup measurements.
- Scope scroll and mutation observation to the affected controls. Ancestor,
  viewport, stylesheet, unknown and mixed changes still update native hit regions.
  Coalesce resize work while retaining the final requested size and safe teardown.
- Native measurement reuses child layout parameters and scratch bounds. Reset
  weights and margins when switching between rows, stacks and hidden previews;
  continue reading current display geometry after rotation or multi-window changes.
- Search input may be debounced; paging, retrying and refreshing an already
  dispatched query must not wait for the typing delay again. Show loading until
  the requested result settles, and reject results from an older query or visit.

## Cancellation and display ownership

Register teardown ownership before removing a connection from its slot or
exposing recovery. Cancelling the caller must not cancel the resource owner.
Concurrent cleanup requests join the correct generation/connection; completing
an older owner cannot erase its replacement. Hard stop aborts provider waits,
while normal stop retains bounded audio drain, final work and tail publication.
Desktop capture admission remains a native adapter concern; JNI uses the same
shared cleanup primitive. See [native lifecycle pitfalls](common-regressions.md).

Treat provider identifiers as opaque. Shared source-order evidence governs live
display and pending deadlines; late older finals may enter retained history
without replacing a newer pair or reopening its completed translation. Admit
source-final text and translation activity together. Reset identity scopes at
the existing generation/session boundaries, independently for each input.

Settings work has its own lifetime: retain only the latest unsent choice in a
coalesced queue, preserve drafts and IME ownership, and prevent old saves or
navigation callbacks from overwriting later intent. Reuse process-stable native
capabilities per WebView, keep failures retryable, and treat an iconless query
as a completed query. See [UI ownership rules](ui-guidelines.md).

## Measuring a change

Record the source revision, build profile, fixture, warm-up and measurement
boundary. Compare the same workload before and after, changing one variable.
Count commits, formatting, allocations and IPC calls to expose repeated work;
use native measurements for process memory, frame timing and session latency.
Report retained memory separately from allocated bytes and temporary peaks.

For startup, time when a control accepts native input and when its result
becomes visible. Confirm the click through the resulting native state. A paint
callback used by the probe can delay the click, especially on a virtual display,
so measure that callback separately from input readiness.

Keep behavior regressions on production code and shared fixtures. Use synthetic
loopback peers for protocol, retry, cancellation and deadline checks. An old
implementation copied into a test does not establish current-product coverage.
Cargo does not execute dependency crates' unit tests. The desktop Apple Speech
adapter harness compiles the maintained shared source with the real Swift
linkage, so its callback/backpressure/teardown regressions run on Apple silicon
without starting recognition. Keep that harness in the normal desktop suite.
Do not turn a synthetic work-count reduction into a CPU, RSS or latency claim.
Record native and service evidence with its limits in the
[integration ledger](integration-runs.md); follow the
[integration workflow](integration-learning-loop.md) for real-session work.
