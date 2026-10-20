# Frontend work, visibility and snapshot ownership

The review baseline is `10d726e`. This change reduces repeated presentation work
and fixes stale asynchronous settings results without changing IPC, provider
behavior, retention policy or the existing cross-page audio playback contract.

## Bounded presentation work

- Closed Select controls do not filter or construct their option rows. Moving
  repeatedly within one hovered row does not change cursor state. Keyboard
  navigation still records its intent and reveals the active row, including
  Home/End when the cursor index is unchanged.
- Tooltip scroll observation ignores element scrolls outside its anchor's
  ancestors. Window/document scroll, ancestor scroll, resize and element
  observation continue to reposition the portal.
- Language display records are immutable locale-only data. Cache each of the
  eight supported locales instead of rebuilding the complete language table
  for each property access; switching locale still resolves immediately.
- Gemini display formatting only searches closing punctuation after a period.
  Searching after every character repeatedly visited long quote runs. Source
  and translation formatting are independently memoized by text and provider;
  final confirmation and the existing draft stabilization timing are preserved.

Deterministic tests count formatter regex operations rather than imposing a
machine-dependent time limit. A temporary call to the actual formatter with
8,192 closing quotes measured 1,496 ms before and 5.9 ms after in one process;
200 deterministic Unicode/punctuation examples retained identical output. This
is a synthetic function benchmark, not end-to-end subtitle latency.

Developer React/jsdom probes with 200 Select options and 50 parent renders
reduced closed-control label reads from 10,050 to 50. Fifty same-row moves fell
from 50 commits to 2. One hundred unrelated scroll events fell from 200 tooltip
anchor measurements to zero. A Node microbenchmark of 20,000 language property
reads fell from 20,000 complete table constructions to zero after warmup
(854 ms to 11 ms in one sample). These establish avoided work, not production
CPU percentages. No bundle configuration or dependency change is included.

## Settings lifecycle

Current transcript pages belong to a local session/retention generation, not
just their item count. Equal counts after stop/start must trigger a new read;
old requests cannot refill the new view. A context change also releases the
old current page and error while hidden. Saved-history selection has its own
identity and is not invalidated by live-session transitions.

Standard document visibility suspends archive and Windows audio-device
presentation polling and refreshes immediately on return. Ordinary focus loss
does not suspend a visible window. Audio playback keeps its existing lifetime.
This does not establish which visibility events every native WebView emits.

Diagnostics results belong to the visible visit that requested them. Reentry
during an in-flight operation queues one fresh read after it completes; repeated
visits coalesce, native reads remain serial, and an earlier visit cannot replace
the new report. Existing explicit copy/issue operations retain their behavior.

## Verification

Focused component/hook regressions cover lifecycle changes, late replies,
visibility/focus, keyboard intent, work counts and final text delivery. Existing
browser fixtures pass 96 tooltip and 30 motion checks using actual components
with a synthetic native boundary. They do not access captures, providers,
credentials or user transcripts. Full frontend lint, tests and production build
are recorded in the integration ledger and PR. Native hidden-window behavior
and interactive macOS/Windows acceptance remain separate checks.
