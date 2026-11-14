# Desktop session feedback

Starting and resuming should show Connecting as soon as the desktop session
enters that state. Listening still requires the provider and all selected
capture lanes to be ready. Publish these boundaries through the ordered
session snapshot path without waiting for the 60 ms subtitle batch. Keep
the batch for streaming content and avoid a second immediate Connecting
snapshot when a manual start has already sent one.

Pause closes admission for the old generation before publishing Paused.
Release capture and the provider connection alongside that publication.
Keep the existing lifecycle guard until cleanup completes so Resume, a new
Start, and source changes cannot install resources over the retired ones.
Both forms of the subtitle overlay show the Resume icon on the confirmed
Paused snapshot. The button stays disabled until the pause command returns.
Settings and the tray continue to use the same native snapshot.

Verify the expanded and collapsed overlays with a delayed pause reply.
Exercise start, pause, resume, stop, and error paths with the signed dev app
and the existing synthetic fixtures. Provider handshake, first caption, and
OS capture timings are separate measurements.

## Overlap pause cleanup

Once the retired generation's echo processor and PCM send queues are shut
down, start the native capture stops and provider disconnects together. Each
client still saves its own translation budget before disconnecting. The
existing cleanup owner waits for every branch and preserves the provider
pump when it owns recovery. Resume and replacement starts still wait for
the same cleanup boundary. Explicit Stop keeps its ordered audio drain and
provider finish so accepted final subtitles can reach history.

If a recovery owner is already cleaning up when Pause is accepted, retire
the active generation and detach recovery before publishing Paused. The
ordered snapshot can reach the overlay while the old owner finishes. Keep
the recovery wait before active-generation cleanup, and retain the same
lifecycle guard and disabled Resume control until both are complete.

On Windows, the capture worker signals completion only after its native
stream has dropped. Have Pause and Stop await that signal directly, with
the existing timeout at the caller and the same worker-slot ownership. A
completion that races the wait must wake all callers, and cancelling a
caller must leave the worker reserved until its native resources drop.

Use the same worker ownership for capture from a selected Windows process.
Its own native loop, activation timeout, target selection and COM release
order stay in the process adapter. The shared worker notifies Pause or Stop
after that adapter releases its resources, without polling for completion.

## Publish the first snapshot after idle

When no batch publisher is active, its first poll reads and publishes the
current snapshot. Requests that arrive while it publishes or during the
following 60 ms interval share a trailing snapshot. One mutex protects the
transition back to idle so a concurrent request either joins the current
publisher or starts the next one. Snapshots still read current state and
publish under the same gate as Clear and event application.

## Keep settings navigation on its own control

A provider configuration error can arrive between two clicks on the pause
button. Retire that button when the session changes to an error.
The existing error message offers the named configuration action, or the
open control panel offers it when it owns the recovery actions. Transient
errors can still offer a separate Retry button.

## Avoid rewriting identical preferences on Start

Normalize and validate preferences under the existing transaction lock.
At startup, compare the serialized preferences with the existing regular
file before writing. An exact match already has the required contents.
Missing, unreadable, or differing files go through the atomic writer.
Explicit preference saves continue to use the normal write path.

## Show the initial draft in an empty subtitle view

An unstamped draft can arrive with the same stream key as the empty view.
Allow the first text in each lane to render immediately when the lane has
no confirmed subtitle. Keep the existing settle and maximum-wait timers for
revisions. A temporary removal still clears the text without reopening the
first-draft path for the same stream key.

## Check the Windows overlay controls

In the manual Windows package check, close Settings before interacting with
the standard overlay. Find its enabled controls in the native accessibility
tree. On the x64 runner, require a native hit at the button center before
sending a Windows mouse click. The hosted ARM64 runner can have another
foreground window covering the desktop; invoke the native accessibility
action there and record it separately from mouse coverage. In both modes,
Pause and Resume must alternate while Settings stays hidden, and the gear
must open the original Settings window. Use the existing synthetic session
and temporary English preferences without a provider connection or capture.

On x64, also record the Pause button's screen point, then use a one-shot
fixture to deliver a configuration error on that click. Once the error's
configure control is rendered, click the original point again. Settings must
stay hidden. Explicitly click the separate configure control to prove the
original Settings window remains reachable. Keep the ordinary Pause/Resume
and gear checks independent of this error fixture.
