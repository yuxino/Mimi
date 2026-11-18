# Android audio inputs and saved sessions

**Goal:** Complete issue #316 with explicit microphone capture and private, opt-in caption sessions that remain available after Stop.

**Architecture:** Android owns permissions, capture and native screens. Each selected audio source has an independent capture buffer and shared Rust runtime handle. Both platforms use the same bounded disk journal, subtitle aggregation and export formatter; JNI exposes native lifecycle and file operations.

**Technology:** Kotlin native Android UI, AudioRecord and MediaProjection, Rust shared runtime/core, JNI, Android Storage Access Framework.

## Decisions

- Keep system playback as the default; offer System, Microphone and Both. Microphone-only never asks for a projection token. Both has independent streams, recognition state and source labels, never mixed PCM or silent fallback.
- Keep Android's required RECORD_AUDIO permission for playback capture; this Android permission name does not imply that a system-only session opens a microphone. Use the foreground-service types matching the selected sources.
- Source changes require stopping. Existing permission revocation, lock-screen and capture failure policies stop all selected lanes and release resources.
- Saving captions is off by default and independent of the bounded recent-caption display preference. Append accepted confirmed original/translation pairs to private local files as they arrive. Do not derive persistence from UI polling or store an entire transcript in Kotlin memory.
- Stop drains accepted work before sealing the journal. Disable saving clears the current session's saved text. Saved sessions survive normal restarts and require explicit deletion. The bounded current confirmed display remains readable after Stop until a new session begins.
- Expose a native saved-session list and paged reading, TXT/Markdown export through the system document picker, and confirmed deletion. Export uses the existing shared formatter and preserves source identity.
- Reuse the existing desktop journal through a shared Rust module rather than introducing a second archive format or Kotlin provider engine.

## Implementation and checks

1. Capture adapter: update `MimiService`, the start/permission flow, manifest and source selection preferences. Add capture-selection and actual AudioRecord fixture coverage, including both lanes, stop and permission failure.
2. Shared persistence: move portable `session_history` into `shared/mimi-runtime`, retain desktop re-exports, and connect confirmed reducer output to the JNI journal. Verify disabled/default, append/finalize, limits, two-source identity, deletion/export and interrupted-session recovery.
3. Shared snapshot aggregation: reuse the desktop aggregation for independent Android handles, with bounded combined history and Stop presentation. Exercise direct Rust and actual JNI boundaries.
4. Native UI: source selector, opt-in saving, saved-session list, paged reader, export and delete, localized in the existing eight interface languages. Verify empty/error states and that no preview starts provider/capture activity.
5. Acceptance: inspect the signed desktop development app and Android emulator/native fixtures; run `./scripts/check.sh`, full Android host JNI tests and debug/release compilation. Review privacy and concurrency boundaries independently.
6. Delivery: combine with the already implemented provider-mode regression repairs, wait for desktop and Android CI, merge to main, then prepare and test the signed multi-platform release. Publish only verified assets and bilingual notes; verify public downloads/updater and clean superseded task outputs.

## Evidence limits

Synthetic/native fixtures establish UI, capture routing, journal and lifecycle behavior. They do not establish real provider translation quality or physical-device microphone acoustics; report those checks separately.
