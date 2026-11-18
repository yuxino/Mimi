# Platform capture health and evidence boundaries

## Confirmed gap and Android change

A successfully started Android service previously displayed “subtitles enabled”
without observing PCM. A live provider connection and AudioRecord RECORDING state
can coexist with zero PCM, silent PCM or source capture restrictions.

Observe only the PCM16 mono bytes produced by the existing resampler for the
provider. Keep two monotonic timestamps, never samples or recognized text. After
a five-second startup grace period distinguish no recent PCM, recent silent PCM,
and recent nonzero PCM. Any nonzero PCM16 sample counts as sound; this is a data
observation, not speech recognition or a guarantee of subtitle delivery. A
one-second main-thread refresh detects missing PCM even while a blocking native
read has not returned. The generation-owned observer is removed on stop and
replaced on every start. Silence is informational and does not trigger restart.

Show a neutral no-usable-sound hint in the home page and overlay, with a suggestion
to confirm playback and a possibility of source restrictions. Never infer DRM,
source opt-out, volume, route failure or permission revocation from silence alone.
Copy a versioned diagnostic containing only Android API level, fixed source and
usage labels, running state, PCM/sound ages and fixed capture error labels. Do not
copy credentials, hostnames, application/device names, audio or subtitles. Copying
is an explicit local clipboard action; there is no external diagnostic upload.

## Native mechanism audit (main 5f2a595)

| Platform | Actual source | Existing lifecycle boundary | Honest panel label |
| --- | --- | --- | --- |
| Android | MediaProjection + AudioPlaybackCapture, MEDIA/GAME/UNKNOWN, float stereo 48 kHz downmixed to provider PCM16 | FGS before getMediaProjection; callback registered before recording; generation guard on revocation; stop unblocks read; native read error stops service | Internal app playback stream; no microphone or physical speaker selection |
| macOS | ScreenCaptureKit audio on a display filter, own process audio excluded; provider-rate resampling | Bounded async startup barrier; generation-scoped teardown; native delegate stop and decode/backpressure errors use existing recovery owner | System mix / output device unknown |
| Linux | libpulse on PulseAudio or PipeWire-Pulse, startup default sink's validated monitor; DONT_MOVE recording stream | Bounded startup; cancelable worker; context/stream state and monitor index checked; failures use existing recovery owner | Output monitor selected at start / device unknown |

macOS native sample callback arrival currently updates last_audio_buffer_at before
successful decoding. That timestamp is unsuitable for pcmDataRecent. Missing or
empty sample buffers and partial resampling output produce no provider PCM;
observation should use the real ingress, after decoding and resampling. Native
stop errors are safe generic labels; they cannot reliably identify which device
or permission caused a runtime stop. A live permission revoke, output disconnect
or route change still needs a signed app and actual hardware validation.

Linux PeekResult::Empty emits no PCM. PeekResult::Hole is explicitly zero-filled
and assembled into bounded PCM frames: ingress recency means provider PCM exists,
not that native non-hole audio arrived. This is compatible with the shared
pcmDataRecent/soundRecent contract but must not be described as proof of a valid
native signal. The stream is fixed to its initial monitor; changing the server's
default sink does not imply automatic tracking. Server disconnect or a changed
stream monitor is reported as NativeStopped; recovery resolves a fresh monitor.
A still-existing silent old monitor requires a neutral hint, not forced switching.

No confirmed native lifecycle defect was found that warrants a speculative
macOS/Linux API change. Shared desktop observation/UI belongs to PR #72; this
change deliberately does not edit its public contract or session manager.

## Android platform rules reviewed

- [Playback capture](https://developer.android.com/media/platform/av-capture):
  introduced in Android 10, needs RECORD_AUDIO, user MediaProjection consent and
  the same user profile. Only eligible MEDIA/GAME/UNKNOWN playback with permitting
  capture policy can be copied. Keep all existing policy restrictions.
- [Media projection](https://developer.android.com/media/grow/media-projection):
  Android 14 foreground-service type/permission and per-session consent; do not
  reuse the projection result Intent. Android 15 QPR1 status-chip stop/lock-screen
  stop must update UI and release resources through onStop.
- [Android 15 changes](https://developer.android.com/about/versions/15/behavior-changes-15):
  mediaProjection foreground services cannot start from BOOT_COMPLETED. Mimi
  currently starts through the visible activity consent flow, not a boot receiver.
- [ScreenCaptureKit audio](https://developer.apple.com/documentation/screencapturekit/scstreamconfiguration/capturesaudio)
  and [native stop delegate](https://developer.apple.com/documentation/screencapturekit/scstreamdelegate/stream(_:didstopwitherror:)).
- [PipeWire Pulse protocol](https://docs.pipewire.org/page_module_protocol_pulse.html):
  PulseAudio-compatible server consumed through the original PulseAudio library.

## Verification scope

Synthetic tests cover startup grace, no PCM, silent PCM, signed nonzero PCM,
stalls, recovery, session isolation, partial-sample rejection and actual stereo
cancellation through the resampler. Debug/release JVM tests, lint and APK builds
are required. Repository checks cover existing desktop logic but do not prove
native device behavior. Emulator UI smoke is separate from capture verification.

Physical Android 10/14/15 playback, source opt-out, revocation, screen lock,
Bluetooth/wired disconnect and reconnect are not established by unit tests or CI.
Do not claim universal Bluetooth support or bypass communication/DRM restrictions.
macOS physical routes/TCC revocation and Linux physical PulseAudio/PipeWire-Pulse
routes also remain a hardware validation boundary. Use only synthetic tones and
no provider credentials when extending capture checks.
