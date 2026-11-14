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
