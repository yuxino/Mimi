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

On Windows, this panel-authored native move transfers foreground and keyboard
focus from the control to the subtitle root. This is an internal gesture rather
than click-away. While the focused panel owns that gesture, retire an older
scheduled dismissal and preserve the expanded surface. Observe actual move-mode
ownership and button release: outline dragging can defer all movement until
release, and a short click or cancelled drag can produce no Moved event. This
observer exists only for that gesture. On completion, return keyboard ownership
to the same control only if the owning root is still foreground. Settings or
another application's foreground dismisses without reactivating Mimi. Attempt
ordinary Win32 foreground restoration once; do not use Tao's force-focus retry
(which can inject Alt). A refusal leaves the compact entry, not an unfocused
expanded panel. Explicitly opening controls retains its existing focus path.

A cross-WebView press can release before the root acquires capture. The hosted
WinEvent trace observed the root remain in its move modal after stationary,
3-pixel and 48-pixel quick gestures. Only the current owned gesture may forward
the already-released button once to the original root, while it is foreground,
owns capture and is still in its native move modal. That modal consumes the
actual screen point: a client-relative point committed the outline to (0, 0),
while the screen-point control preserved 0, 3 and 48 pixels respectively.
Polling the cursor after up is insufficient: moving it 30 pixels immediately
after a stationary release moved the window those extra 30 pixels. The gesture
therefore holds a release-point lease only while the primary button is held and
native root capture is not established. It reads the first owned up and removes
the hook immediately, without reading other mouse payloads. Target WebView2
input is in another process, so the app's own window message queue does not
cover this boundary. Cancellation, a new move intent and teardown retire the
lease; a startup still without native capture expires after 30 seconds. Native
end messages are queued on the main thread under the current gesture token,
so a closed or subsequent gesture cannot consume an earlier release.

Normal release preserves outline displacement without synthesizing system input. A queued
panel press whose physical button is already released must not start a new OS
move. Continue to wait for native completion before restoring panel focus.
Tao posts the move request to the window thread. If the owned button release
arrives before that thread processes the request, retain its position through
that queue turn. Once the thread has handled the request, a click that never
entered a move loop can finish normally.
Keep the original long-drag and 15ms stationary-press assertions, plus quick
3-pixel and 48-pixel moves, immediate pointer movement after release, and an
ordinary blank click.

Mode changes, lock, collapse, Stop and explicit dismissal retire gesture
ownership. A late completion must not reopen a closed or newly opened panel.
Focus-loss scheduling and Panel-to-Island transitions check their generation and
current mode atomically at the state owner, so a stale timer cannot replace a
new drag or Hidden presentation. Native acceptance includes the same 48-pixel
middle-blank gesture, a short stationary press, subsequent real language
selection and true external click-away. Keep screenshot/video pixels restricted
to the offline test windows and a neutral background; report actual frame PTS
and capture gaps instead of assuming the requested frame rate.

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
