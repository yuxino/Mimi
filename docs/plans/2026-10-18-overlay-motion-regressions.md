# Overlay motion and settings regression fixes

## Reading and motion ownership

New sentence following starts before layout observers run. While that glide is
in flight, measured row sizes and streaming revisions retarget the same smooth
follow instead of issuing an instant scroll. Reaching the tail, explicit history
intent, a display-mode change or disabling motion ends that ownership. A pointer
press pauses automatic movement before recording user scroll intent; release
resumes following only if no actual history scroll occurred. Release outside
the viewport and window blur must also finish the pointer interaction.

Compact-lane roll-up restores the change in clipped overflow, not the change in
full text height. Crossing from one fitting line to five lines in a three-line
viewport restores two lines, avoiding a downward jump before the glide.

PulseRing keeps its SVG nodes and CSSAnimation identities. Each style's tracks
run while visible and for 520ms after leaving, then pause. Hidden measurement
copies pause immediately. Chrome reduced-motion rules exclude the status light,
whose explicit setting already overrides the system preference. Timestamp entry
has its own opacity keyframe.

## Native geometry

Expanded geometry remains canonical through interrupted collapse/expand. Mode
revisions invalidate queued work even after returning to an earlier mode.
Native window moves promote position only; custom resize interactions own size.
An intermediate animated size must not become the next expanded target, even
when the user drags the window during that transition.

## Settings boundaries

Proxy-address blur skips only its own mode picker. A picker elsewhere still
commits the address. Snapshots received during a successful save are reconciled
after completion; failed saves keep the draft and retry. Font-family drafts
participate in the same optimistic settings merge as font size and color.

## Verification

Focused component and production geometry regressions cover these boundaries.
`scripts/fixtures/overlay-motion.html` and `verifyOverlayMotion` in
`scripts/verify-overlay-motion.js` exercise real layout, system/explicit motion
preferences across four surfaces and retained animation clocks. Use a foreground
browser and prime the Vite root once before loading the fixture. This synthetic
UI check makes no audio, credential, provider or native-window acceptance claim.
