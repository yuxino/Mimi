# Runtime reliability and release baseline

> 2026-08-24. This record captures the architecture hardening pass that keeps
> the existing Tauri/React/Rust product shape while making every streaming
> boundary bounded, every startup acknowledgement truthful, and the shipped
> bundle consistent with the APIs it uses.

## Decision

Mimi remains one native Rust process with small React WebViews. The current
stack already fits the product: native system-audio capture, Tokio networking,
provider-specific clients behind a typed facade, and a UI-independent core.
Replacing it would add migration risk without removing the actual failure
modes, which live at asynchronous ownership and queue boundaries.

The hardening work therefore follows four rules:

1. **Latest previews, reliable finals.** Recognition and translation drafts
   are replaceable snapshots. Provider-confirmed subtitle pairs are reliable,
   ordered events and the only entries written to durable on-screen history or
   translation memory.
2. **Bound every producer/consumer boundary.** Audio ingress, provider events,
   capture failures, high-quality final translation work, retries, connection
   setup, writes, health checks, and shutdown all have explicit capacity or
   deadlines. An authoritative overflow is a content-free recoverable failure,
   never silent data loss or unbounded allocation.
3. **Generation-own all asynchronous work.** Timers, preview translations,
   startup completions, event pumps, health checks, and recovery attempts can
   publish only while their generation or revision remains current. Cleanup
   clears its own task slot on success, failure, and cancellation.
4. **Acknowledge real readiness.** A session reaches Listening only after the
   provider has accepted its configuration and the platform capture API has
   completed its native start callback. Waiting is asynchronous; the macOS main
   thread is never blocked.

## Streaming topology

System-audio callbacks use a non-blocking `try_send` into the single bounded
audio-send pipeline. There is no unbounded bridge ahead of that queue. The
callback drops no old audio secretly: on the first full queue it emits one
typed overload signal, closes admission for that generation, and lets the
session recovery owner rebuild the transport.

Provider output has two logical lanes:

- source/translation drafts are latest-value state and may be replaced before
  the UI consumes them;
- lifecycle, error, and final-pair events use a bounded reliable lane.

Every event carries ordering ownership within a session generation so a late
draft cannot overwrite a later final. Backpressure on reliable events reaches
the owning provider task; it cannot grow an unbounded heap queue.

For Alibaba high-quality and Turbo modes, stable/maximum-wait work translates a
latest-only preview. It never advances the authoritative committer, appends
history, or enters Qwen translation memory. Audio 3.0 server finals alone enter
the hard-capped serial final queue and produce an atomic `SubtitleFinalPair`.
A server final invalidates any in-flight preview. While the final lane is busy,
new ASR drafts accumulate but cannot start a competing preview; the latest
pending draft is rescheduled only after the lane becomes idle. This removes
provisional history revocation and the associated cross-request race.

## Recovery and diagnostics

Transport EOF, unexpected WebSocket close, native capture stop, resampler
failure, queue overload, and timeout are typed failures with stable diagnostic
labels. User-facing errors may explain the problem, but logs contain only
labels, status codes, timing, counts, and language/mode identifiers—never
recognized text, translations, credentials, or raw provider payloads.

Retryable failures use bounded exponential backoff with jitter and generation
checks. Authentication, invalid configuration, and permission denial are
terminal. A newer stop/start/settings action always invalidates older recovery
work.

## Overlay control ownership

The existing language-popover WebView becomes an `overlay-control` owned/child
window with Island and Panel modes; no extra WebView is added. The subtitle
overlay remains a click-through canvas when locked or background-blended, while
the compact control island remains interactive. Session inactivity and the
collapsed overlay hide it. Native code is the single writer for visibility,
mode, and geometry, including negative-coordinate monitor work areas.

The panel exposes only provider-supported language/mode choices plus an
explicit subtitle-background show/hide action. Paused sessions may change
language and mode because the backend already applies those settings on resume;
connecting and stopping remain guarded. Language and mode choices use compact
two-column grids: every choice remains one click away, odd final choices span
the full row, and the common automatic-recognition panel covers materially less
of the video than the former vertical lists without introducing nested menus.
When a monitor work area is shorter than the preferred content height, the
native window remains clamped and the panel scrolls internally; its intrinsic
height stays measured so moving it to a larger display restores the full panel.

## Release baseline

- macOS 13.0 is the deployment minimum, matching ScreenCaptureKit audio APIs;
  native compilation, generated bundle metadata, the stable dev bundle, and CI
  use the same target.
- Vite emits Safari 16 code for macOS and Chromium 105 code for Windows.
- The production WebViews use a restrictive CSP that permits only bundled
  assets and Tauri IPC. Development has a separate localhost/HMR policy.
- Tauri capabilities are window-scoped: all windows may listen for events,
  only Settings may emit its navigation acknowledgement, native dragging
  belongs solely to the overlay, and native resizing solely to the tray panel.
- WebView devtools are an opt-in feature of the stable development bundle and
  are absent from formal release builds.
- Ordinary CI and pull-request bundle jobs have read-only repository access;
  tag builds stage private artifacts, and only the final publication job
  receives `contents: write` after the developer-installable macOS package,
  privacy metadata, and embedded-app checks have passed.
- The Rust library emits only the desktop `rlib`; the release profile uses LTO,
  one codegen unit, size optimization, panic abort, and symbol stripping.

## Verification contract

Pure tests cover queue capacity, timer ownership, stale preview suppression,
authoritative overflow, worker-slot cleanup, startup completion, retryability,
negative-coordinate work areas, control-state transitions, and mode capability
filtering. `./scripts/check.sh` remains the repository gate.

On macOS, `./scripts/dev-app.sh --ui-only` verifies normal, blended, locked,
paused, connecting, error, collapsed, long-subtitle, English, and Japanese
surfaces without reading credentials, opening provider sockets, or starting
capture. A real-provider latency check is separate and uses only credentials
saved in the app’s private local credential file, with current task authorization.
