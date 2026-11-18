# Android compact and immersive subtitles

The Android port keeps Mimi's desktop visual language: grayscale surfaces, the
existing character artwork, one primary session action, and a live subtitle
overlay. The supplied mobile reference is a floating reading window over other
content. Mimi starts with a compact subtitle; tapping it opens a translucent
reading panel with a language route, type-size control, collapse action, and
one-tap immersive entry.
The panel stays short when history is off. If history is enabled in settings,
it grows into a scrollable window for bounded confirmed subtitle pairs. There
is no extra recording or preview control in the floating panel.

Normal mode shows the current translation in a compact translucent card, with
the source line for English speech. The reading panel always shows the current
source and translation. The compact card's background opacity remains
adjustable, and users can drag it vertically. New installations use a readable
65% dark background; an existing saved value still wins. History remains off by
default and never appears until explicitly enabled in settings.

The panel or appearance switch enables immersive mode immediately without
restarting audio capture. The subtitle window then has no background, hides
history, uses a text shadow for contrast, and passes touches through. A separate
small, touchable exit control sits on the right edge. Users can drag it
vertically out of the video's controls; it remains available even when subtitles
temporarily hide. Tapping it returns to the ordinary compact card. In landscape,
the expanded reading panel caps its width at 560 dp and its height at 48% of
the screen without history, leaving the video sides operable. Android 12 and
newer reject touches passing through a fully opaque
untrusted overlay, so the immersive text window's opacity is capped at 0.8.
The app and foreground notification remain backup controls. The ordinary card
stays touchable for dragging. The compact card and expanded panel consume
touches only inside their own window bounds; the immersive subtitle text does
not consume touches. The small exit control consumes touches in its own bounds
and can be moved away from another app's controls.

Mode and appearance settings save separately from provider credentials. A
debug-only instrumentation fixture renders the actual floating window over the
emulator home screen without a service account, capture permission, or network
session. Its synthetic subtitles are only for visual checks, not evidence of
live translation. Verify compact, expanded, collapsed, history, immersive entry,
and immersive exit states on
an API 35 emulator; build, lint, and unit tests cover the Android package.
Emulator UI checks do not establish physical-device touch-through behavior or
provider translation.
