# Android service profiles

Android currently keys speech credentials by provider and text translation settings globally. Replace that singleton model with an ordered catalog of independent profile identities, matching the desktop concept while keeping native phone controls.

## Behavior

- Show saved configurations, not one fixed row for every provider. Support multiple configurations for the same provider, each with its own name, speech credentials, text-translation drafts and model choice. Keep a stable saved order and mark the active entry without pinning it.
- Empty state opens the provider picker. Adding opens a local editor draft; only successful Save persists it. Back and cancellation leave no empty profile. The first saved configuration becomes active; later saves do not silently select it.
- Tapping a configured list entry selects that exact profile, using native single-choice semantics; tapping the selected entry leaves it selected. Settings edits it without selecting it; duplicate and confirmed delete live in the editor’s More menu and target profile IDs. A duplicate has independent credential storage and never changes selection. Deleting the active entry selects the first remaining entry, or leaves the catalog empty. Limit the catalog to 20.
- Keep all profile mutations unavailable during capture/stop, with the existing stop action. Clear save failures must preserve the draft. Missing/corrupt storage is an error, never a fresh empty catalog.
- Show names and essential missing-state feedback; omit routine credential-saved subtitles. Reuse the existing Android list: a 40 dp icon slot, 16 sp name, 78 dp minimum row and the existing Settings action on the right. Put Add beside the heading; do not spread actions across a second row. Use the native monochrome radio indicator before each provider mark. Only the active entry is checked and its name bold; omit the redundant active text label. Expose the checked state to accessibility. On phones one profile remains a compact row with an explicit editor, so Back cannot immediately reopen the activity. Large text retains the existing stacked fallback; long names wrap to two lines and remain complete in accessibility labels.

## Migration and boundaries

Migrate existing provider settings once, including partial configurations and independent translation drafts; do not manufacture entries for untouched providers. Preserve the previously selected configured provider and appearance/language preferences. Store metadata separately from private credential generations. Commit/read back credentials before atomically switching the catalog; retire legacy credential values only after durable verification. Failed/corrupt migration must not lose old settings or silently substitute other credentials.

Production capture resolves the selected profile into the existing shared Rust runtime. Legacy provider-keyed fixture helpers remain compatibility entrypoints only. Kotlin must not duplicate provider networking, subtitle rules, model or language policy.

## Alibaba modes

Expose simultaneous translation and ASR + MT with a workspace ID for simultaneous translation. Existing Android configurations remain ASR + MT. Mode-specific languages, validation and wire behavior come from shared Rust through JNI. Independent text-translation fields are hidden/ignored during simultaneous translation but retained for switching back. Home, language selection, model labels and caption provider marks reflect the effective mode.

## Acceptance

Cover migration from multiple providers, original shared-key ownership, partial settings, no-configuration startup, corrupt/failed writes, independent same-provider keys and MT settings, copy/delete/selection, 20-entry limit, active-session rejection and cancelled drafts. Run direct Rust and actual JNI mode checks, Android JVM tests/lint/builds, and native offline profile/UI instrumentation with zero/one/several profiles, narrow layouts, large fonts and supported languages. No real-provider or playback acceptance is implied by offline UI tests. Update platform parity and bilingual release notes only to match implemented/verified behavior.

## Shared explanations

Service capability, credential prerequisites and Alibaba mode copy come from `shared/provider-copy.json`, in eight languages. A deterministic generator feeds desktop TypeScript and Android XML; frontend/shared/native checks reject stale outputs. Platform navigation and permission instructions remain local. Setup guides cover both entry paths without duplicating common requirements.

An explicitly requested one-time development transfer uses a debug-only instrumentation importer. It reads a private, bounded temporary file, appends supported profiles under deterministic IDs, skips repeat imports, and deletes the transfer file. It never connects providers or becomes automatic production/development sync.
