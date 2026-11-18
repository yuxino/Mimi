# Overlay control recovery and deliberate keyboard access

Native acceptance exposed two code gaps. Secondary overlay actions discarded
rejected IPC promises, and optimistic collapse never restored its prior layout.
The macOS subtitle panel also explicitly refused key-window status, making its
DOM End/Home handlers and keyboard tooltips inaccessible through normal input.

Secondary controls now share a serial action guard. The chosen icon shows its
existing spinner and retains its action label; the other secondary controls
are disabled until it settles. Failure uses the existing localized
`controlActionFailed` text in the status row, or the single compact status line
at 54px. A new attempt clears that feedback. No raw native error is rendered.
Compact pause/resume failures use the same feedback and remain retryable.
Timing and subtitle geometry are unchanged.

Collapse remains optimistic. A rejected request restores only its own current
presentation flag, preserves newer subtitle content, and cannot roll back a
later local request. Native state events remain authoritative. The clear
pipeline and its separate generation boundary are outside this change.

macOS uses the existing language control panel's focus policy for the subtitle
panel too: `canBecomeKeyWindow = true`, `canBecomeMainWindow = false`, and
`becomesKeyOnlyIfNeeded = true`. The nonactivating style, floating level and
`orderFrontRegardless` remain; no show, hover or subtitle update calls
`set_focus` or activates Mimi. This allows a user-requested responder to receive
keyboard reading input without adding a global End binding. The independent
tracking area still supplies hover while another app owns keyboard focus.

Signed macOS acceptance confirmed deliberate body click followed by Home/End
reaches the Timeline. Both subtitle and language-control WebViews accept the
first mouse click while their nonactivating panels are inactive, so that click
also executes the chosen control instead of requiring another click. Settings
and application-activation policy remain unchanged. This builder option still
requires separate native verification of one-click clear and language controls.

Focused regressions first failed on duplicate actions, missing failure text
and missing rollback, then passed with this implementation. They cover all
five secondary controls, manual retries, compact expansion/pause failures,
new subtitle preservation and late older collapse failures. Existing hover,
three-mode and deliberate reading regressions remain required.

Signed native acceptance must separately prove click-to-focus End/Home/Tab,
hover tooltips and cursor transitions at 136px and 54px, and verify that passive
subtitle updates do not activate Mimi or leave another app's fullscreen Space.
Component tests do not establish those AppKit behaviours. Windows retains its
existing nonfocusable subtitle policy; this macOS change does not claim Windows
keyboard acceptance. Dock/menu finalization remains the existing shared quit
path, with save-failure native acceptance still separate from its unit contract.
