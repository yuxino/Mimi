# Linux desktop support

## Scope

Add x86_64 Linux packages built on Ubuntu 22.04, retaining the same providers,
bounded PCM pipeline, overlay, opt-in export controls, and credential privacy.
Ship `.deb` and AppImage; keep the website in its separate repository.

## Capture

Use `libpulse-binding` only on Linux. PulseAudio's native API also works with
PipeWire's PulseAudio compatibility server. Direct ALSA capture lacks a reliable
system-output monitor; a `parec` child process adds an external CLI dependency
and complicates lifecycle handling. The native asynchronous API allows bounded
startup and cancellation without a blocking read that can strand shutdown.

One worker owns the PulseAudio context, stream, and mainloop. Resolve the
default sink and verify its monitor belongs to that sink before connecting.
Never connect to an unspecified/default recording source. The server converts
to provider PCM16LE mono at 16 or 24 kHz. Assemble native peek fragments into
fixed 20 ms frames before the existing 20-slot send queue; small fragments
must not shorten its audio-duration budget. Keep less than one frame pending
and drop that tail on stop/cancellation rather than padding or waiting. Keep
the 250 ms native-fragment limit and distinguish oversized-fragment diagnostics. Capture remains pinned to that output;
users restart after changing devices. Pause stops capture and resume starts a fresh worker; stop cancels the
worker, and fatal errors use the existing sanitized capture-failure channel.
Content-free slow-path diagnostics separate native polling gaps, send-lock
waits, socket-send waits, and time since the last completed send when the
bounded queue fills. Keep timing separate from log-write overhead and preserve
the existing queue and recovery policy while diagnosing VM/network stalls.

## Credentials and desktop

Initialize Xlib threading before Tauri/GTK opens its first display. Reuse Tao's
already-locked `x11-dl` version as a direct Linux dependency for that early call;
it does not open a display or select X11 over Wayland. Ubuntu 22.04 native UI
stress tests reproduced XCB sequence corruption when initialization happened
too late, and the early call must be verified without preload diagnostics.

Credentials use the same private local-file design as the other platforms.
Secret Service is a one-time legacy import and verified-retirement source only;
normal reads, saves and deletes never require it after import. See the
[Linux development guide](../development/linux.md) and
[credential storage design](2026-10-04-local-credential-storage.md).
Optional microphone capture follows the separate
[audio input design](2026-10-03-optional-microphone-input.md). Retention and
recording stay off by default.

Use a colored tray asset outside macOS. Settings remains accessible at startup
when the desktop has no tray host. Linux closes to exit rather than hiding an
unreachable process; minimize Settings to keep subtitles running. Recommend X11; document compositor limits
for positioning, always-on-top, click-through, and global shortcuts on Wayland.

## Linux presentation

Read mapped overlay geometry directly on GTK's main thread. The initial
ConfigureNotify cache can still report `(0, 0)` after showing a hidden window,
which detaches the language island at the desktop origin. Keep controls hidden
until the parent maps, then follow its map event without a timer. Explicitly
size the non-resizable GTK control window to its visible island/panel bounds;
the default natural height otherwise leaves a transparent click-catching area.
Reuse the already-locked GTK crate for these native operations. Native smoke
checks first-map attachment, 236-by-30 island bounds, and movement following.

Prefer a consistent installed Noto/desktop UI font across WebKitGTK text and
form controls. Allow full language labels and credential state to wrap within
the default Settings width. Use a darker subtitle card and clearer empty-state
text on Linux while preserving the existing appearance on other platforms.

## Distribution and proof

Use a Linux Tauri config with explicit package dependencies. AppImage updates
use the existing signed manifest; non-AppImage Linux copies open Releases for
package updates. Preserve macOS identity/source verification and Windows ZIP
behavior. Release publishing requires Linux assets and signatures as well as
the existing platforms.

Run strict Rust/frontend checks on all platforms, an isolated PulseAudio test
with generated PCM, an isolated Secret Service test with non-secret fixtures,
package inspection, and a credential-free Xvfb native launch. State physical
Linux and real-provider acceptance separately from these automated checks.
Native smoke requires both lazy-loaded frontends to commit and paint with the
listening snapshot before recording separate, content-free readiness markers.
The marker IPC does nothing outside explicit UI-test mode and accepts no path
or content from the frontend. Publishing also waits for Windows ARM64 checks.
