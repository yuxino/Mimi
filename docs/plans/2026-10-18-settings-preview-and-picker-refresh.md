# Keep slider previews stable and avoid repeated application queries

The font-size and subtitle-background sliders optimistically save each input.
Native settings saves broadcast before returning, so an earlier save could
replace a newer local slider value while that value was still queued. Keep a
bounded, field-level preview for these two standalone numeric drafts. A native
event merges every other field immediately. An unchanged slider field or the
currently submitted value keeps the newer local preview; a different native
value supersedes it. A matching value confirms only the latest operation, so
dragging back through an older queued value stays stable too. A response
acknowledges only its owned slider fields; per-field native revisions prevent
a pre-external-edit response from replacing a later intent's rollback baseline,
including external edits that return to their original value. A failure rolls owned fields
back to their latest confirmed native values.
When the operation still owns the latest overall revision, its full native
confirmation or rollback also clears older failed ordinary-setting previews.
An earlier operation keeps subsequent ordinary drafts intact; after an external
event, only the still-owned slider fields can merge into that newer snapshot.
Accepted slider confirmations and rollbacks also update an existing ordinary
operation's rollback baseline, so a later failed setting cannot resurrect an
already failed slider value.

The IPC has no mutation identifier. An external slider change equal to the
submitted or already-confirmed value is indistinguishable from a save echo;
while local intent is pending, its preview takes priority in that case. The
native save queue and persistence order are unchanged, including already queued
requests after an external event. There is no debounce or write coalescing.
Mixed drafts, history/recording opt-outs and credential operations keep their
existing transaction behavior. This fixes preview backtracking; it does not
claim fewer settings writes or cross-window optimistic previews.

Application enumeration distinguishes a completed query from a cached icon.
A selected application that exited or has no PNG icon is still a successful
query, so a manual refresh must not immediately start a second enumeration.
Target visit generations isolate late results, failures and loading cleanup;
failed queries can still retry. The existing macOS/Windows icon-only restoration
and Linux cached-choice behavior remain in place.

Behavioral regressions cover both sliders, partial snapshot merging, external
updates, rollback/retry and ordered privacy changes, plus empty/iconless queries,
retry and target changes. The original slider implementation failed five of
the initial seven cases; the original picker failed all six new query cases.
Focused and quick frontend checks are separate from native/browser acceptance.
