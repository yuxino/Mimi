# Linux native audio recovery regressions

These checks extend the synthetic Linux coverage for #78. They use the real
PulseAudio server and Mimi's native monitor capture, with generated tones and
isolated null sinks. They do not use a physical microphone, provider credentials,
external speech services, or the released microphone UI.

Run `dbus-run-session -- ./scripts/linux-audio-smoke.sh` on a Linux environment
with the repository build dependencies. The harness discovers every required
ignored test before starting the private audio server. Each native test verifies
the private directory marker, socket/runtime selection and directory permissions
before changing the server. No desktop audio service is stopped or reconfigured.

## Added scenarios

Each scenario runs at 16 kHz and 24 kHz PCM16 mono:

- Change default output from A to B while both play distinct tones. Capture must
  remain on A until explicitly stopped. Stop A's original tone, observe silence,
  then play a new 811 Hz tone on A while B plays 613 Hz. This prevents pre-switch
  buffered PCM from falsely passing the route assertion. A fresh capture selects B.
- Remove the selected null sink while another output is playing. Capture reports
  one `NativeStopped` failure rather than falling back to the other output.
  Recreate the sink and explicitly restart, observing fresh silence and tone.
- Kill and reap a test-owned PulseAudio child. Capture reports `NativeStopped`;
  starting while the service is absent reports `AudioServerUnavailable`. A new
  child on the same private socket permits fresh silence/tone capture. This test
  runs separately after the harness-owned initial daemon has exited.

After native stop, the receiver stays open while queued sends settle; new PCM
must cease and the activity window must expire. Retiring the pipeline then rejects
old ingress and releases pending PCM before the next generation is created.
This does not assert that provider sends cease at the instant of native failure:
SessionManager lifecycle cleanup and provider reconnection are separate layers.

## Interpretation

A passing run proves these software-monitor transitions for the tested PulseAudio
version. Default changes intentionally require a session restart; automatic
following is not the product contract. These tests do not establish PipeWire-Pulse,
physical hotplug/Bluetooth, native Wayland, actual caption/translation accuracy,
or Android/macOS acceptance. No production capture behavior is changed by adding
the regressions.
