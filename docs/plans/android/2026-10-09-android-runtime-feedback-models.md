# Android runtime feedback and model identity

Android continues to use the PC Rust runtime. Native UI must retain its connection
status, translation recovery reason, scheduled-retry flag and translation timeout
state rather than silently dropping those fields. Display only localized,
allowlisted state messages; capture-no-audio guidance has priority. A healthy
snapshot clears recovery guidance without clearing the complete subtitle pair.
No Kotlin timer, request scheduler or provider error parser is introduced.

Show actual model identifiers on Home and in expanded floating controls. Resolve
them from shared protocol constants and the selected text route through JNI;
ignored legacy model/endpoint overrides must not be presented as effective.
Capture the model list and language route when a session starts. Pending settings
must not relabel a running session. Keep compact/immersive captions uncluttered.

PC permits saved-profile switching through reconnect, while changing a profile's
model requires stopping. Android currently requires stopping before changing
service configuration or model. Disable those editors during capture, retain the
save-handler guard for stale events, and re-enable on stop. Do not imply live
switch parity. Test Lite, Flash and Plus sequentially with fresh Android consent,
then restore the original model. Use existing configured providers where available.

Acceptance evidence belongs in the integration run ledger: actual JNI state,
native compact/expanded feedback, model identity, attempted in-session edits,
real playback subtitle/final observations, stopping and restarting. Synthetic
snapshots alone do not establish provider or translation quality acceptance.

Gemini continuous translation may omit `turnComplete`. Its previous two-second
close deadline raced the shared two-second quiet checkpoint, dropping a paired
tail at Stop on both platforms. Give this client a 4.5-second budget inside the
six-second provider bound and finish after the existing paired quiet checkpoint.
Keep explicit-turn late-tail grace, interruption resets, generation guards and
deadline rejection of unmatched text. Do not manufacture sentence alignment or
promote drafts just because the user stops.

Compact Android captions keep the complete shared text but follow its latest
visible lines, matching the desktop compact rollup. Implement vertical viewport
scrolling in the native TextView adapter after layout, including text changes
that preserve the view's size. Expanded captions remain fully readable with
manual scrolling; no shared state, pairing or retained text is truncated. Native
regressions assert the last line's visible bounds after growth, font changes,
short/empty replacement and immersive transitions, rather than counting text
updates alone.
