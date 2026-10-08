# Audio-only permission for macOS system mix

Refs #230. Draft implementation; audio-only native acceptance remains pending.

The current ScreenCaptureKit backend enumerates shareable screen content even
though Mimi installs only an audio output. macOS therefore requires Screen &
System Audio Recording. Apple exposes a separate audio-only permission through
Core Audio process taps, available from macOS 14.2.

## Scope and compatibility

- Use a private mono global Core Audio tap for the default system mix on 14.2+
  when screen capture has not already been authorized.
  Exclude Mimi's own Core Audio process object and leave playback unmuted.
- Keep ScreenCaptureKit on macOS 13–14.1 and for explicitly selected applications.
  Application capture still needs Screen & System Audio Recording in this draft.
- Resolve the new tap functions at runtime; retain the macOS 13 deployment target.
  Apply the same lookup to the existing vendored cpal loopback implementation:
  its strongly imported tap APIs can prevent the executable loading on 13,
  even when Mimi chooses ScreenCaptureKit. Loopback reports unsupported on older
  macOS without touching a tap. Verify the final executable's imports, not only
  the unit-test binary, where unreferenced native code can be stripped.
- Fail a tap start without falling back to broader screen permission.
- Check existing screen authorization with the public, non-prompting
  CGPreflightScreenCaptureAccess API. Already-authorized installations retain
  ScreenCaptureKit instead of being migrated to a separately permissioned API.
  New installations and users who choose audio-only access use taps on 14.2+.
  If both scopes are enabled, preserve the existing screen-authorized path;
  no private AudioCapture preflight API or cached substitute for TCC is used.
- Add no microphone access, credential handling, recording, or transcript storage.
  The existing optional local recording behavior remains unchanged.
- Promote already locked Core Audio, Core Foundation and libc dependencies for
  typed native integration; introduce no service or new transitive package.

## Lifecycle and audio

A dedicated worker owns the tap, private aggregate, and IOProc context. The
callback copies at most 64 KiB of mono PCM into a four-buffer queue. The worker
uses the existing decoder, provider-rate resampler, bounded ingress and failure
channel. Full queues fail the generation instead of accumulating audio.

Reserve the existing teardown gate before spawning. Shared capture-generation
checks discard stale callbacks and stop a worker after cancellation, including a
late native startup. Release the gate only after native cleanup. If IOProc
removal fails, retain its context and close the gate until restart rather than
risking a dangling callback or overlapping sources. Changes in output route or
format trigger the existing recoverable reconnect path.

## Implementation / verification

- [x] Add runtime tap routing and preserve application/old-system fallback.
- [x] Add worker-owned native resources, bounded callback copying and teardown.
- [x] Reuse decoding/resampling and provider ingress.
- [x] Run focused callback/routing/lifecycle tests (5 passed) and canonical
  check.sh (passed after the compatibility update, including 2,012 frontend
  tests). Development-feature library clippy and the signed dev build also pass.
- [x] Native: existing screen authorization with no audio-only entry uses
  ScreenCaptureKit without requesting a new grant; audible 16/24 kHz, stop/start.
- [ ] Native: screen permission denied + audio-only allowed; audible 16/24 kHz
  output, own-playback exclusion, stop/start/pause/source switching.
- [ ] Native: deny/revoke audio access, cancel while permission is pending,
  output-device changes and sleep/wake; confirm no ghost capture.
- [ ] Older macOS launch/fallback and selected-application regression checks.
- [ ] After native acceptance, update user-facing permission instructions for
  system mix versus application capture before marking the PR ready.

Native tests must use the stable signed /Applications/mimi-dev.app. Do not alter
permissions without explicit user approval or replace the formal release
installation during automated checks. The development-debugger-only
`--native-audio-smoke OUTPUT auto|tap|legacy` mode starts the real capture and
bounded PCM pipeline locally, checks 16/24 kHz signal and stop/start, and writes
only counts/peaks/status to an exclusively created private output file. It
initializes no product settings, credentials, providers, microphone or recording.

## System version versus permission scope

macOS 13's Apple guide describes Screen Recording; macOS 14's guide already
describes both Screen & System Audio Recording and audio-only authorization.
14.2 is the availability of the Core Audio tap implementation, not the date
after which existing screen permissions become invalid. Both scopes coexist on
newer macOS. Do not describe the scopes as exclusive old/new OS permissions.

## Native evidence (2026-10-09)

Tested on Apple silicon macOS 26.3.1(a), using the stably signed canonical dev
bundle and a locally generated 440 Hz playback signal. No provider connection
or captured audio file was created.

- Existing Screen & System Audio Recording enabled, audio-only list empty:
  production `auto` routing selected ScreenCaptureKit. Both six-second runs
  passed: 299 buffers / 93,167 audible samples at 16 kHz, then 298 buffers /
  82,161 audible samples at 24 kHz. Both drained and became idle after stopping.
  The second run proves restart after cleanup. No new permission was needed.
- Explicitly forcing the tap backend with only the existing screen grant timed
  out during setup. This does not establish that the old permission satisfies
  the tap API, or identify the exact failure cause. Production routing preserves
  the working ScreenCaptureKit path for this grant state.
- Audio-only permission with screen access disabled is still a separate required
  acceptance case. Changing only the development app's grants and restoring
  them afterward was authorized, but System Settings requires the user's
  Touch ID/password authentication before the temporary change can complete.
  The pending operation was canceled; verified the dev and formal screen grants
  remain enabled and the audio-only list remains empty, matching the initial
  state. The formal release application's grants were not changed.

## References

- https://support.apple.com/zh-cn/guide/mac-help/mchld6aa7d23/mac
- https://developer.apple.com/documentation/coreaudio/capturing-system-audio-with-core-audio-taps
- AudioHardwareTapping.h and CATapDescription.h in the macOS SDK (14.2 availability).
