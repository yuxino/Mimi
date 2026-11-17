# Move the subtitle overlay from its language panel

An expanded panel lets a mouse user drag its header outside the close control or designated blank space. Its buttons, selectors, links, help, and scrolling retain their own gestures. A blank click does not collapse the panel. Position lock and immersive presentation disable movement from the panel.

The subtitle overlay owns the user frame. A detached panel may invoke the existing `overlay_move_start` command while the native mode is Panel and both presentation locks are off. The command selects the subtitle root before recording drag intent. Existing native root movement keeps the panel attached on AppKit and follows it through geometry events on Windows and X11. On Wayland the panel already shares the same window and uses the compositor's move gesture.

Native acceptance uses a held mouse drag from an expanded panel. Check that the subtitle root moves and the panel stays open and follows, that a click still closes the panel through its own control, and that a locked overlay, scrolling, help, and language controls retain their behavior. The command's return value alone does not confirm a native drag. On macOS also verify the child-originating mouse gesture with the signed app. Wayland acceptance needs compositor or screen evidence because its global window origin is unavailable to the client.

Floating help retains hover while crossing its narrow trigger gap, and remains
open while keyboard focus stays on its trigger. Escape dismisses the current
help before its containing panel, without moving focus; an already consumed
Escape or an active IME composition must not dismiss the panel. Compact inline
labels remain pointer-transparent so they do not cover the overlay drag surface.
