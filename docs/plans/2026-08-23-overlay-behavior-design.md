# Subtitle overlay behavior

## State and content

The overlay is a Tauri window rendered by React and backed by the native session
state. Streaming drafts are replaceable; confirmed pairs form bounded, durable
on-screen history. Local transcript and audio retention are off by default.
When explicitly enabled, bounded recording and export follow the
[session history design](2026-09-28-session-history-design.md).

The timeline uses one scrolling flow for confirmed history plus the active tail.
Long text wraps, the newest content remains visible, and draft presentation is
bounded without truncating confirmed history.

The active tail respects the requested output language. A live translation
draft or uncommitted final takes priority. During cross-language translation,
source recognition must not appear as fallback while translation is pending,
empty, or timed out. Confirmed translated history remains visible and the
activity status communicates the wait. If the detected source language matches
the target (or the target is Original), recognition remains valid display text,
including final source lines. A source final already committed to the newest
history pair is not rendered twice. Pending/timeout bits distinguish repeated
utterances from previously committed pairs in modes that display recognition.
The text stabilizer immediately clears removed tails and confirms final text;
source/translation identities prevent an old source preview surviving a switch.

The activity presentation is derived from session state:

- connecting and recovering show a non-destructive progress state;
- listening distinguishes recognition from pending final translation;
- paused is static and keeps existing subtitles visible;
- errors show sanitized status text; and
- reduced-motion mode removes decorative movement without hiding state.

## Controls

Expanded and collapsed layouts expose only actions that can take effect in the
current lifecycle state. Pause/resume, collapse/expand, language/mode selection,
clear, lock, and settings actions share native session guards; controls do not
optimistically claim a mutation that the backend rejected.

Clearing subtitles resets visible drafts, pending pairing state, and confirmed
history without changing listening status. It is available from the overlay and
tray panel and does not require confirmation because it deletes no persisted
content.

The existing language-picker WebView is an owned/child `overlay-control` window
with two modes. Island is a compact always-available status/language capsule;
Panel expands in place for provider-supported language/mode choices, Immersive
Mode, position locking, and the settings entry. It never changes subtitle-canvas
geometry and does not add another WebView. Paused sessions may change language
or mode; connecting and stopping mutations remain disabled. Language and mode
buttons form two-column, one-click grids; an unpaired final option spans the
full row so the panel stays compact without hiding choices in submenus. If the
available work area is shorter than the panel, the control content scrolls
inside the clamped native window instead of making its lower actions unreachable.

On macOS, AppKit's native child-window relationship is the live source of truth
while the subtitle window moves; handling the later `Moved` event with another
manual position write would make the control island trail the canvas. AppKit
automatically detaches a child when hiding it with `orderOut:`, so every control
show restores the parent relationship immediately before ordering the panel
front, in the same main-thread transaction. Windows owned windows and Linux
transient windows continue to follow explicitly. Every platform performs one
final derived-position clamp after movement settles, so monitor, scale, and
work-area changes remain correct without adding drag lag.

## Geometry and interaction

Native code owns the overlay frame, control-window mode, collapse/expand
transitions, resize bounds, work-area clamping, click-through lock state, and
control anchoring. Geometry writes are versioned and atomic. Layout uses each
monitor's work-area origin as well as its size, including left/above monitors
with negative coordinates.

Native move and resize gestures remain unconstrained while the pointer is down
so the window server can render them without a one-event-late correction. When
the gesture settles, native state fits the complete subtitle frame into the
work area that contains most of it: oversized dimensions are reduced first and
then the origin is clamped. The corrective position/size write is skipped when
the frame is already valid. This prevents partially visible frames from being
persisted without adding per-frame work that would make the separate control
island trail the subtitle canvas.

The native window remains above ordinary windows and joins every desktop space.
On macOS 13 and later it also opts into joining other applications' window sets
and full-screen Spaces, then identifies as a full-screen auxiliary window so
subtitles remain visible over full-screen media. True full-screen video is
composited above ordinary floating and status windows, so the subtitle and
control surfaces use the native screen-saver window level. They are stationary,
ignored by window cycling, and ordered with `orderFrontRegardless` instead of
Tauri's key-window show path; starting subtitles therefore does not activate
mimi or pull focus away from the media app. The macOS native presentation is
the sole runtime writer for these properties rather than racing the
cross-platform setters. Explicit show requests may re-order the surfaces, while
routine subtitle broadcasts never do so. All raw AppKit collection-behavior,
level, and ordering updates are dispatched to the main thread because
visibility transitions can originate from the asynchronous session worker.

An ordinary `NSWindow` remains outside the compositor group used by Safari and
other players for true full-screen video even when its level and collection
behavior are otherwise correct. On macOS only, the existing Tauri subtitle and
control WebView windows are therefore converted in place to dedicated
`NSPanel` subclasses after their first page load has finished. Reload callbacks
first check the registered panel handle so the native class is never converted
twice. Both panels preserve their existing borderless/resizable style bits and
add `NonactivatingPanel`; the subtitle panel can never become key or main, while
the control panel may become key only when its expanded controls need input and
can never become main. Expanding that control does not use Tauri's macOS focus
path, because it activates the whole app; the click on the nonactivating panel
already supplies any key status required by its WebView. Settings and tray
surfaces remain regular windows. The conversion uses `tauri-nspanel` as a
macOS-only dependency pinned to commit
`c9ec2130422200f0863b23dfdad02b133a529b07`, because Tauri does not expose an
equivalent native panel conversion API. Conversion failures destroy the
affected surface and emit only a content-free diagnostic label.

The pinned dependency is deliberately used through only `init`, the panel
macro, and `WebviewWindowExt::to_panel`; mimi does not use its `PanelBuilder`,
stored `PanelHandle`, commands, or event API. The conversion performs an unsafe
Objective-C class swap, so it runs from the WebView's main-thread page-load
callback before either hidden surface can be shown; all later raw AppKit
mutation remains behind Tauri's main-thread dispatcher. This local rule is
stricter than an inaccurate upstream claim that panel calls dispatch
themselves: the pinned implementation merely marks panel handles `Send`/`Sync`
and requires callers to enforce the main thread.

Audit of the pinned tree found no root build script, network client, process
execution, credential access, IPC command, or added protocol. Cargo still needs
GitHub access on a clean build because this is a git dependency. Its upstream
default features expose more Tauri/AppKit surface than mimi needs; that compile
and review cost is accepted for the contained native conversion rather than
duplicating the class-swizzle and lifetime machinery with direct `objc2` calls.
A future offline-build requirement should vendor the exact source, and every
revision change must repeat the source, feature, thread, and license audit.
Mimi selects the compatible MIT license and retains the upstream notice in
`THIRD_PARTY_NOTICES.md`.

The macOS app runs with the accessory activation policy: the menu-bar tray is
its persistent entry point, while removing a separate Dock/Cmd-Tab presence
makes its overlay eligible to accompany another application's full-screen
presentation. The ordinary settings window remains available from the tray.

The drag handle and resize regions provide visible hover/focus feedback. Locking
enables click-through without hiding subtitles. Animation is brief and uses the
same 180 ms ease-in-out timing in native geometry and React content; reduced
motion replaces it with immediate state changes.

All eight resize handles remain independent rather than forcing an aspect ratio.
The empty presentation adapts across comfortable, compact, and minimum-height
densities; at the native minimum, the status line takes priority over the
decorative pulse so free vertical stretching never clips the only useful text.

## Presentation

Subtitle alignment is a persisted visual preference with left, center, and
right options. It affects text layout only and remains safe to change while a
translation session is active. The settings window exposes the complete
preference and the tray panel provides the same three-way quick control.

Immersive Mode is a persisted, runtime-safe visual preference. In this mode
native state forces the overlay expanded and renders subtitle text directly on
the transparent window with a restrained readability shadow. The subtitle
canvas removes its card, status chrome, timestamps, buttons, drag affordance,
and resize handles and becomes click-through, effectively locking the position
without overwriting the user's independent position-lock preference. The
separate control island is hidden as part of the chrome; Immersive Mode remains
reversible through the tray panel, settings window, and the global
Command/Ctrl+Shift+M shortcut. Entering or exiting the mode preserves the exact
subtitle content inset, so text does not jump vertically. When the regular
canvas is interactive, Immersive Mode and Lock Position appear in its right-side
quick-action row. The action row remains subtly visible at rest and reaches full
contrast on hover; the language island likewise stays readable at full opacity,
with near-white primary text and clearly legible secondary status instead of a
dimmed inactive treatment. Its timeline remains internally scrollable and
auto-pinned to the newest subtitle, but the visual scrollbar is hidden. No
subtitle text or audio is persisted by this presentation mode.

## Verification

Pure tests cover state derivation, segmentation, geometry, resize regions,
popover clamping, and reducer behavior. UI checks cover expanded/collapsed,
empty, listening, translating, paused, error, long-text, locked, reduced-motion,
and keyboard-focus states. Use `./scripts/dev-app.sh --ui-only` for visual checks
that must not access credentials, provider networks, or system-audio capture.
For deterministic native window-level checks, launch that UI fixture with
`MIMI_UI_TEST_STANDARD_OVERLAY=1` and `MIMI_AUTO_START=1`; the standard
presentation override exists only in memory and is never written to
preferences.
