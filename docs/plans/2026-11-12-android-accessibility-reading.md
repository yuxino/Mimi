# Android accessibility caption reading

The native ScrollView accepts accessibility scroll actions without touch events.
The caption panel previously observed reading intent only through touch, so a
successful backward action left live following enabled. On API 35, the fixed
synthetic history case moved from offset 543 to 810 when the next caption arrived,
returning to the bottom instead of retaining the reader's position.

Keep the framework's movement, acceptance result and accessibility semantics.
A local accessibility delegate updates the existing reading state only after a
successful backward/up or forward/down action. Backward/up enters history;
forward/down expresses return-to-live intent and resumes at the existing bottom
boundary. Check that boundary after synchronous movement as well as through the
scroll listener, while respecting pending reading-anchor restoration. Rejected
actions leave intent unchanged. Touch and programmatic layout changes retain
their current ownership; no new subtitle state or provider behavior is needed.

Extend the existing native continuity fixture. Fix its reading viewport rather
than its outer panel height: removed caption chrome had enlarged that viewport
and stopped the old sample from exercising its cropping assertion. Keep the
original crop/reset/touch assertions, compare the same synthetic history before
and after new captions, and retain native screenshots outside Git.

Direct native actions establish this callback and reading-state boundary.
They do not establish a full TalkBack journey, real-service subtitle quality,
or physical-device acceptance. Decorative previews continue to hide their
accessibility descendants and must not gain interactive actions.
