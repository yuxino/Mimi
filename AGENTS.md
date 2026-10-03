# AGENTS.md

## Project

Mimi is a Tauri v2 desktop app (Rust backend + React/TypeScript frontend) that listens to system audio and/or an explicitly selected microphone on macOS, Windows, or Linux and shows live translated subtitles in a floating always-on-top overlay. It supports built-in service profiles and custom live speech recognition with independent text translation, plus optional, explicitly enabled session recording and export.

Preserve these product constraints:

- Capture system audio by default. Microphone capture requires explicit selection and starting a session. Allow either or both sources, with independent capture, bounded queues, recognition sessions and subtitle state. Never mix sources, silently fall back, or request microphone permission for system-only sessions. Changing selected sources requires stopping and clears the audio-recording opt-in. Preserve source identity in subtitles and recordings.
- Subtitle history retention and selected-input audio recording are off by default. When enabled, save bounded confirmed subtitles and/or selected audio to private local session files as content arrives. Do not keep full session transcripts or PCM recordings in memory; only bounded overlay display content and size/limit metadata may remain there. New sessions and normal exit finalize the local files. Disabling an option clears its current-session content; saved sessions require explicit deletion.
- Store production API credentials in the OS keychain only (macOS Keychain / Windows Credential Manager / Linux Secret Service via `keyring`). Never add source-controlled or process-environment credential fallbacks. The explicitly requested local macOS dev exception is the default-off `local-dev-credentials` feature, further gated by `app.yuxino.mimi.dev` and non-UI-only mode: a private, validated, read-only app-config `.env` supplies only the built-in Alibaba development preset, without fallback, reveal, copying or migration. All other development configurations remain editable and use their own development-scoped OS credentials, just like normal profiles. Keep this exception isolated; see [local development credentials](docs/development/local-dev-credentials.md).
- Keep diagnostics content-free: timing, counts, language codes, status codes, and sanitized error labels are acceptable; recognized or translated text is not.

## Repository map

- `shared/mimi-core/`: the single Rust implementation of subtitle state,
  transcript alignment and final translation policy. Desktop imports it directly;
  Android calls it through `shared/mimi-android-jni/`. Keep capture, transport,
  platform credentials and rendering in native adapters.

- `src-tauri/src/core/`: UI-independent models, configuration, wire protocols, subtitle assembly, text segmentation, and pipeline diagnostics. Pure Rust, fully unit-tested.
- `src-tauri/src/clients/`: tokio network clients (Alibaba live translate/Audio 3.0/Qwen-MT pipelines and OpenAI Realtime translation).
- `src-tauri/src/audio/`: system-audio capture (macOS ScreenCaptureKit via `screen-capture-kit`, Windows WASAPI loopback via `cpal` + `rubato`), Linux PulseAudio / PipeWire-Pulse output monitors, optional default microphone capture, and the bounded PCM send pipeline.
- `src-tauri/src/session_manager.rs`: session lifecycle — start/stop/pause/resume, language/mode switching, health checks, automatic reconnection, state events.
- `src-tauri/src/settings_store.rs`: preferences/profile JSON in the app config directory + provider/profile-scoped keychain credential storage.
- `src-tauri/src/{commands,windows,lib}.rs`: IPC commands, overlay/tray-panel window management, tray/shortcut wiring.
- `src/`: React frontend — `src/windows/{overlay,tray-panel,settings}/` contain the three product surfaces; `src/lib/{ipc,store,types,i18n}.ts` define the IPC contract.
- The product website lives in the separate `yuxino-labs/mimi-web`
  repository. Never add a website copy, subtree, or generated site assets to
  this application repository.
- `docs/plans/`: current accepted design records; completed checklists and
  superseded designs stay in Git history.
- `docs/development/common-regressions.md`: required macOS signing, permission,
  Keychain, overlay, and local-testing pitfalls. Read it before packaging or
  diagnosing a repeated system prompt.
- `scripts/check.sh`: canonical automated test and strict-build entry point.
- `scripts/package-app.sh`: release-shaped local build via `tauri build`, signed with the stable local identity; compare its identity before replacing a release, especially older ad-hoc installations.
- `scripts/codesign-identity.sh`: honors an explicit `MIMI_CODESIGN_IDENTITY`; otherwise it selects the exact fingerprint of the unique `mimi Local Development` identity or reports unavailable. macOS packaging and development launch fail closed rather than use ad-hoc signing.
- `scripts/prepare-macos-release.sh`: prepares public macOS assets on the signing Mac using the certificate pinned in `scripts/macos-release-identity.txt`. Keep the private key local. Tag CI verifies the staged draft assets, source revision, updater signature, and DMG; never restore an ad-hoc fallback. See `docs/development/macos-release-signing.md`.
- `scripts/verify-macos-install-identity.sh`: compares the complete designated requirement before a formal app is replaced.
- `.github/workflows/ci.yml`: CI (Rust fmt/clippy/test on macOS, Windows, and Linux, frontend checks).

## Working agreements

- Interactive controls with hover feedback must keep a pointer cursor across the overlay, control panel, tray panel and settings, including help controls. Preserve disabled/busy, text-input, slider and resize cursors. On macOS, verify the nonactivating overlay's native cursor path as well as CSS; a successful native cursor update does not guarantee WebKit will retain it on later movement.
- Keep explanatory copy out of persistent small-print paragraphs. Use compact help icons with hover/focus tooltips for non-essential descriptions, protocol requirements, and storage details. Keep field labels, essential choices, and actionable errors visible at the normal interface text size. Do not add small text merely to fill space or explain an otherwise clear control.
- Keep related action buttons compact, consistent, and right-aligned. Use existing icons. Configuration deletion uses a red destructive action and a standard confirmation dialog, never an expanding inline strip. Choosing a service type must not create a profile until the user confirms adding it. Display connection-check progress and results with the triggering action, including actual request duration; recognition and text translation have independent checks.
- One-time operation feedback (copy, refresh, save, export, or up-to-date checks) belongs in the shared transient toast, never a full-width banner or a persistent paragraph that moves page content. Keep one toast per settings window, replace repeated notifications, and clear on navigation, native/DOM blur, close, hide and unmount. Keep field validation, connection-check results, storage failures and update actions visible beside their controls.
- Read the relevant source and tests before changing behavior. For non-trivial behavior changes, add or update a design note in `docs/plans/`.
- Keep UI-independent logic in `src-tauri/src/core/`; keep Tauri, window, keyring, and OS-audio integration in the app-layer modules. Never import `tauri` types in `core/` or `clients/`.
- Preserve Rust concurrency safety. Isolate mutable network or lifecycle state behind `Arc<Mutex<…>>` or actors; never hold a `std::sync::MutexGuard` across an `.await`.
- Treat streaming drafts as replaceable previews and final events as durable subtitle history. Do not let preview work block, reorder, or overwrite final translations.
- Keep queues and on-screen draft growth bounded. Latency fixes must account for cancellation, reconnects, stale generations, empty results, and out-of-order completions.
- Add focused `#[cfg(test)]` coverage when changing `core/`. `cargo test` runs the repository's suite.
- The wire protocols (JSON shapes, model names, domain prompts, filler glossaries) mirror the upstream services exactly; do not reword the translation prompts.
- Do not introduce a dependency, external service, or credential requirement unless the task needs it and the trade-off is documented.
- On macOS use `/Applications/mimi-dev.app` for all pre-push testing. Never overwrite `/Applications/mimi.app` with a locally signed package when its designated requirement differs from the installed GitHub Release. An intentional certificate migration must be explicit and is expected to require one final Screen Recording and Keychain authorization.
- A normal credential snapshot may read each profile API key once, but must not touch migration-only Keychain items after a profile-scoped key exists. Keep non-secret migration bookkeeping off the steady-state authorization path.
- Never delete and recreate a Keychain credential to refresh its ACL, widen an item or keychain to allow-all, or fabricate a Team ID for a self-signed build. Preserve the same service/account and update its secret in place. Password-free Keychain continuity across rebuilt binaries requires an Apple-issued signing identity with a stable Team ID; the current self-signed identities guarantee a stable designated requirement for TCC, not that stronger Keychain property.

## PC and Android parity

- Change common subtitle/translation rules in `shared/mimi-core`, never by adding
  another Kotlin or desktop implementation. Update shared behavioral fixtures and
  run both direct Rust and actual JNI tests. Shared source changes must trigger
  both desktop and Android CI. A packaging or JNI failure must fail the build;
  never silently fall back to an independent reducer.

- Maintain shared provider behavior through `shared/translation-contracts.json`, consumed by Rust and Kotlin tests. A provider/API fix must update the common fixtures and both implementations together; do not treat a passing test on one platform as proof for the other.
- Keep text translation separate from recognition on both platforms. Protocols, optional authentication, response filtering, source/result pairing, cancellation, and final deadlines must follow the same contract where the feature exists.
- Keep platform-native UI and capture implementations. Record intentional feature or resource-limit differences in [platform parity](docs/development/platform-parity.md); do not silently imply an Android feature exists because desktop supports it.
- Shared fixture and provider changes trigger both desktop and Android CI. Verify both suites before publishing their changes.

## Verification

Run the repository check from the repository root:

```bash
./scripts/check.sh
```

It runs `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, and the frontend typecheck/lint/test/build pipeline, plus whitespace/error checks on the diff.

Additional checks by change type:

- UI changes: on macOS run `./scripts/dev-app.sh` and inspect the settings window, tray panel, and overlay in normal, empty, error, paused, collapsed, translating, and long-subtitle states. This launches a signed bundle from one canonical path so macOS does not treat every rebuild as a new app and repeat privacy prompts. Use `./scripts/dev-app.sh --ui-only` for credential-free UI smoke tests; UI-test mode must not access provider networks or start any audio capture. On Windows, use `npm run tauri:dev`.
- Latency or streaming changes: measure against a real session for the affected provider (user-supplied OS-keychain credentials) and report timing diagnostics as well as correctness tests.
- Packaging or signing changes: read `docs/development/common-regressions.md`, run `./scripts/package-app.sh`, and verify the resulting app opens without replacing an installed app of a different designated requirement. Windows packaging is verified on a Windows machine (or CI). Never commit `dist/`, `src-tauri/target/`, or signing identities.

Before committing, inspect the diff for credentials, recordings, subtitle content, personal paths, and build artifacts.

## Release language

- Write public release notes in English first, followed by Simplified Chinese, using `## English` and `## 中文` sections. Translate the same changes, installation/update requirements, and verification limits; keep links, filenames, version numbers, and checksums exact.
- Keep release titles concise: product and version, with any descriptive subtitle in English and Chinese. Do not publish Chinese-only or English-only release descriptions.
- Use the same reviewed bilingual notes for GitHub Releases and updater metadata. Check generated release text before publishing; autogenerated commit lists alone are not bilingual release notes.
- Credit contributors for work first shipped in that release with plain `@username` mentions, then verify GitHub's native Contributors avatars on the public release page. Do not repeat historical credit on later releases just because they bundle the same Android APK or other existing work.
