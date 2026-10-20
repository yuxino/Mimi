# Finish interrupted overlay resize gestures

An expanded overlay can lose pointer capture or remove its resize handles when
locking, collapsing, entering immersive mode or presenting an error. Previously
these paths could retain a native resize gesture: later hover events forwarded
`resize_move`, and an unmounted component no longer had the window listeners
needed to send `resize_end`. Browser previews also kept their local drag after
an outside release when pointer capture failed.

Use one gesture owner for both implementations: the original pointer ID,
capture element and either native ownership or browser geometry. Start only for
the primary pointer and primary button, and ignore another start while owned.
Only the owner may move or finish. An owner move without its primary button is
also a finish, covering a missing release event without resizing on hover.

Capture loss, pointer release/cancel, window blur, disabled state and component
unmount share an idempotent finish. Clear ownership before releasing capture so
the resulting capture-loss event and bubbling window release cannot send another
native end. Ordinary capture-loss events without an owner do nothing. Capture
and IPC failures must not interrupt local cleanup or leak rejected promises;
preserve immediate start/move/end command ordering and native screen coordinates.

Native review: resize completion targets the overlay only and never changes its
lock preference or click-through state. A locked overlay's final frame is not
persisted. Collapsed/error states clear resize markers without applying expanded
geometry; queued native geometry writes still validate the current snapshot.
Active-Space following is blocked while native resize owns geometry.

Native completion also requires a still-active native resize owner. Error
presentation already cancels that owner; a cleanup received after recovery must
not clear a newer presentation frame or native move. With no owner completion
is a complete no-op. Pure geometry regressions exercise this recovery sequence,
duplicate completion, transient modes and cancellation of queued resize writes.

This change preserves the frontend's immediate command invocation order. Tauri
uses separate IPC requests, which do not provide a cross-request ordering
contract. A delayed old command arriving after a new gesture is a pre-existing
unverified native transport boundary: this fix does not introduce gesture IDs
or claim to solve request reordering. Signed native acceptance should include
rapid release/restart and mode changes during a drag.

The focused component suite first reproduced 16 failing cases and six uncaught
exceptions against the old implementation. It covers native/browser capture
loss, button loss, disable/reenable, unmount, outside release/cancel/blur, pointer
ownership, ignored secondary starts, duplicate finish events, IPC exceptions and
the existing minimum-size/opposite-corner geometry. Automated DOM events do not
establish real WebView capture delivery, native cursor behavior or signed
macOS/Windows interaction acceptance. Native validation uses a temporary Rust
driver extracting production pure functions and formal geometry tests, rather
than compiling or launching the GTK/Tauri application.

At this revision all 40 focused component cases pass. The quick frontend check
passes typecheck, selected lint, provider-copy validation and 214 related tests
across nine files. The standalone Rust driver passes 58 geometry/resize tests;
three new completion cases failed before the active-owner guard. Rust formatting
and diff checks pass. Full application/platform acceptance remains separate.
