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

Native setup diagnostics name only fixed stages. The aggregate uses explicit
start rather than `tapautostart`: the SDK documents that enabling that key waits
for the first tapped audio. This wait is unsuitable for starting subtitles while
the computer is quiet. The tap's quiet-start behavior still requires native
acceptance once its separate permission case can be tested.

Capture active-state changes and generation assignment/invalidation share a
short synchronous transition lock. Otherwise a concurrent stop between claiming
active state and allocating a generation could be followed by a fresh generation
from the already-stopped start. Failed-start state updates use the same lock so
that they cannot clear a newer start. No transition lock is held across awaits.

## Implementation / verification

- [x] Add runtime tap routing and preserve application/old-system fallback.
- [x] Add worker-owned native resources, bounded callback copying and teardown.
- [x] Reuse decoding/resampling and provider ingress.
- [x] Run focused callback/routing/lifecycle tests (9 tap tests, plus lifecycle regressions) and canonical
  check.sh (including 2,012 frontend tests). Development-feature library clippy and the signed dev build also pass.
- [x] Native: existing screen authorization with no audio-only entry uses
  ScreenCaptureKit without requesting a new grant; audible 16/24 kHz, stop/start.
- [x] Native: selected-application capture, source restart, and application exit
  under the existing screen authorization.
- [ ] Native: screen permission denied + audio-only allowed; audible 16/24 kHz
  output, own-playback exclusion, stop/start/pause/source switching.
- [ ] Native: deny/revoke audio access, cancel while permission is pending,
  output-device changes and sleep/wake; confirm no ghost capture.
- [ ] Older macOS launch/fallback; selected-application authorization without
  an existing screen grant.
- [ ] After native acceptance, update user-facing permission instructions for
  system mix versus application capture before marking the PR ready.

Native tests must use the stable signed /Applications/mimi-dev.app. Do not alter
permissions without explicit user approval or replace the formal release
installation during automated checks. The development-debugger-only
`--native-audio-smoke OUTPUT auto|tap|legacy [SUITE]` mode starts the real capture and
bounded PCM pipeline locally, checks 16/24 kHz signal and stop/start, and writes
only counts/peaks/status to an exclusively created private output file. It
initializes no product settings, credentials, providers, microphone or recording.

## System version versus permission scope

macOS 13's Apple guide describes Screen Recording; macOS 14's guide already
describes both Screen & System Audio Recording and audio-only authorization.
14.2 is the availability of the Core Audio tap implementation, not the date
after which existing screen permissions become invalid. Both scopes coexist on
newer macOS. Do not describe the scopes as exclusive old/new OS permissions.

| Runtime / grant state | Production choice | Acceptance |
| --- | --- | --- |
| macOS 13–14.1 | ScreenCaptureKit | Runtime pending; API routing tested |
| 14.2+, screen grant on, audio-only absent | ScreenCaptureKit | Native passed on 26.3.1(a), no new grant |
| 14.2+, both grants on | ScreenCaptureKit | Routing tested; native grant case pending |
| 14.2+, screen grant off, audio-only on | Core Audio tap | Native pending |
| 14.2+, neither grant on | Core Audio tap, request audio access | Prompt/denial native cases pending |
| Explicit application source | ScreenCaptureKit | Native source/exit passed with screen grant; separate grant case pending |

Audio-only authorization is not inferred or cached by Mimi. A new tap start
lets macOS enforce that scope; the non-prompting screen preflight only preserves
existing compatibility. Availability/routing tests do not prove native behavior
on every historical runtime or grant combination.

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

## Expanded native matrix (2026-10-09)

All successful rows below use the existing screen grant on macOS 26.3.1(a).
They verify the ScreenCaptureKit compatibility path. They must not be described
as audio-only tap permission acceptance.

| Case | Result |
| --- | --- |
| 12 alternating 16/24 kHz starts, two seconds per capture; duplicate start; double stop; clone handle | Passed: audible buffers in each cycle, first PCM after 154–222 ms from pipeline creation, duplicate start rejected, idle/drain true, no new ingress after stop |
| Startup cancellation after 0/1/5/20/100/250 ms | Passed: canceled or already-completed start cleaned up, no post-stop buffers |
| Quiet system without playback, 16/24 kHz | Passed: capture starts and sends quiet PCM, no timeout or failure |
| Mimi plays its own synthetic tone through its real output device | Passed: 620,544 output frames, output error false; captured audible samples remain zero at both rates |
| Selected application is missing, then select system mix | Passed: application unavailable, zero input and clean idle; subsequent audible system capture succeeds |
| Selected QuickTime synthetic playback, then system mix | Passed at 16/24 kHz: audible selected-app capture and clean source restart |
| Selected QuickTime process exits during capture | Passed: application-unavailable failure observed, then idle/drain and no post-stop buffers |
| Force the tap to fail, then retry in the same process | Passed as a failure-containment case: setup timeout, pending teardown, retry rejected as previous capture stopping, zero PCM; no second native tap setup or broader fallback |

A 650 ms capture window passed an earlier 12-cycle run but a later run produced
zero buffers in its first cycle, with no native error and clean teardown. The
cause remains unconfirmed. A subsequent six-second 16/24 kHz control passed,
then the final 12-cycle two-second run passed with first-buffer timing recorded.
This establishes those measured runs, not guaranteed audio within 650 ms. One
separate run was invalidated when the coordinator parsed an incomplete JSON line
and terminated playback; it is excluded from acceptance evidence. The
coordinator was corrected to read only complete lines and preserve playback
until native exit.

The tap setup timeout was reproduced with fixed-stage diagnostics and a sampled
worker stack: `AudioDeviceCreateIOProcID` waits in Core Audio's server property
RPC. The main event loop continues running. The exact cause remains unconfirmed;
changing the first-audio-wait flag and starting only after Tauri Ready did not
resolve it under the existing screen grant. No Apple/permission inheritance
conclusion is drawn from this failure.

Permission cases still required: screen disabled/audio-only enabled; both scopes;
neither scope; user denies the first prompt; grant revoked while capturing; stop
or quit while a permission decision is pending. The routing and native-error
classification have automated coverage, but changing these grants still needs
user Touch ID/password authentication on this Mac. Both broad grants remain on
and the audio-only list is still empty after canceled verification attempts.

Other native cases still required: macOS 13/14.1/14.2 runtime behavior, tap-own
playback exclusion, physical device disconnect/default route changes, sleep/wake,
and complete product pause/resume/reconnect. This smoke mode deliberately avoids
SessionManager/providers; backend stop/start proof is not full product pause or
provider acceptance.

Suites: `smoke`, `stress`, `silence`, `cancel`, `missing-application`,
`own-playback`, `failed-start`, `application`, `application-exit`. Stress/cancel
and ordinary signal cases need controlled external playback. Silence needs a
quiet output; own-playback generates and counts its own test signal. Application
cases use a task-owned synthetic file in QuickTime; exit testing closes only
that task-owned player after the capturing marker. These suites do not save PCM
or read product profiles/API credentials.

## References

- https://support.apple.com/zh-cn/guide/mac-help/mchld6aa7d23/mac
- https://developer.apple.com/documentation/coreaudio/capturing-system-audio-with-core-audio-taps
- AudioHardwareTapping.h and CATapDescription.h in the macOS SDK (14.2 availability).
