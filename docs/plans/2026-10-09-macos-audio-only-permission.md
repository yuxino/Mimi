# Audio-only permission for macOS system mix

Refs #230. Draft implementation; native permission acceptance remains pending.

The current ScreenCaptureKit backend enumerates shareable screen content even
though Mimi installs only an audio output. macOS therefore requires Screen &
System Audio Recording. Apple exposes a separate audio-only permission through
Core Audio process taps, available from macOS 14.2.

## Scope and compatibility

- Use a private mono global Core Audio tap for the default system mix on 14.2+.
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
  check.sh (passed, including 2,012 frontend tests).
- [ ] Native: screen permission denied + audio-only allowed; audible 16/24 kHz
  output, own-playback exclusion, stop/start/pause/source switching.
- [ ] Native: deny/revoke audio access, cancel while permission is pending,
  output-device changes and sleep/wake; confirm no ghost capture.
- [ ] Older macOS launch/fallback and selected-application regression checks.
- [ ] After native acceptance, update user-facing permission instructions for
  system mix versus application capture before marking the PR ready.

Native tests must use the stable signed /Applications/mimi-dev.app. Do not alter
permissions or replace the formal release installation during automated checks.

## References

- https://support.apple.com/zh-cn/guide/mac-help/mchld6aa7d23/mac
- https://developer.apple.com/documentation/coreaudio/capturing-system-audio-with-core-audio-taps
- AudioHardwareTapping.h and CATapDescription.h in the macOS SDK (14.2 availability).
