# Move and recover the subtitle overlay

The expanded subtitle canvas accepts a primary mouse drag from its blank space,
listening state and subtitle text. Taking the gesture before the timeline's
reading handler keeps window movement from pausing automatic subtitle following.
Wheel/trackpad scrolling, touch gestures, the actual scrollbar, resize edges and
interactive controls retain their own input. Pointer events from portaled menus
outside the canvas cannot start a window move. A double-click does not fold the
overlay. Position lock, immersive presentation and a pending control action
disable this move path.

An expanded panel lets a mouse user drag its header outside the close control or designated blank space. Its buttons, selectors, links, help, and scrolling retain their own gestures. A blank click does not collapse the panel. Position lock and immersive presentation disable movement from the panel.

The subtitle overlay owns the user frame. A detached panel may invoke the existing `overlay_move_start` command while the native mode is Panel and both presentation locks are off. The command selects the subtitle root before recording drag intent. Existing native root movement keeps the panel attached on AppKit and follows it through geometry events on Windows and X11. On Wayland the panel already shares the same window and uses the compositor's move gesture.

Native acceptance uses a held mouse drag from an expanded panel. Check that the subtitle root moves and the panel stays open and follows, that a click still closes the panel through its own control, and that a locked overlay, scrolling, help, and language controls retain their behavior. The command's return value alone does not confirm a native drag. On macOS also verify the child-originating mouse gesture with the signed app. Wayland acceptance needs compositor or screen evidence because its global window origin is unavailable to the client.

Floating help retains hover while crossing its narrow trigger gap, and remains
open while keyboard focus stays on its trigger. Escape dismisses the current
help before its containing panel, without moving focus; an already consumed
Escape or an active IME composition must not dismiss the panel. Compact inline
labels remain pointer-transparent so they do not cover the overlay drag surface.

Keyboard exposure requires both a currently focused document and a live
`:focus-visible` descendant. A retained focus style in a background WebView
must not keep the local toolbar or its peer visible after opening Settings.
Track document focus independently of physical hover; losing focus alone does
not mean the mouse left a nonactivating subtitle window.

Position lock hides the whole independent control window as well as the root
toolbar; it must not leave an invisible input island. The frontend also marks a
locked control surface inert while the native mode update arrives. Existing
error recovery temporarily restores interaction, without changing the saved lock
preference. Unlock through Settings or the global Command+Shift+L shortcut on
macOS (Control+Shift+L on Windows/X11). Wayland exposes `--toggle-lock` for the
compositor's shortcut configuration, alongside the existing immersive command.
Each native action has its own debounce and persists through the shared
preference store. A failed save preserves the current lock state.

The immersive and lock actions show their available shortcut in a compact
tooltip keycap. Unknown shortcut support and Wayland do not advertise native
key combinations. Settings exposes the same recovery shortcut/command. Verify
both lock/unlock and immersive/restore in the signed native app; DOM hiding and
mocked command calls alone do not establish global-shortcut recovery.
