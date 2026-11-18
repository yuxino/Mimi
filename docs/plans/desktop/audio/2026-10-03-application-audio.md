# Application audio capture (#94)

The system audio lane defaults to all applications. Settings offers a separate
capture scope and a searchable running-application picker with explicit refresh.
Browsing the picker does not save a selection or start audio capture. App names
are local presentation data and never appear in support diagnostics. Existing
microphone selection remains independent, including dual-input sessions.

Changing the target requires a fully stopped session, uses the existing lifecycle
mutation guard, and clears recording even when a combined draft requests it.
The target survives profile changes and app restarts. Unavailable targets never
fall back to the system mix. A stopped session can always return to all apps.

## Native implementations

- macOS: enumerate ScreenCaptureKit applications on explicit picker interaction.
  Use an including-applications display filter for every matching bundle ID.
  Existing audio-only configuration, own-audio exclusion, bounded queues and
  main-thread stream ownership remain intact. Browser tabs belong to the same
  application. Enumeration may request the existing recording permission.
- Windows build 20348+: enumerate visible application processes and existing background audio sessions, then activate
  WASAPI process loopback in include-process-tree mode. Identity combines PID
  with process creation time, preventing PID reuse from changing the selected
  target. Reopening an application requires explicit reselection. All child
  processes of the selected process are included; another independent process
  instance is a separate target. The audio engine produces provider-rate mono
  PCM16; silent packets are zero-filled without dereferencing undefined data.
  A cancellable worker owns its COM apartment; activation uses an agile reference
  to marshal its result. Startup, packet size and per-poll work are bounded.
- Linux/Android: retain their existing playback capture. Do not expose a working
  application picker or claim parity; no virtual sinks, audio rerouting, playback
  muting or new system dependencies are introduced by this feature.

Selecting an app does not mute its playback. Applications that opt out of native
capture may remain silent; a browser tab cannot be isolated from its sibling tabs.

## Verification

Core serialization/validation/default and recording opt-in regressions; command
window/lifecycle restrictions; frontend picker search, refresh, empty/error,
unavailable, saving and active/paused states; canonical desktop check; native
Windows/macOS/Linux CI. Signed macOS UI smoke uses the canonical dev bundle.
Live app isolation and physical Windows behavior require native audio acceptance;
automated compilation and UI fixtures are not proof of those measurements.
