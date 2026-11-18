# Session workspace recovery and export ownership

This pass improves existing session history and export behavior without changing
capture, recording defaults, provider protocols or portable subtitle rules.

## Export worker ownership

An export stays exclusive until its disk work has actually completed. For a
current-session export, the lifecycle guard must stay with each blocking read
or write, including when the requesting future is cancelled. Move current
snapshot reads off the async executor; retain the existing archive revision
check after the save picker. Do not hold the lifecycle lock while the user
chooses a destination. Saved-session exports remain available during capture.

The blocking worker owns the busy guard during reads and the final write.
The native picker's completion callback owns it while the dialog is open and
returns it together with the selected path; cancelling the awaiting request
cannot open a second picker before the first callback completes.
Errors, picker cancellation and completed writes release ownership normally.
Cancellation tests hold a worker open and prove that another export or lifecycle
mutation cannot enter before that worker finishes. Existing atomic replacement
and sanitized errors remain in place.

## History and transcript recovery

History-list and transcript-page read errors need a nearby Retry action using
existing localized copy and button styles. Retrying preserves the selected
session, search and page, and does not read credentials or start playback or
capture. Keep read feedback beside its content, disable duplicate retries while
that read is pending, and reject results from older visits after navigation,
hiding or unmounting. Repeated failures remain retryable. Only a successful
list read clears a catalog error; opening a record, renaming or deleting cannot
dismiss an unresolved read failure or show it inside an unrelated confirmation.

## Verification

First reproduce both boundaries with focused behavioral tests. Review siblings
including archive monitoring and audio-player retry without broad UI changes.
Visible acceptance should cover browser fixtures across eight locales and
minimum/default widths plus the signed macOS development surface. The user
requested that testing not disturb video playback, so foreground browser/native
inspection is deferred; the watch launcher was stopped during compilation,
before installation or launch. A hidden browser covered 96 settings combinations
across six pages, eight locales and 520/760 px widths, plus 32 retry-error cases.
Targeted DOM interactions checked recovery, navigation, reading choices, delete
cancellation and diagnostic refresh; 28 screenshots remain outside Git. These
fixtures do not establish native, hover/animation or provider/audio acceptance.

Run the relevant frontend and desktop checks and the complete canonical check
in the background with limited build concurrency. Review the final diff
independently, keep the running production
app untouched, and remove task-owned temporary artifacts after verification.
