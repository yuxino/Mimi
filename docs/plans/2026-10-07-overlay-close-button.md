# Stop and close from the subtitle toolbar

Add a neutral × button at the right end of the subtitle toolbar and collapsed
bar. Its tooltip/accessibility label says “Stop translating and close subtitles”
in all three UI languages. At the native minimum width, prioritize close beside
the capsule; pause/retry joins it when both buttons fit. Reserve seven buttons
in full toolbar geometry.

The button calls the existing store `stop` → `session_stop` lifecycle. That path
stops capture and provider work, finalizes opted-in local files, publishes idle,
and hides the subtitle and capsule/panel windows together. Do not directly hide
a window before stopping, quit the application, change saved preferences or
clear saved history. Settings and the tray retain their existing session toggle.

Reuse existing pending/failed action feedback. Disable duplicate clicks and close
while already stopping; keep close usable during connecting, paused and error
states. Grant `session_stop` to the subtitle overlay's native permission.

The user reviewed the native panel preview and requested removing its duplicate
pause and close actions. The expanded configuration panel retains error recovery
but has neither session button nor pause/stop permissions. Locked/immersive reading
keeps its existing presentation; sessions can still be stopped from settings,
the tray or the existing global shortcut.

Test toolbar/collapsed routing, pending/failure/retry, narrow clearance and the
native permission. Verify signed macOS UI-only close without credentials,
provider traffic or capture, then restore ordinary idle dev. Live provider/audio
correctness is outside this UI feature's verification scope.

The first full-suite run reproduced the settings render regression on unchanged
main. The upstream fix was merged as #196 during this task and adopted before
final verification; this feature does not duplicate that change.
