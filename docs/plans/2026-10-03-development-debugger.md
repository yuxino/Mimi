# Development debugger and subtitle evidence baseline

The debugger comes first. Subtitle behavior is changed only after its trace can
distinguish provider output, backend admission/reduction, snapshot delivery,
frontend selection/stabilization, and committed DOM/clipping.

## Development surface

- Only an isolated development application can enable collection. Collection
  is off on launch and is controlled from Settings → Diagnostics.
- A bounded in-memory journal records numbered events against one monotonic
  clock. Starting a trace resets the journal; stopping preserves it for export.
  Evicted entries and dropped frontend observations are reported explicitly.
- Existing content-free pipeline diagnostics provide capture/send gaps,
  recognition boundaries, preview scheduling/replacement/cancellation,
  translation requests/deadlines/backoff and queue pressure.
- Provider events add source identity, connection generation, clear revision,
  controlled event kind, numeric utterance IDs where available, character
  counts, and an explicit admission or rejection reason.
- Backend application records before/after bounded subtitle summaries and
  whether the snapshot changed, including same-length replacements. Numbered
  snapshots join publication to per-window receipt/store application and DOM
  commit. DOM commit is not proof of native on-screen visibility.
- Frontend observations use typed fields, a bounded asynchronous batch queue,
  and commit effects. Raw, selected, stable, displayed and clipped character
  counts are distinct. The debugger must never wait in the subtitle path.

## Content and evidence

The metadata journal contains no recognized/translated text, audio, credentials,
endpoints, arbitrary provider errors, profile names, or text hashes. The live
inspector reads only the existing bounded subtitle display state. A separate,
explicit development recording opt-in saves bounded subtitle snapshots and PCM
decoded at the actual audio socket-send boundary into private local case files.
System and microphone evidence remain separate. Existing product recording
preferences remain off by default and are unaffected. Metadata-only tracing
does not enable content recording. No raw protocol payload is logged.
The private case also retains allowlisted provider text and constructed
independent text-translation HTTP bodies. Headers, URLs, credentials and
arbitrary service errors are excluded. Recognition, request and result pairing
can therefore be inspected without inferring semantics from character counts.
Input/output and the configured route are observable; a
model's internal reasoning is not.

The JSON export includes schema version, app/build identity, route and limits,
ordered trace events and loss counters. AI analysis must first check evidence
loss, locate the earliest divergent stage and cite event IDs, then formulate a
reproduction and a focused regression. A trace alone cannot establish semantic
accuracy or that the user saw the complete DOM text.

Each case has an immutable ID. Start, stop, read, reopen and export operations
share an operation lease across asynchronous work. Stop seals the writers,
flushes window observations, finalizes audio and persists the complete report.
Audio reads and exports use the selected case's files, including after restart.
Normal quit stops the subtitle session and seals an active private case before
exit, using the same bounded flush and save path as the recording stop button.
All evidence uses one monotonic origin; frontend receipt timestamps include
bounded batching delay. Limits and loss counters are visible in the inspector.

## Subtitle acceptance baseline (second phase)

1. Drafts may be replaced only by work belonging to their current owner.
2. Confirmed finals remain durable within the documented display-history bound;
   new drafts, empty responses, timeout, pause and reconnect do not erase them.
3. Out-of-order final translations retain source/result pairing and ordering.
4. Clear, new session and source changes have explicit generation/revision
   boundaries. Stale work cannot restore cleared content.
5. Original, translated and dual display modes have explicit projection rules.
   Stabilization never reintroduces an invalidated preview.
6. Compact clipping, repetition folding and retention are visible decisions,
   separately measured from actual provider/backend loss.

Validate collection bounds/privacy/gating, rejection reasons, frontend batching
and real signed dev UI. Run the canonical repository check. Use non-sensitive
speech and explicit transcript/audio opt-in for real-provider experiments;
keep native permissions and credentials outside exported diagnostics.
