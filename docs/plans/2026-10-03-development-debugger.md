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
  Queue publication and session admission retain the same transport sequence;
  locally generated overflow controls have no fabricated sequence. Join using
  source, generation, content revision and sequence within the provider lane.
  A queue attempt alone is not proof of admission, and an intermediate draft
  absent from admission may have been superseded in its latest-value lane.
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

## Continuous metadata for private cases

The real 41-second film reproduction exceeded the 2,048-entry live ring.
Increasing that ring would only postpone evidence loss. An explicitly recorded
private case now streams each content-free metadata entry into
`trace-events.jsonl`, sharing the existing 32-item queue and 16 MiB content budget.
The 20 MiB audio cap, 4 MiB report reservation, 40 MiB case reservation and 128 MiB
global reservation remain unchanged. Individual JSONL lines are at most 512 KiB.
Production and collection-off sessions have no sink. Metadata-only collection
keeps its original memory-only behavior and does not save private display text.

Journal sequence allocation captures the fixed case target and increments an
in-flight count under its lock; callbacks serialize and try-send after releasing
the lock. Concurrent enqueue order can differ from sequence order, so persisted
rows retain exact IDs and offline analysis sorts by ID. Stop records `Stopped`,
disables and detaches the sink, waits for all assigned callbacks outside the
journal lock, then seals and joins the writer before persisting the final report.
A five-second callback timeout fails explicitly without sealing; retry can finish
the same case. New starts and case switches require the previous case finalized.

The live view still shows the newest 240 of 2,048 retained entries. Saved metadata
is readable by case-bound pages of at most 64 physical rows and exported in full.
Reports separately expose persistence availability, written rows/bytes, dropped
rows, byte-limit and storage failure. Live ring eviction is not durable loss when
the complete private-case stream exists; old cases deserialize these fields to
off and retain their original eviction warning. Tests cover default-off behavior,
capacity, fixed-target tickets, drain-timeout retry, bounded queue/line/disk loss,
pagination/export and rejection of symlinks or nonregular metadata files.

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

## Second phase: complete previews for custom recognition

The current `TranslationClient` factory routes Alibaba, DeepLX, custom
DashScope ASR and custom OpenAI ASR through `HighQualityTranslationClient`.
That pipeline publishes an identified recognition draft, an unowned raw
translation draft, and a complete preview pair carrying the source owner.
The overlay previously enabled complete-pair selection only for Alibaba and
DeepLX. Custom recognition therefore hid a completed bilingual translation
because the raw owners did not match, and translated mode could replace a
complete preview with a later short HTTP streaming prefix.

One pure, typed route helper now selects complete previews for all four factory
families in both the live overlay and debugger replay. Other realtime transports
retain their own draft rules; legacy snapshots without `previewPair` retain the
existing fallback. Confirmed history stays independent and bounded.

Regression cases reproduce both custom families before the fix and cover owned
bilingual pairs, shorter subsequent raw prefixes, original-mode recognition,
confirmed-history durability, legacy snapshots and the seven unrelated realtime
transports. These are synthetic frontend checks, not custom-service network
validation. Native geometry is a separate acceptance case.

## Second phase: native height for independent source rows

The expanded native minimum was 136px regardless of selected sources. In the
active, narrow layout, the 61px controls, 12px outside inset, 10px inner padding
and 2px border leave a 51px Timeline body. Two independent bilingual live blocks
at the default 18px font need at least 92px even with tight spacing. Tail scrolling
therefore places the first source lane above the viewport. The old 130px Timeline
fixture did not reproduce this actual native minimum.

The UI-independent core rule now reserves 85px for chrome and one block per
selected audio source. For two-source bilingual output, each block reserves
`ceil(max(12, font * .82) * 1.32) + ceil(font * 1.32) + 2 + 5` pixels. Original,
translated and target-original output reserve one full-font line plus 5px per
block. The source label sits beside the lanes and adds no vertical line. Fonts
are bounded to the supported 14–20px range; the result is rounded up to 4px and
never below 136px. Both-source bilingual minima are 172–200px, including 188px
at the default font and 200px at the maximum. A single source retains 136px:
even font 20 fits one tight bilingual block in its 51px body.

This requirement follows source, display, target-language and font preferences
through startup, native constraints, queued geometry snapshots, resize,
collapse/expand and display following. A growing requirement raises only height;
a smaller requirement permits later manual resizing without shrinking the saved
frame. Position and width remain independent, and the folded bar remains
280×54px. Including the requirement in geometry snapshots rejects older queued
reads/writes even when the existing frame is already taller than both minima.
The browser resize fallback uses the same contract fixtures as Rust.

Focused checks cover the actual 51px body before the fix, full visible source
lanes at each supported font after the fix, side labels and target-original
layout, opposite-edge resize anchoring, source changes, stale geometry,
collapse and saved/presentation frame independence. These are deterministic
layout and geometry checks; signed native verification is separate. The rule
guarantees space for the current block from each selected source, not the whole
history or arbitrary multi-line text. On work areas smaller than the computed
minimum, normal work-area fitting still has the physical display limit.

## Second phase: complete publication before stop returns

A signed dev music/effects case exposed a stop completion race. An accepted
session-finish translation changed confirmed history from zero to one row,
but the last recorded snapshot still contained zero rows. The coalesced
publisher's 60ms delay extended beyond the debugger's stop/seal boundary.
This proves a missing publication completion barrier; the sealed trace alone
does not prove that the frontend never received a later publication.

Both stop completion paths now retain the normal publication request and await
the existing immediate publisher after provider/pump draining and `did_stop`.
The lifecycle lease prevents a new session from starting during this boundary;
the content gate serializes snapshot capture, private recording and emit.
No synchronous controller mutex is held across the await. Completion confirms
the final publication attempt, not a subscriber receipt or native paint.
Any already scheduled publisher reads current state rather than a saved older
snapshot. Provider deadlines, final admission and history limits are unchanged.

A regression uses the real subtitle controller, admits an atomic stopping tail,
holds the publication content gate and verifies stop remains pending until the
idle snapshot includes that tail. The offline analyzer additionally flags a
sealed, complete trace with no delivered snapshot after its last changed
reducer. That ordered-evidence candidate keeps event/snapshot IDs and the
record-order limitation explicit. Signed native retakes validate the boundary
separately from the deterministic test.
