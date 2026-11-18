# Windows tray and capture recovery

## Reported behavior

- The tray panel opened toward the bottom taskbar and was mostly off-screen.
- Starting subtitles briefly showed a red system-audio capture error, then the overlay disappeared.

## Root causes

1. Tray placement always used the tray icon's lower edge, which only matches a top menu bar.
2. The Windows loopback backend requested `default_input_config()` from an output endpoint. CPAL rejects that direction before it can create a WASAPI loopback stream.
3. A terminal session error is inactive, so native presentation synchronization hid the overlay immediately even though the frontend has an error state designed to explain the failure.

## Design

- Calculate tray placement from the click event's physical tray rectangle, the owning monitor, and its work area. Pick the nearest screen edge, open inward, and clamp the complete panel inside the work area. Recalculate after the window becomes visible because its final native size may arrive late.
- Keep the positioner plugin only as a defensive fallback when a platform does not provide a usable tray rectangle.
- On Windows, read the output endpoint's default format and use it to build an input stream on that same output endpoint; CPAL then enables WASAPI loopback.
- Keep an error overlay visible without treating it as an active session. Tray and shortcut actions can therefore retry, while the user can read the failure instead of seeing a disappearing window.

## Verification boundary

- Pure geometry tests cover bottom, top, left, and right taskbars, high-DPI physical coordinates, right-edge clamping, and a negative-origin secondary monitor.
- Lifecycle tests cover the distinction between error visibility and active-session behavior.
- macOS tests and the repository check protect the shared code path.
- Windows CI must prove the Windows branch compiles and its portable tests pass. A real Windows device with an active playback endpoint is still required to prove tray interaction and WASAPI loopback end to end.
