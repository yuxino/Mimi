# Animation switches

The caption surfaces animate: the recognition pulse breathes with ripples, new
sentences settle in, lanes fade in, bottom-anchored text glides when a new line
pushes it up, and history ages in steps. All of it is decoration on top of state
that is also visible without motion.

Windows reports `prefers-reduced-motion: reduce` whenever "Show animations in
Windows" is off, which a machine-level tweak can set without the user knowing.
Every one of those effects then collapses to a static fallback, and the overlay
reads as abrupt rather than calm. Honoring the media query alone leaves no way
back: the tunable lives in the OS, not in the app.

Add two animation switches next to the subtitle preview: the breathing light and
the subtitle motion. Each is a stored `Option<bool>`: `None` (never touched)
follows the system's reduce-motion preference, an explicit value overrides it.
The switch shows the resolved value as a simple on/off choice. The settings UI
does not offer an unset state or describe it as a third choice. An existing
unset preference remains compatible with system reduced motion; clicking either
switch persists an explicit boolean. Status-light motion needs only its switch,
and subtitle motion has a short description of its visible effect. The overlay
resolves both once per render, passes them to `PulseRing` and
`Timeline`, and mirrors the subtitle switch onto `document.body` as
`motion-reduced` for the CSS animations. The media query stays authoritative for
the settings window's own chrome, which is not a caption surface.

The pulse moves from an rAF loop to CSS keyframes. Streaming text already keeps
the main thread busy; a per-frame JS loop competes with it, while a keyframe
animation on `transform`/`opacity` runs on the compositor thread. The rings keep
the tuned expansion and fade, staggered by negative `animation-delay`, and the
rest state is the same dot-and-rings stack the loop used to write.

## Text that arrives

Streaming text used to pop: a lane's element survives between sentences, so the
first translation of a sentence appeared between two frames and a settled draft
replaced its preview instantly. Two changes fix that without bringing back the
blink that made committing feel like a flash:

- While a row is live, its text is emitted as one element per unit (a word where
  the script has them, a CJK word or a punctuation mark otherwise), keyed by the
  unit's offset in the text. A unit that is already on screen keeps its element
  and its identity as more text arrives, so the CSS fade runs exactly once per
  unit and never replays; a corrected word updates in place instead of blinking.
  Settled rows render plain text, so no wrappers survive the stream.
- A three-dot typing wave rides the end of the lane that is still arriving, with
  dots lifting and brightening in sequence (the staggered-dot loader pattern from
  the MIT `three-dots` / SpinKit families, written for inline caption text).

Committing a sentence keeps its row in place: the live row becomes the committed
row under the same visual position, and only a genuinely new sentence animates
in. The typing wave also requires a working session phase, so a paused session
keeps its frozen draft but stops claiming that text is still coming.

Review: the switches belong with the other subtitle presentation controls rather
than behind a hidden gesture, and they read as two separate decisions because
they are: a static indicator over moving text, or the reverse, is a legitimate
choice. Off must stay readable: no information lives in the motion, so a static
overlay is a complete overlay.
