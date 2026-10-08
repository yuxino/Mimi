# Current design records

This directory contains decisions that still constrain the current Tauri
application. Completed task checklists and superseded Swift-era plans belong in
Git history, not on the active documentation path.

## Architecture and security

- `2026-10-08-build-throughput.md` — lean validation symbols, faster local QA
  packaging, shared package caches and independent updater verification.
- `2026-09-28-session-history-design.md` — opt-in local transcript and audio
  history, date groups, deletion, and export boundaries.
- `2026-09-27-linux-support-design.md` — output-monitor capture, Secret Service,
  Linux desktop limits, packages, and signed AppImage updates.

- `2026-07-22-stable-local-signing-design.md` — stable development identity,
  canonical install path, and privacy-permission continuity.
- `2026-08-27-developer-installable-release-design.md` — credential-free,
  ad-hoc-signed GitHub packages with structural and digest verification.
- `2026-08-15-dashscope-unified-endpoint-design.md` — Alibaba shared endpoints
  and the API-key-only configuration contract.
- `2026-08-15-session-start-main-thread-blocking-design.md` — async IPC and
  ScreenCaptureKit startup boundaries.
- `2026-08-22-multi-provider-professional-settings-design.md` — provider
  profiles, credential isolation, migration, and runtime ownership.
- `2026-08-27-mainstream-realtime-providers-and-profile-deletion-design.md` —
  typed realtime adapters, provider-specific keychain credentials, and
  WebView-safe destructive confirmation.
- `2026-08-24-runtime-reliability-and-release-design.md` — bounded streaming
  topology, generation ownership, recovery, control-island ownership, and the
  native/WebView release baseline.
- `2026-08-28-efficiency-size-maintenance-design.md` — per-window frontend
  loading, allocation-free buffer slicing, parallel branch bundles, and the
  supported desktop asset boundary.
- `2026-08-30-windows-audio-format-and-ci-design.md` — complete Windows PCM
  sample conversion plus native x64 and ARM64 compile/start CI coverage.
- `2026-09-01-single-instance-design.md` — process-safe repeated launch,
  activation, and hidden-settings restoration on Windows.

## Runtime behavior

- `2026-09-24-session-export-design.md` — default-off session retention,
  bounded in-memory audio and transcript buffers, and explicit native export.

- `2026-09-07-audio-http-lifecycle-design.md` — cancellation-safe audio drain,
  bounded Qwen responses, and HTTP dependency consolidation.
- `2026-08-23-alibaba-pipeline-design.md` — current Alibaba modes, subtitle
  commit rules, translation context, and recovery behavior.
- `2026-08-23-overlay-behavior-design.md` — overlay state, history, controls,
  geometry, cross-space visibility, presentation, accessibility, and animation.
- `2026-08-14-tauri-dev-dock-icon-design.md` — why development builds require
  the custom-protocol feature for stable bundle behavior.

## Product surfaces

- `2026-09-24-settings-layout-design.md` — sidebar navigation, compact session
  controls, dedicated export page, and appearance previews.

- `2026-10-03-subtitle-background-opacity.md` — adjustable desktop background
  transparency, a stronger default card, and full-contrast history.
- `2026-09-24-settings-appearance-design.md` — system, light, and dark
  settings appearances and the local preference boundary.
- `2026-08-23-simplified-settings-and-tray-design.md` — current settings and
  tray information architecture.
- `2026-09-02-signed-in-app-updater-design.md` — signed, user-initiated
  download/install/relaunch updates and the release metadata contract.
- `2026-08-26-fullscreen-space-following-design.md` — macOS active-Space
  reassertion and non-persistent cross-display overlay following.

When a decision changes, update or supersede the relevant record. Do not add a
second implementation checklist after the work is complete, and do not include
fixed test counts that become stale as the suite grows.
