# Bound native resize IPC work

The native resize handles previously sent one `resize_move` invocation per
pointer event without waiting for preceding commands. A lightweight real
component probe held responses: 60 moves produced 61 outstanding requests, 120
moves produced 121, and release/unmount/remount plus a new start/move reached
124. A deliberately reordered mock delivery of old start/moves, new start, old
end, new move cleared the new native owner. This proves the frontend has no
backpressure and that such delivery would be harmful; it does not establish
native transport reordering frequency, desktop CPU usage or interaction latency.

Use one module dispatcher per WebView. Keep one sent gesture, one latest waiting
gesture and one latest unsent coordinate for each. There is at most one unsettled
invocation. Await start before sending moves; while a move is outstanding,
replace its unsent successor with the newest screen coordinates. When released,
flush the final coordinate and await end before starting the next gesture. Send
immediately when idle, without adding a frame timer or routing subtitle work
through this queue.

Handles retain pointer/capture ownership locally. Unmount, disable, capture loss
and release close their token, while the module finishes its native work across
component remounts. It retains no DOM nodes or component callbacks. A gesture
released before start is sent is discarded; repeated clicks cannot create a
backlog of start/end pairs. A newer waiting owner replaces the previous waiting
owner. Browser-preview resizing and its geometry remain local and unchanged.

A rejected or synchronously throwing start/move aborts that gesture and requires
a successful end barrier before another start. A failed normal end gets one
cleanup attempt. Failed cleanup leaves the dispatcher dirty and blocked; a fresh
gesture can request another end attempt before starting. There is no timer or
unbounded automatic retry. A request that never settles keeps the single slot:
timeouts cannot safely release it and send later commands without reintroducing
ordering ambiguity. Waiting gestures and move storage remain bounded meanwhile.

Ordering follows acknowledged completion of each request, rather than assuming
FIFO delivery of concurrent requests. It does not guarantee native geometry has
already been drawn when a command responds. Module state is isolated to a
WebView's JavaScript realm; it survives component remounts but not a WebView
reload. Requests from an old realm or another native command owner, and
transport-internal retries/duplicates, still require a native gesture identity
protocol for a complete guarantee. This change neither introduces that protocol
nor claims signed native acceptance.
Windows additionally samples the native cursor when a command executes instead
of trusting WebView screen coordinates; rapid release/restart still needs
Windows interaction validation, independently of a browser's coordinate fixture.

Formal scheduling tests cover the in-flight bound, move replacement, closing
barriers, dropped waiting clicks, superseded owners, finite failure recovery and
unsettled requests. A real component regression drives 120 pointer moves then
unmounts/remounts: only old start is sent before its response, old final move/end
must complete before the replacement start, and peak in-flight stays one. The
existing primary-pointer, capture cleanup, minimum-size and native screen-coordinate
regressions remain in the focused suite.

The quick frontend check passes provider-copy validation, typecheck, selected
lint and 228 related tests across ten files, including 41 component and 13
dispatcher cases. Signed platform interactions and full application acceptance
are separate checks.

A Chromium preview also drives 120 actual pointer moves with capture and
unmount/remount while holding synthetic invoke responses. Peak in-flight is one,
final screen coordinates match and the old end response precedes the new start.
Eight existing release/capture-loss/unmount/disabled browser and native-stub
checks pass. This validates browser/component behavior at a synthetic IPC
boundary; it does not replace Windows/macOS native interaction acceptance.

Complete frontend validation passed 2,317 tests in 142 files, lint, the
typechecked production build and all 17 production-output boundary checks.
Rebasing onto `ba87a50` preserved the tested source tree. Compared with the
existing `f949d61` build in the same environment, total JavaScript grew by
755 bytes, or 295 bytes when summing Node gzip level 6 output per file; this
is the dispatcher cost, not an installer-size or end-to-end performance claim.
