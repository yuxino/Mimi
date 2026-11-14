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
