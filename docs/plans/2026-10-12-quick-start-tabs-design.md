# Getting started and the system tray menu

Issue #237 identified an undiscovered subtitle drag handle. Present this work in
an unmerged draft PR and leave that issue open for follow-up acceptance.

Use two tabs inside Getting started. Get started retains three setup steps;
Common actions explains moving, resizing and collapsing subtitles, then the
purpose of locking and immersive mode. Omit the general prerequisite paragraph
and the obvious pause/resume instruction. A neutral diagram identifies the top
drag bar. Explain unlocking and leaving immersive mode beside those modes.

Follow the existing settings font and neutral theme tokens. Use text tabs with a
thin selected underline and separated instruction rows. On wider windows, place
the two mode explanations side by side; stack them and the drag diagram at
narrow widths. Use tab/tabpanel semantics, one tab stop, wrapping arrow keys and
Home/End. Localize all eight desktop languages.

Only offer free local Apple speech recognition on macOS when the existing native
support query confirms availability. Unknown, failed and unavailable support
must not advertise it. Explain language resources and separately selected text
translation. Reading help must not download resources, start capture, access
credentials or save preferences.

Use the existing system tray menu for both mouse buttons. Keep its compact
start/stop action, subtitle display submenu, macOS Dock preference, settings and
quit. Remove the custom tray popup, window manager, IPC, capability, positioner
plugin and unused UI copy. Retain the overlay's own controls and all detailed
configuration in Settings. Update the guide to name the native menu and direct
locking/immersive-mode changes to Subtitles.

Verify both guide panels across languages, themes, minimum/default widths and
platform layout fixtures, including confirmed and unavailable Apple support.
Review and embed screenshots in the PR. Run the canonical automated checks.
Browser fixtures establish layout; native tray activation and subtitle gestures
require separate acceptance in the signed development app. Preserve a running
production app when its launch guard prevents that acceptance.
