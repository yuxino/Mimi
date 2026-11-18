# Windows audio-format and dual-architecture CI design

## Problem

Windows loopback capture reads the default playback endpoint's mix format, but
the callback currently accepts only F32 and I16. CPAL 0.18 can expose ten other
PCM or floating-point formats, so an otherwise valid playback device can be
rejected before capture starts. The existing Windows CI also uses an implicit
x64 runner and has no ARM64 compile, test, or launch coverage.

## Design

- Keep the usual F32 callback as a zero-copy fast path.
- Convert I8, I16, I24, I32, I64, U8, U16, U24, U32, U64, and F64 through
  CPAL's sample conversion contract into normalized F32 before the existing
  downmix/resample/PCM16 pipeline.
- Treat WASAPI I24 as a special case. Windows stores 24 valid PCM bits
  left-aligned in a 32-bit `WAVEFORMATEXTENSIBLE` container, while CPAL 0.18
  exposes those native bytes through its four-byte `I24` value. Arithmetic
  right-shift by eight before normalization so the signal is not amplified and
  clipped by roughly 256 times; discard the container padding bits.
- Reuse one normalization buffer owned by the capture processor so integer or
  F64 callbacks do not allocate a new vector every time.
- Reject DsdU8, DsdU16, DsdU32, and future unknown formats before stream
  creation. DSD is a bitstream rather than PCM and cannot use the PCM
  resampler safely.
- Pin x64 CI to Windows Server 2025 and add a native Windows 11 ARM64 job. Both
  architectures run Rust tests, compile the complete Tauri app, verify the PE
  machine type, and keep a credential-free UI-test process alive briefly.
- During local and native Windows acceptance, run the vendored CPAL WASAPI
  capture-packet tests explicitly. A path dependency's own unit tests are not
  discovered by Mimi's root `cargo test`, so the command must be invoked
  separately from the repository's existing CI workflow.
- Install the built stream in Mimi's capture slot before queueing CPAL `play`,
  while holding the same slot lock. An immediate play failure clears the slot
  and active flag in that critical section; asynchronous WASAPI failure cleanup
  can then never run before installation and strand a dead stream that blocks
  retries as `AlreadyRunning`.

Mimi's patched CPAL source uses `windows` 0.61 COM interfaces. Its direct
`windows-core` dependency stays on the same 0.61 line; pairing those generated
interfaces with 0.62 macro/runtime types does not compile and would make fresh
dependency resolution non-reproducible.

## Verification boundary

Windows unit tests cover support classification, DSD rejection, normalization
of every signed, unsigned, 24-bit, and floating-point representation, and
buffer reuse. They also inject occupied-slot, immediate-play-failure, and
cleanup/retry cases without requiring an audio device. CI startup uses Mimi's
synthetic UI-test mode, which never reads
credentials, opens provider connections, or starts audio capture. It proves
the x64/ARM64 executable can start on GitHub runners, but does not prove real
WASAPI loopback, tray interaction, installer acceptance, or physical x64/ARM64
audio-driver behavior.

The ignored `native_default_output_opens_as_a_wasapi_loopback_stream` test is
the credential-free real-device probe. Run it explicitly on Windows to open
the current default render endpoint as a loopback input, play an in-memory
997 Hz non-speech tone through that same resolved device, and require a 4,096-
sample PCM16 window with sufficient RMS and normalized 997 Hz projection
energy to reach Mimi's normal bounded send pipeline. Starting the continuous
tone before capture prevents queued pre-tone audio from satisfying the probe.
The probe distinguishes an empty failure queue from a disconnected worker and
an actual native failure, then stops both streams without persisting audio. It
stays outside ordinary CI because hosted runners do not guarantee a playback
endpoint.
