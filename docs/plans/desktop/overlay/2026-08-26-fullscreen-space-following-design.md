# Full-screen Space and display following

## Problem

The subtitle and control panels already opt into every Space and full-screen
presentation, use a full-screen auxiliary collection behavior, and reassert
their native level when Mimi explicitly shows them. That is sufficient while
the active Space remains stable, but macOS can rebuild or reorder the
full-screen compositor group after another application enters or leaves full
screen. Mimi currently receives no signal for that transition, so a visible
panel can remain eligible for the full-screen Space without being ordered into
the newly active presentation.

Space eligibility also does not move a window between physical displays. On a
multi-display Mac, a subtitle panel can correctly join the active full-screen
Space while its frame still belongs to the display saved in Mimi's normal
layout. The result looks like a z-order failure even though the panel is simply
outside the visible coordinates of the display showing the full-screen app.

## Decision

On macOS, Mimi observes the public
`NSWorkspace.activeSpaceDidChangeNotification`. After the Space transition has
settled, a visible overlay reasserts the existing panel collection behavior,
native window level, parent relationship, and front ordering. Notifications
are coalesced so an animation burst cannot enqueue stale reassertions after a
newer Space change.

The same settled callback treats `NSScreen.main` as the display currently
presenting the user's active app. If the subtitle panel is on another screen,
Mimi temporarily maps its frame into the main screen's visible frame. The
mapping preserves the panel's relative placement and size where possible, then
fits the complete frame into the target work area. The control panel continues
to derive its position from the subtitle panel and follows in the same native
presentation transaction.

`NSScreen.main` is ignored while Mimi is frontmost or owns a key window. A
settings window, tray panel, or overlay control can otherwise make Mimi's own
screen look like the media target and pull subtitles away from the user's
full-screen app. In that case Mimi only reasserts Space eligibility and window
ordering; it waits for the next non-Mimi Space/show transition before changing
presentation geometry.

The observer, screen lookup, frame mapping, collection-behavior changes, and
ordering all run through AppKit on the main thread. Geometry debounce tasks
read window and monitor state before acquiring the overlay mutex, then recheck
both the event timestamp and geometry-state snapshot after locking. This avoids
a main-thread/worker lock inversion and rejects stale reads during simultaneous
Space and native-move events. Every deferred window write is then serialized
through the main event loop and revalidates the same snapshot; a superseded
worker transaction cannot move the panel back after a newer Space transition.
Custom-resize reads and writes follow the same rule. Each collapse/expand
animation step is also revalidated on the main event loop before touching the
window, so a newer Space follow, resize, mode change, or user frame cancels the
old animation tail. The native-drag marker is deliberately separate from this
apply snapshot: merely beginning an AppKit drag must not strand an unrelated
size transition at an intermediate height. Native frame reads use the broader
snapshot, including that marker and the latest geometry-event timestamp, so a
drag that begins during a read still invalidates its stale result.
A missing main screen or destroyed panel is a safe no-op apart from the ordinary
eligibility and ordering reassertion. Hidden overlays update their runtime
presentation frame and native geometry without being ordered onscreen, so the
next session cannot flash first on a stale display. A single-display setup
resolves to the saved screen and therefore produces no cross-display movement.

## Persistence and interaction

`OverlayState.user_frame` remains the sole saved user position. An automatic
display-follow move is a runtime `presentation_frame`, not a preference change:
native geometry matching that presentation must not update `user_frame` or the
settings store. The override remains active across collapse, expansion,
and visibility changes so those transient operations cannot snap the overlay
back to its saved display. When Mimi returns to the saved display, the first
matching native geometry event suppresses persistence and then clears the now
redundant override.

The drag handle records an explicit native-drag origin before handing the
gesture to AppKit. Only an origin change promotes a followed frame to
`user_frame`; intermediate collapse/expand animation sizes are not mistaken for
manual placement. Custom resize already has its own intent marker and similarly
promotes the currently presented frame before resizing it.

This preserves both expected behaviors:

- entering a full-screen Space on another display makes the subtitles visible
  there without silently changing the user's normal layout; and
- dragging or resizing the followed overlay remains intentional and persists
  through the existing geometry path.

The runtime override is replaced by each newer Space-follow generation and is
cleared when a different user-authored frame arrives or the saved-display move
settles. It does not contain subtitle content and is never serialized.
User-frame persistence also rechecks the current frame and serializes competing
manual placements before writing preferences, so an older drag cannot finish
after and overwrite a newer one.

## Platform boundaries

The implementation uses only documented AppKit APIs:

- `NSWorkspace.activeSpaceDidChangeNotification` for Space lifecycle changes;
- `NSScreen.main` and visible screen frames for the destination display; and
- the existing `NSWindow`/`NSPanel` collection behavior, level, frame, child,
  and ordering APIs.

It does not request Accessibility permission, inspect another application's
windows, depend on private Space identifiers, or use private Core Graphics or
WindowServer APIs. Windows and Linux keep their existing overlay behavior.

## Verification

Pure geometry tests cover relative mapping between displays, negative display
origins, smaller target work areas, and full-frame clamping. State tests cover
matching programmatic geometry being ignored, a later manual move being
promoted to the user frame, collapsed manual movement preserving the saved
expanded size, collapse animation not impersonating a native drag, and a fitted
drag result not being overwritten by a second sync. The observer coalescing
comparison prevents an older delayed notification from applying after a newer
generation.

macOS QA uses the signed `/Applications/mimi-dev.app` path. With a second
display connected, start with a saved overlay position on one display, enter a
true full-screen video on the other, and verify that the already-visible
subtitle and control panels appear on the full-screen display without focus
theft. Exit full screen and relaunch Mimi to verify the original saved frame was
not overwritten. Repeat while manually dragging the followed overlay and
verify that this explicit position does persist. The same flow on one display
must reassert presentation without changing geometry or producing repeated
privacy prompts.
