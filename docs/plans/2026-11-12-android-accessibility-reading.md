# Android caption reading inputs

The native ScrollView accepts accessibility scroll actions without touch events.
The caption panel previously observed reading intent only through touch, so a
successful backward action left live following enabled. On API 35, the fixed
synthetic history case moved from offset 543 to 810 when the next caption arrived,
returning to the bottom instead of retaining the reader's position.

Keep the framework's movement, acceptance result and accessibility semantics.
A local accessibility delegate updates reading intent only after successful
backward/up or forward/down actions. Native wheel movement and the framework's
arrow/page/full scrolling methods use that same intent boundary. Observing the
scrolling methods preserves older Android's Space behavior: key dispatch can
return false even though its native page scroll succeeded. No new node actions,
focus routes or preview navigation are added. Rejected actions and consumed
wheel events without movement leave intent unchanged.

Backward input enters history. Forward input resumes live following at the
existing bottom boundary, after synchronous movement or an animation callback,
but never before a pending reading anchor has been restored. Touch direction
accumulates small moves; comparing only adjacent MOVE events discarded slow
drags that already exceeded the framework's touch slop.

Returning to live or accepting a later wheel movement retires the older native
animation's endpoint. ScrollView has no public cancellation method, so let its
scroller finish while discarding its stale writes in computeScroll. Ignore those
intermediate callbacks, then keep the current live bottom or newer reading
position. A newly accepted native scroll owns its animation; touch DOWN lets the
framework abort the old animation normally. Do not pin every unrelated draw,
inspect private framework fields or rely on animation delays to hide conflicts.

The pending reading anchor identifies a row plus its local content offset,
including a cropped TextView's internal scroll. Resolve it after the full lanes
expand. A later accepted action that moves synchronously replaces that pending
anchor with its observed position; a rejected or unmoved action cannot do so.
This preserves the same visible text when preceding rows change height. The
same-turn hidden-source mode case passes without a special coordinate fallback.

Extend the existing native continuity fixture. Fix its reading viewport rather
than its outer panel height: removed caption chrome had enlarged that viewport
and stopped the old sample from exercising its cropping assertion. Keep the
original crop/reset/touch assertions, compare the same synthetic history before
and after new captions, and retain native screenshots outside Git.
`CaptionScrollContinuityChecks` owns the blank activity/window and differential
layout checks; `CaptionReadingInputChecks` owns the input assertions and its draw
observers. Cover both accessibility action pairs, rejected actions, long lanes,
rapid opposite actions, cumulative slow dragging, an injected overlay wheel,
new captions during history reading, and focused keys in the real activity
preview. Keep diagnostic reflection and passive trace sampling out of the suite.

The independent adjustment input window must remain paint-free after keyboard
input too. Suppressing its switch in onDraw does not suppress the framework's
default focus background; disable that background only for the input-only
instance, preserving its click, focus and accessibility semantics. Preview
parity explicitly checks this focused proxy, then compares frame pixels in
matched touch mode. The fixtures restore the previous input mode on exit.

Direct native actions and injected inputs establish these reading-state boundaries.
They do not establish a full TalkBack journey, real-service subtitle quality,
or physical-device acceptance. These checks do not invoke capture controls.
The settings preview uses the same caption frame inside `CaptionPreviewStage`;
retain its current semantics
and verify that sibling separately from the interactive overlay.
