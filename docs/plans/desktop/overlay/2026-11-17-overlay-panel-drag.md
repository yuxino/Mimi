# Move and recover the subtitle overlay

The expanded subtitle canvas accepts a primary mouse drag from its blank space,
listening state and subtitle text. Taking the gesture before the timeline's
reading handler keeps window movement from pausing automatic subtitle following.
Wheel/trackpad scrolling, touch gestures, the actual scrollbar, resize edges and
interactive controls retain their own input. Pointer events from portaled menus
outside the canvas cannot start a window move. A double-click does not fold the
overlay. Position lock, immersive presentation and a pending control action
disable this move path.

Dragging the timeline scrollbar back to the bottom resumes live captions.
Some WebViews send a series of scroll events without pointer movement while
the thumb is held. Follow that gesture through release, including outside the
viewport. A resize or a new caption can also move the scrollbar. Keep the
reader on their sentence when the change comes from layout alone.

An expanded panel accepts the same primary mouse drag from its header, section
labels and blank space, including below the first section. Its buttons,
selectors, links, help, portaled menus and scrolling retain their own gestures.
A blank click does not collapse the panel. Position lock and immersive
presentation disable movement from the panel. The panel fits its content up to
520 CSS pixels and the available viewport; excess content scrolls. This height
limit must not disable movement or take over the scrollbar's gesture.

The subtitle overlay owns the user frame. A detached panel may invoke the existing `overlay_move_start` command while the native mode is Panel and both presentation locks are off. The command selects the subtitle root before recording drag intent. Existing native root movement keeps the panel attached on AppKit and follows it through geometry events on Windows and X11. On Wayland the panel already shares the same window and uses the compositor's move gesture.

On macOS the press must stay anchored to its position inside the subtitle root.
A stationary click must leave both the root and its attached language control
in place, including when the pointer is over a visible subtitle glyph. A drag
of that glyph moves both windows by the pointer's displacement while held.
The native check compares both window origins before pressing, while held and
after releasing, then drags back to the starting point.

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

Position lock makes the subtitle root click-through and hides its toolbar, but
keeps the compact independent control region reachable. Moving the pointer over
that region reveals a direct Unlock action; moving away hides it again. The
saved lock must not pin this entry visible. Locking closes an expanded panel,
and unlocking leaves a compact language capsule without opening the panel or
Settings. The frontend uses the same compact presentation while native mode
updates arrive. A failed unlock preserves the lock and shows sanitized,
transient feedback with a retryable action. Recording and error states retain
their explicit visibility. Error recovery temporarily restores interaction
without changing the saved preference.

On Windows, a pointer returning to the same spot can hit the correct WebView2
child while the page still reports no hover. While a subtitle surface is
visible, Mimi checks which native window owns the pointer and sends changes
to the owning surface. Covered and click-through windows do not own it.
Hidden windows stop the checks. A reloaded renderer requests current ownership
through the same handshake that restores hover between the two surfaces.

Settings and the global Command+Shift+L shortcut on macOS (Control+Shift+L on
Windows/X11) provide additional recovery. Wayland exposes `--toggle-lock` for the
compositor's shortcut configuration, alongside the existing immersive command.
Each native action has its own debounce and persists through the shared
preference store. A failed save preserves the current lock state.

The immersive and lock actions show their available shortcut in a compact
tooltip keycap. Unknown shortcut support and Wayland do not advertise native
key combinations. Settings exposes the same recovery shortcut/command. Verify
both lock/unlock and immersive/restore in the signed native app; DOM hiding and
mocked command calls alone do not establish global-shortcut recovery.

Temporary changes block conflicting actions without fading the whole panel.
Shared pickers distinguish busy from unavailable: busy keeps the existing
appearance and accessible pending state; empty or unsupported choices remain
dimmed. Refresh animation belongs only to an actual refresh. Retain the panel,
selection, scroll position and retry behavior through success and rejection.

The compact entry shows the current language pair. The open panel keeps each
label above its own dropdown. The two fields share a row when their labels
and selected languages fit. A longer name can use the spare width or move
its field onto a full row. Each audio source has its own switch. The
application picker sits directly under system audio, before the microphone.
The three subtitle display modes use short labels with their full meaning
available to assistive technology. The floating toolbar shares one background.
Thin dividers separate the reading actions, immersive and lock, and Close.
