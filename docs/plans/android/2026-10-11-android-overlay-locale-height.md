# Android caption placement, direct styling and interface language

The waiting overlay can disagree with the app's selected interface language.
The user also requested one adjustable reading panel instead of separate
expanded/collapsed states, direct styling of the real captions and left/center/
right text alignment.

Service-owned strings resolve the app-selected locale explicitly, including
API 33+ where Android persists application locales. A display/window/service
context is not authoritative. Following System resolves actual system locales.
Refresh provider labels and accessibility descriptions without recreating the
caption window or losing subtitle state and reading position.

A session opens one interactive panel unless immersive mode is enabled. Its
top handle moves the window; its bottom-right handle resizes width and height
together. The same caption controls offer width (55–100%) and height (20–70%
of the safe viewport). Save completed resize bounds atomically. Preserve legacy
normal/immersive position keys to retain existing preferences. Safe viewport
and minimum reading/control bounds clamp the displayed frame without rewriting
preferences. History scrolls inside the selected height rather than enlarging
the window. No expand/collapse controls remain.

Center the top drag mark and language route against the whole panel, reserving
equal space around the route for the right-side settings action. Center the
provider group at the bottom, with immersion at the left and resizing at the
right. If complete provider names cannot fit between symmetric action margins,
give the provider group its own centered row above the two actions. Bound long
names with an ellipsis while retaining their complete accessible description.
Measure the immersion action's wrapped height at its final assigned width so
locale changes and large system fonts do not clip its label.
Omit the immersion icon when it would squeeze the visible action label.
The floating identity shows service brands and marks, deduplicating a built-in
recognition/translation provider. Independent services keep both identities.
Tapping it opens Services in the existing main activity; model details stay
with service configuration instead of occupying subtitle space.

Initialize surface color and opacity from saved settings before the first native
frame, using the same paint path as live updates. Font changes must preserve the
surface and rendered background pixels, including a round-trip through 100%
opacity. Check nondefault saved values rather than only the preview defaults.

The floating quick-settings dialog contains controls only. Font, color, width,
height, opacity, display mode and alignment update the existing caption window,
including the listening placeholder. Temporarily fit that window above the
bottom editor and hide its duplicate actions. Dismissal restores those actions
and the saved placement. This temporary fitting never writes position or size.
The main Captions page keeps its single pinned preview beside scrollable styles.

Text alignment applies to source, translation, history, waiting text, the main
preview and immersive captions. Center matches the desktop default. Immersive
mode keeps its touch-through captions and adjacent 48dp restore target; exit
returns to the same normal panel with its saved width, height and position.
Its content determines height, so disable the normal-panel height control there.

Native regression covers explicit foreign-resource contexts, all eight locales
and System, normal waiting/reading, stable history height, actual resize/drag,
live edits without a duplicate preview, all alignments and repeated immersion
restoration. Include narrow and wide screens, large fonts, own-page hiding,
single/independent provider badges and unchanged private preferences. These
synthetic checks start no audio capture or provider requests; record device and
provider limits separately in the run ledger.
