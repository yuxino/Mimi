# AGENTS.md

## Project

Mimi is a Tauri v2 desktop app (Rust backend + React/TypeScript frontend) that listens to system audio and/or an explicitly selected microphone on macOS, Windows, or Linux and shows live translated subtitles in a floating always-on-top overlay. It supports built-in service profiles and custom live speech recognition with independent text translation, plus optional, explicitly enabled session recording and export.

Preserve these product constraints:

- Capture system audio by default. Microphone capture requires explicit selection and starting a session. Allow either or both sources, with independent capture, bounded queues, recognition sessions and subtitle state. Never mix sources, silently fall back, or request microphone permission for system-only sessions. Changing selected sources requires stopping and clears the audio-recording opt-in. Preserve source identity in subtitles and recordings.
- Subtitle history retention and selected-input audio recording are off by default. When enabled, save bounded confirmed subtitles and/or selected audio to private local session files as content arrives. Do not keep full session transcripts or PCM recordings in memory; only bounded overlay display content and size/limit metadata may remain there. New sessions and normal exit finalize the local files. Disabling an option clears its current-session content; saved sessions require explicit deletion.
- Store production API credentials in a dedicated private local file, separate from preferences and profile metadata. This explicit user request supersedes the earlier OS-keychain-only rule. Native credential stores are compatibility sources for automatic one-time import and verified retirement of Mimi-owned old items; never touch them in normal snapshots, reads, saves, deletes, switching or updates after import. Delete old items only after durable local write and read-back verification; checkpoint successful slots and cleanup for retry. Use atomic writes, Unix 0700/0600 protection and private Windows ACLs. Keep keys out of environment variables, source control, diagnostics and exports. Development uses the same editable local-file design with its own app-scoped directory. Do not read `.env`, create environment-backed presets, or automatically share production configuration or credentials.
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
- `src-tauri/src/settings_store.rs`: preferences/profile JSON in the app config directory + provider/profile-scoped private local credential storage.
- `src-tauri/src/{commands,windows,lib}.rs`: IPC commands, overlay/tray-panel window management, tray/shortcut wiring.
- `src/`: React frontend — `src/windows/{overlay,tray-panel,settings}/` contain the three product surfaces; `src/lib/{ipc,store,types,i18n}.ts` define the IPC contract.
- The product website lives in the separate `yuxino-labs/mimi-web`
  repository. Never add a website copy, subtree, or generated site assets to
  this application repository.
- `README.md`: the single English entry point. Localized READMEs live in
  `docs/readme/`; contribution and security guides live in `.github/`.
- `docs/plans/`: current accepted design records; completed checklists and
  superseded designs stay in Git history.
- `docs/development/common-regressions.md`: required macOS signing, permission,
  Keychain, overlay, and local-testing pitfalls. Read it before packaging or
  diagnosing a repeated system prompt.
- `docs/development/ui-guidelines.md`: shared layout, action-feedback rules and
  the cross-page review checklist. Read it before changing a product interface.
- `docs/development/integration-learning-loop.md`: the ongoing integration,
  sample comparison and regression-learning workflow. Read it before integration
  testing or investigating subtitle quality; keep verified runs and remaining
  gaps in `docs/development/integration-runs.md`.
- `scripts/check.sh`: canonical automated test and strict-build entry point.
- `scripts/package-app.sh`: release-shaped local build via `tauri build`, signed with the stable local identity; compare its identity before replacing a release, especially older ad-hoc installations.
- `scripts/codesign-identity.sh`: honors explicit `MIMI_CODESIGN_IDENTITY`, then a per-Mac public certificate fingerprint saved after an approved local migration; otherwise it selects the exact fingerprint of the unique `mimi Local Development` identity or reports unavailable. A missing/invalid pinned identity fails closed rather than reverting to self-signing. macOS packaging and development launch fail closed rather than use ad-hoc signing.
- `scripts/prepare-macos-release.sh`: prepares public macOS assets on the signing Mac using the certificate pinned in `scripts/macos-release-identity.txt`. Keep the private key local. Tag CI verifies the staged draft assets, source revision, updater signature, and DMG; never restore an ad-hoc fallback. See `docs/development/macos-release-signing.md`.
- `scripts/verify-macos-install-identity.sh`: compares the complete designated requirement before a formal app is replaced.
- `.github/workflows/ci.yml`: CI (Rust fmt/clippy/test on macOS, Windows, and Linux, frontend checks).

## Working agreements

- Follow [UI consistency and feedback](docs/development/ui-guidelines.md). When fixing a repeated UI pattern, find and review its sibling controls across settings, tray panel and overlay; cover success and failure paths, including silent save rejections. Reuse shared components and design tokens. Do not call the sweep complete after checking only the reported page.
- Interactive controls with hover feedback must keep a pointer cursor across the overlay, control panel, tray panel and settings, including help controls. Preserve disabled/busy, text-input, slider and resize cursors. On macOS, verify the nonactivating overlay's native cursor path as well as CSS; a successful native cursor update does not guarantee WebKit will retain it on later movement.
- Keep explanatory copy out of persistent small-print paragraphs. Use compact help icons with hover/focus tooltips for non-essential descriptions, protocol requirements, and storage details. Keep field labels, essential choices, and actionable errors visible at the normal interface text size. Do not add small text merely to fill space or explain an otherwise clear control.
- Keep related action buttons compact, consistent, and right-aligned. Use existing icons. Configuration deletion uses a red destructive action and a standard confirmation dialog, never an expanding inline strip. Choosing a service type must not create a profile until the user confirms adding it. Display connection-check progress and results with the triggering action, including actual request duration; recognition and text translation have independent checks.
- One-time operation feedback (copy, refresh, save, export, delete, quit, or up-to-date checks) belongs in the shared transient toast, never a full-width banner or a persistent paragraph that moves page content. Instant preference changes save quietly on success and report sanitized failures through that toast; do not swallow save rejections. Keep one toast per settings window, including inside an active modal, replace repeated notifications, and clear on navigation, native/DOM blur, close, hide and unmount. Keep unsaved-field validation/retry, connection-check results, ongoing storage failures and update actions visible beside their controls.
- Read the relevant source and tests before changing behavior. For non-trivial behavior changes, add or update a design note in `docs/plans/`.
- Keep UI-independent logic in `src-tauri/src/core/`; keep Tauri, window, keyring, and OS-audio integration in the app-layer modules. Never import `tauri` types in `core/` or `clients/`.
- Preserve Rust concurrency safety. Isolate mutable network or lifecycle state behind `Arc<Mutex<…>>` or actors; never hold a `std::sync::MutexGuard` across an `.await`.
- Treat streaming drafts as replaceable previews and final events as durable subtitle history. Do not let preview work block, reorder, or overwrite final translations.
- Keep queues and on-screen draft growth bounded. Latency fixes must account for cancellation, reconnects, stale generations, empty results, and out-of-order completions.
- Add focused `#[cfg(test)]` coverage when changing `core/`. `cargo test` runs the repository's suite.
- The wire protocols (JSON shapes, model names, domain prompts, filler glossaries) mirror the upstream services exactly; do not reword the translation prompts.
- Do not introduce a dependency, external service, or credential requirement unless the task needs it and the trade-off is documented.
- On macOS use `/Applications/mimi-dev.app` for all pre-push testing. Never overwrite `/Applications/mimi.app` with a locally signed package when its designated requirement differs from the installed GitHub Release. An intentional certificate migration must be explicit and is expected to require one final Screen Recording and Keychain authorization.
- Normal credentials use the local file only, including missing or damaged values. Never fall back to native stores after migration. Keep a non-secret completion marker so a missing file cannot reopen OS authorization. Legacy import verifies each durable copy before deleting only its original Mimi-owned OS item; checkpoint deletion separately for interrupted cleanup.
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

Follow [the integration learning loop](docs/development/integration-learning-loop.md)
for native integration and subtitle-quality work. Reuse comparable samples, trace
the earliest evidenced failure, repair that boundary, and rerun the same case.
After a meaningful run, update the concise run ledger with the exact revision,
build mode, evidence, limitations and next unresolved check. Promote verified,
reusable lessons into the owning guide and focused regression tests; do not turn
one successful sample or an interrupted check into a general acceptance claim.
At the end of related development or quality-investigation conversations, carry
forward new evidence and explicit durable user corrections. Keep unconfirmed
feedback as pending; add no duplicate lesson when nothing new was established.
When an error recurs, strengthen its concrete prevention rule or automated check.
Keep private media, transcripts and credentials outside Git. This maintenance
rule does not authorize new capture, paid batches, recording or publication.

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
