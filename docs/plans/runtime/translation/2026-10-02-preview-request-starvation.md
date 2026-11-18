# Complete started previews during fast speech

## Observed defect

The signed `8a530d8` fast-speech measurement window lasted 31.36 seconds.
Content-free diagnostics recorded 53 preview schedules, 12 preview HTTP starts,
one completed preview (290 ms), one completed final (574 ms), and no HTTP 429.
The next complete subtitle pair arrived about 27.43 seconds after that first
preview. Several newer candidates were scheduled only 93–283 ms after their
preceding HTTP start. The old scheduler cancelled every changed candidate,
including an already-started request, so rapidly changing cumulative ASR drafts
could prevent complete previews from arriving. The old diagnostics did not
record cancellation explicitly; incomplete requests are not counted as proven
cancellations in the measurement report.

## Accepted behavior

- A waiting preview remains replaceable before its HTTP attempt starts.
- A started preview may complete while drafts from that same utterance change.
  Its source and complete translation remain an atomic pair; it never borrows
  the newest source text for an older translation.
- Keep only one optional pending revision marker. Its text is read from the
  existing bounded ASR committer when the preceding preview succeeds. Later
  draft updates replace that bounded text, including updates not yet delivered
  by a stable/max-wait timer. There is no text queue or second concurrent HTTP
  request. Returning to the active candidate clears the pending marker.
- Successful cleanup starts at most the latest successor, using the same pacer,
  cooldown, token estimate, deadline and owner checks. Consumed request budget
  is never refunded.
- Server finals, session finish, stop, reset/reconnect and same-language bypass
  immediately clear the marker and cancel the active preview. Existing durable
  final FIFO and the recognizer sentence-ID replay guard remain unchanged.
- A failed preview discards its pending marker and does not automatically
  repeat. A subsequent ASR update can schedule normally, subject to existing
  cooldown/preview suppression; authentication failure remains terminal.
- Fixed diagnostic labels record queued replacements, discarded markers, and
  cancellation reason/stage. They contain only booleans, numeric revisions and
  allowlisted labels, never subtitle text, provider bodies or credentials.

## Verification boundary

Local delayed-HTTP fixtures verify completion of the first atomic pair and only
the latest successor, no concurrent request, a newer untimed draft at handoff,
waiting-slot replacement, cosmetic/reverted updates, final/stop peer closure,
late-event rejection, and queued-work cleanup after 401/429. Existing HQ tests
continue to cover FIFO finals, bounded retry/overflow, cooldown continuity,
source IDs and same-language bypass. Installed fast-speech latency and visible
stability require the subsequent signed native build; the existing `8a530d8`
session cannot establish acceptance of this source change.
