# Preserve hand cursors on interactive surfaces

Clickable and hover-help controls use the hand cursor consistently in settings,
the tray/control panel and the subtitle overlay. Add a zero-specificity semantic
CSS fallback so component-specific disabled, busy, input and resize behavior
continues to win. Help buttons use the same hand as the other controls.

The nonactivating macOS overlay already supplies native mouse movement and a
hand-cursor IPC. Its renderer cached a successful hand update indefinitely,
although WebKit can reset the cursor on later movement. Reassert the hand for
each delivered native movement over an enabled control. Keep at most one IPC in
flight and coalesce pending work to the newest sample. Idle success/failure must
not trigger a retry loop; native exit/hide/lock and stale-generation protection
remain authoritative. No polling or activation/focus change is added.

Native hit testing covers semantic buttons, links, disclosure summaries,
selectable options/tabs and discrete input controls, including nested icons.
Disabled fieldsets and ARIA-disabled/busy controls must not request the hand.
Text/search inputs, sliders, ordinary content and resize handles are excluded.

Validate the accepted-then-reset recovery path, bounded pending work, stale
replies after exit, and control classification. Run the repository checks and
launch the canonical signed development app. CSS/computed styles alone do not
prove that AppKit retains the cursor while another application is active.

Verification: the repository check passed 978 Rust tests (2 manual tests
ignored) and 961 frontend tests across 87 files, plus formatting, strict Clippy,
lint and production build. Browser-computed styles confirmed enabled settings
controls and help icons use `pointer`, the disabled immersive switch uses
`default`, range inputs retain `default`, and all eight overlay resize regions
retain their directional resize cursors. The signed canonical development app
was rebuilt and opened. The actual macOS pointer glyph while another app is
active has not been independently observed by this automated check.
