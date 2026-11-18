# Explicit return to live subtitles

## Decision

Add a visible “Back to live” action while the user reads complete subtitles.
Keep the existing atomic original/translation pairs and measured visual-line
budgets. Do not zip paragraphs into sentence pairs by matching sentence counts:
translators may merge, divide, or reorder sentences even when both sides contain
the same number. Successful punctuation segmentation does not establish semantic
alignment. No unused sentence helper or prototype is included in production.

The independently cropped tails of a long bilingual paragraph can refer to
different parts of that paragraph. This reading affordance does not repair a
stale or incorrectly translated backend pair; those defects require their own
confirmed-identity, translation-memory, and clear-boundary fixes.

## Interaction and state

Upward wheel/key/touch intent opens full confirmed and current live text and reports reading
state to the overlay. Incoming content, pause/resume, display-mode changes, and
resize retain that deliberate reading position. Previously only End or scrolling
back to the bottom resumed live following.

The new action increments a controlled follow-tail request. Timeline responds
through its existing `TimelineScroll.followTail` API and returns its lane
presentation to compact following before paint. Reading state is stamped with
the request, so a request is applied once without timer work or a second command
on a later render. A new upward gesture can enter reading again. End and normal
scroll-to-bottom behavior remain available.

The action is outside the timeline's direct children: canonical subtitle-block
IDs and reading anchors remain intact. Clear removes empty timeline content,
cleans up the reading callback, and hides the action immediately. A subsequently
mounted session starts in live following. No preview is promoted to confirmed
history; snapshots and saved session files are unchanged.

The current live row also opens on deliberate reading, even when no final has
arrived. Its earlier content must not remain trapped behind a finite lane while
the return action is visible. Default following still clips to measured whole
lines. A live row is the latest replaceable snapshot, bounded by the existing
64KiB-per-field limit; this does not archive previous draft revisions. Recognition
corrections can change its text while the reader keeps a block/pixel anchor.
Final promotion changes the canonical row ID, so the existing missing-anchor
policy preserves the current scroll position rather than forcing the tail.

An identified live row uses the actual projected utterance's opaque owner.
An earlier final A can insert above an unchanged complete B preview while raw
recognition is already C. Keeping B's owner preserves its DOM and read anchor
through that insertion; borrowing C's raw owner would identify the wrong pair.
The backend supplies a private layout epoch plus actual server source ID and
changes that epoch at creation, clear and transient reset. The UI treats the
value as opaque. Original mode uses its raw source owner; translated modes use
the owner of their complete pair. A new actual owner gets a different key even
when its words repeat. The stabilizer uses the same projected lane identity.

Legacy snapshots without an owner retain a layout key containing the latest
canonical confirmed timestamp. Without this epoch, a final and new live row arriving in one snapshot reused
the key `live`, so restoring the former live anchor moved the reader to the
new sentence. On confirmation the former key is missing instead, preserving
the current pixel position; subsequent native scroll notifications capture
the confirmed row actually visible there. No text matching or arbitrary
final-to-preview pairing is used. This fallback is presentation identity only;
it does not claim to identify the live utterance. Earlier-history
eviction, display-mode changes and draft revisions keep the same epoch while
the newest confirmed timestamp stays the same. Clear unmounts the empty
timeline and the next session starts in following state. A new live key may
run the existing position-only entrance; confirmed rows never use that
entrance class, so the row being read does not acquire a fade on promotion.

Returning before compact lanes finish measuring can leave the scroll position
above the eventual tail. Observe the fixed viewport and its direct subtitle rows,
then use the existing reflow policy when row height changes: following repins to
the current bottom; reading restores its anchor. Rebind row observations only
when block IDs change, not for every draft. Explicit return intent runs after a
same-render mode change, so the mode anchor cannot override that action. No timer
or forced jump while deliberately reading is added.

Native scroll notifications are not themselves return-to-live intent. A final
can replace the live row ID, change its height, and produce a delayed scroll
at the newly clamped bottom without any user action. Every programmatic
restoration clears input authorization, including a missing reading anchor;
restoration never re-arms it. Upward reading input also does not authorize
returning. Scroll input is consumed once, with real pointer movements renewing
a scrollbar drag. Passive notifications can refresh the visible canonical
anchor while preserving reading. Only a new downward gesture, End, or the
explicit return action can resume following; no timer guesses user activity.

Home is an explicit reading-at-start intent, rather than an ordinary upward
gesture. It moves to zero and keeps that target through the compact-to-full
transition and later row measurements; restoring the former compact sentence
offset would otherwise move the first Home into the middle of the first row.
New wheel, touch, pointer or reading-key input releases that target and returns
to the existing sentence-anchor policy. End and the visible return action still
restore live following. The focusable timeline uses a thin neutral keyboard-only
outline instead of WebKit's default focus ring, with a forced-colors fallback.

## Geometry and copy

Use the existing status band with a small neutral text button on its right.
Timing or action-error feedback flexes into the remaining width and is still
available. The active band remains 61px high; its status row starts at 47px and
is 14px high, below the native language capsule and above the subtitle body.
This preserves the 51px bilingual body budget at the native 360×136 minimum.
The button has a visible focus outline and ordinary hover/pressed affordances.
Idle history uses the existing smaller band's status position.

Copy is synchronized in Chinese, English, and Japanese: “回到实时”, “Back to
live”, and “リアルタイムへ”. There is no automatic jump on resume and no new mode.

## Verification

Synthetic component regressions cover explicit return to the tail and continued
following, later upward reading intent, reading retained through all display
modes and pause/resume, clear/new-session reset, and coexistence of the action
with timings and failed control feedback at the native minimum in three UI
languages. Private acceptance audio/captions are not fixtures. Native acceptance
must still check the actual small-window text geometry, focus/click reachability,
and slow-speech paired subtitles; component checks do not replace those checks.
Long-live fixtures also cover all three display modes, full text before a final,
equal-length rewraps, continued growth, explicit return to compact following,
and live-to-final promotion while retaining reading intent and scroll position.
Size-change regressions exercise delayed compact-row growth/shrink, replacing a
row with the same block count, and preservation of deliberate reading during
those notifications. This models the native post-measurement scrollHeight change
without claiming that mocked component geometry replaces native verification.
Final/geometry fixtures dispatch the later native-style scroll after the live
ID disappears and after a row is clamped to its bottom in all three modes.
They assert that full reading and the return action remain until fresh downward
input, and that a stationary pointer click cannot implicitly resume following.
Production-model fixtures also confirm a live row while adding the next live
row in the same snapshot, with revised final wording, in all three modes. They
keep the viewport on the confirmed row, retain its anchor when older history
is evicted, and resume compact following only on an explicit return request.
Identified fixtures also insert a late earlier final above an unchanged live
owner in all three modes, retaining the same row and reading offset. Projection
fixtures keep the paired B owner separate from newer raw C recognition, while
the legacy final-A/new-B epoch regressions remain intact.
