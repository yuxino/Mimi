# Native capture validation for issue #78

This records bounded native checks on 2026-10-02. Issue #78 remains hardware
acceptance tracking; emulator playback capture and virtual output monitors do
not establish physical Android or Linux route behavior.

## Reusable checks

Android's instrumentation entry is the existing UI runner with
`-e playback_capture true -e consent_timeout_ms 90000`. It returns before the
ordinary settings/credential fixtures. A debug-only, non-exported activity
requests fresh MediaProjection consent; the service accepts a discard/count
engine only for the explicit test action in a debuggable application. Missing
injection fails closed. Normal start actions still use the configured provider.
The release APK must contain no probe activity.

The probe exercises the production foreground service, MediaProjection,
AudioPlaybackCapture, 48 kHz stereo float AudioRecord, provider-rate resampler,
CaptureHealth, overlay and teardown. Synthetic MEDIA AudioTracks alternate
between a permitted tone, zero samples and an opted-out tone. Counters and
timestamps are retained; PCM is discarded. No provider is started, credentials
are not read, and no microphone source is configured.

Approve the entire-screen consent for the first three requests and cancel the
fourth. For the tested English API 35 emulator UI, `-e emulator_consent true`
optionally operates those actual system controls through the instrumentation's
own UiAutomation connection. This option is off by default, requires a debug
application, emulator hardware and the QEMU flag, and refuses physical devices.
It neither grants a projection token directly nor reuses a previous result.
Do not run external `uiautomator dump` during the probe: its second connection
can conflict with the connection used for native screenshots and interrupt the
test. Use one instrumentation connection for consent and screenshots.
If an interrupted runner leaves an old system consent dialog behind, cancel that
dialog or restart the dedicated emulator before another run; do not treat its
result as fresh consent. Use a dedicated emulator or explicit test device with
no active Mimi session. RECORD_AUDIO is required for playback
capture by the [Android platform contract](https://developer.android.com/media/platform/av-capture);
the permission name does not select the audio source. The test emulator has
host audio input/output and cameras disabled.

```bash
adb -s TEST_SERIAL install -r android/app/build/outputs/apk/debug/app-debug.apk
adb -s TEST_SERIAL install -r android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk
adb -s TEST_SERIAL shell am instrument -w \
  -e playback_capture true -e consent_timeout_ms 90000 \
  app.yuxino.mimi.android.test/app.yuxino.mimi.android.UiSmokeInstrumentation
```

Read the final `PLAYBACK_CAPTURE_RESULT` and its `passed` field; an adb exit code
alone does not establish success. Local results and synthetic native screenshots
are saved in the target app's external `files/capture-probe` directory. The
assertions cover sound/silence and the actual overlay hint, fresh-session
isolation, source opt-out, native projection-stop callbacks, canceled consent,
and removal of the capture worker, projection, overlay and first-run evidence.
Projection.stop is a real callback test, not a status-chip or lock-screen test.

Linux uses `scripts/linux-audio-smoke.sh` with a private PulseAudio server and
separate output/microphone null sinks. The ignored native test checks real monitor
PCM at 16/24 kHz and bounded stop/restart. An equivalent private PipeWire-Pulse
server is a separate compatibility test. Neither server connects physical
capture devices or providers.

## Android native playback results

Android 15 / API 35 Google APIs ARM64 emulator, system image revision 09,
incremental build 12960925, Pixel 7 AVD profile (`sdk_gphone64_arm64`),
1080 × 2400 at density 420. System consent UI was English; the application's
existing overlay locale was Japanese. Version 1.5.6 / code 10506 was built from
`cbf7490b2ee2275ddeb45286218ab0daa84fc102` plus this change. Host audio input/output
and cameras were disabled. The synthetic source generated a 1 kHz stereo tone
at 48 kHz through a native MEDIA AudioTrack; capture used the production path
and discarded the resampled 16 kHz PCM after counting it.

The final run passed all ten checkpoints in the
[content-free report](../screenshots/platform-capture/android-api35-results.json).
The first permitted tone produced 51 nonzero chunks and `AUDIO`; zero media then
produced 54 zero chunks and `SILENT`, with the actual
[static no-sound hint visible](../screenshots/platform-capture/android-api35-silent.png).
Restoring the tone returned to `AUDIO` and removed that hint. The
[permitted-audio screenshot](../screenshots/platform-capture/android-api35-audio.png)
contains no captions because the test engine does not generate them.

Normal stop removed capture, projection, overlay, first-run evidence and worker,
with no later PCM. A fresh consent/session started with `WAITING` and only zero
PCM. An opted-out tone genuinely played for more than five seconds while the
capture received only zeros. `MediaProjection.stop()` delivered the native
stop callback and cleaned up; a third fresh consent captured nonzero audio
again. Final stop cleaned up, and canceled fourth consent created no session.
Native blocked-read / `NO_PCM` behavior was not established by these checks.

Each screenshot waits for overlay layout and two display frames before capture;
main-queue idle alone previously captured the old narrow overlay before the
hint had been drawn. The final PNGs were also inspected visually.

| APK | SHA-256 |
| --- | --- |
| Debug target | `f6682ff9f2f24be85484f88a31823f2f43595f9dd1141d14d059683ebb07fb34` |
| Debug instrumentation | `725265c6a6f364ad949466758e8ddb77d736ba9705599c45034d19028caec189` |
| Unsigned release | `8560479b28d72982cfa07553625f5ed4d0109b76652a8a5281577fed32f565ba` |

Debug and release each passed 40 unit tests; lint and APK builds passed. Manifest
and DEX inspection confirmed the release has neither the probe activity nor the
instrumentation classes and is not debuggable. The emulator test used fresh
platform consent on each request; no provider session, credentials or microphone
were involved. SDK, AVD and build caches are retained for another run.

## Linux virtual output results

Ubuntu 22.04.5 ARM64 VM, kernel 5.15.0-185, Rust 1.95.0. Production source was
archived from `cbf7490b2ee2275ddeb45286218ab0daa84fc102`; only the existing ignored
native test was extended. The tested `audio/linux.rs` SHA-256 is
`d43b4607c22ea48f5b04be295a3376590ab2608f2c5af70ee42aae73af235649`.

| Server | Native test result | Duration | Stop at 16/24 kHz |
| --- | --- | --- | --- |
| PulseAudio 15.99.1, original test | 1 passed, 0 failed | 5.03 s | Not measured |
| PulseAudio 15.99.1, extended test | 1 passed, 0 failed | 13.65 s | 6/13 ms |
| PipeWire 0.3.48 + WirePlumber 0.4.8, extended test | 1 passed, 0 failed | 10.36 s | 6/7 ms |

Both servers delivered real monitor PCM through libpulse at 16 and 24 kHz.
The observations were silent PCM, the matching 997 Hz output tone, silent PCM
after playback, and expired PCM/sound activity after stop. A fresh pipeline
started with neither PCM nor sound activity before each native restart. The
default input deliberately pointed at a different silent null sink. Private
PipeWire device monitors were disabled; each server and D-Bus session used its
own socket/runtime directory and only its own processes were stopped. The
existing VM, audio services and shared build cache were preserved.

## macOS installed development application

The installed `/Applications/mimi-dev.app` was tested without rebuilding or
replacing either installed app. Its version is 1.5.6-dev; its precise source
revision is not established by the installed metadata. At the start of testing,
the installed bundle's executable SHA-256 was
`258a9e41daaf61780d008db7c5c783bf36df28aa99da33332f3198aa7e573c94`.
The application was already running: that pathname hash does not independently
identify its loaded executable image. The shared development bundle changed
later during independent work, so these are installed-version observations,
not validation of a particular source commit or a subsequent rebuilt package.
macOS 26.3.1 (25D771280a), ARM64, Simplified Chinese UI, built-in speaker output.
Strict signature verification passed with the existing `mimi Local Development`
identity and the stable `app.yuxino.mimi.dev` designated requirement.

Two normal Alibaba service sessions used locally synthesized English test
speech and the application's existing private development credential source.
No credentials were inspected or changed. The first session visibly produced
source and translated subtitles, with a 44 ms interface round trip and 391 ms
translation time. During playback the actual overlay showed sound received;
after playback it showed recent PCM without obvious sound. Normal stop completed
in about 0.4 seconds. The second session began with fresh silent PCM and no
previous latency measurements; playback then produced new sound observations
and translation (94 ms interface round trip, 232 ms translation). Both sessions
were stopped at the end.

The support report intentionally retains its last capture observation with an
age after teardown; this is documented in `2026-09-30-windows-sound-source.md`.
That aged diagnostic snapshot must not be confused with a live capture observer.

## Remaining acceptance

Android 10/14/15 physical devices, system projection-chip stop, lock-screen stop,
Bluetooth and wired route changes, macOS route changes/TCC revocation, and
physical PulseAudio/PipeWire-Pulse hardware are not covered. The Android discard
engine does not establish provider transport or caption delivery. Silence and
nonzero PCM never identify DRM, speech, source permissions or hardware faults.
