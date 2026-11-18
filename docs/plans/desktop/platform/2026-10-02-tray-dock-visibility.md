# Dock visibility from the tray panel

Add the existing macOS-only “Show in Dock” preference below the tray's lock
position switch. Reuse the settings label and failure message, shared
`showInDock` snapshot, and `saveSettings` action. The tray serializes this save
with its other operations; failures retain the previous confirmed value and
show the localized Dock error. Settings changes from either window update
both controls. Dynamic tray switch visuals use explicit checked classes.
The native right-click tray menu exposes the same checked preference, with
matching English, Chinese, and Japanese labels. Refresh its check mark after
changes from either WebView; native clicks toggle under the shared mutation
guard before applying and persisting the new policy.

The native command allows Dock changes from settings and the tray panel only.
Keep history retention, audio recording, capture device, and proxy changes
settings-only; combining one with a Dock change from the tray is still rejected.
Other platforms retain the native unsupported-preference rejection and do not
render the control. Reuse native Dock policy application and rollback without
reconnecting or stopping translation.

Verify the window permission boundary, existing settings/save tests, canonical
repository checks, and both toggle directions in the signed development tray
panel and native menu.
Confirm settings synchronization, visible switch state, continued overlay
visibility, and ordinary tray dismissal after each native policy change.

The canonical check passes with 803 Rust tests (one ignored) and 750 frontend
tests, strict clippy, formatting, lint, typecheck, and build. Independent review
found no further defect in the window permission boundary, shared settings
transaction, native menu synchronization, or switch rendering.

The signed canonical development app was rebuilt and opened in UI-only mode.
Both Dock directions work through the existing settings control, which remains
visible and usable after the policy changes. Direct native acceptance of the
new tray entry points is pending because the computer-use surface exposes the
app windows but not the menu-bar status icon; opening the tray panel manually
makes that window available to the same automation.
The UI-only process was then quit and the same signed development bundle was
opened in normal mode, with its settings window visible for continued use.
