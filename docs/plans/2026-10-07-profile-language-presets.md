# Optional profile language combinations

**Goal:** Let saved service profiles optionally restore a recognition/translation
language pair without changing existing global language controls.

**Architecture:** Store an optional nonsecret `languagePreset` on each desktop
profile. Missing/null means keep current language normalization. Ordinary profile
selection applies the exact saved pair through the existing guarded lifecycle;
explicit Apple resource language selection takes precedence and uses the current
target. Live/paused switches validate before persistence, preserve pause/history
and reconnect as before. Temporary language changes never write profile metadata.

**Tech stack:** Rust/Tauri settings and lifecycle; React/TypeScript settings controls.

1. Add typed metadata and explicit nullable IPC patch. Preserve legacy catalogs,
   names, credentials and proxies. Validate a requested pair against its route;
   retain stale presets when a route later changes, rejecting selection instead
   of silently changing the saved pair or making the whole catalog unreadable.
2. Apply presets on profile selection (including using the current profile again from Settings
   after a temporary language change). Require Apple resources before switching;
   never download automatically. Failed validation/write leaves the old selection.
3. Add one compact “Remember current languages” action beside a saved-pair
   summary, with explicit Update/Remove actions. Reuse existing current-language
   controls; do not add a second pair of pickers or a switching dialog. Saving
   does not change current languages. Show saved pairs in the configuration list.
   Capture is available for the active profile; inactive profiles can clear a pair. Local development
   credential presets remain read-only.
4. Cover independent Apple/Alibaba profiles, legacy behavior, restart persistence,
   temporary changes, same-profile reapplication, route incompatibility, explicit
   Apple overrides and save failure rollback. Check IPC patch omission vs clearing,
   save acknowledgment, errors, busy/unmount and Chinese/English/Japanese layout.
5. Run `./scripts/check.sh`, inspect the signed canonical UI-only development app,
   review the diff, then open a PR. Do not merge or reply to issue #183.

This is desktop configuration metadata, not a provider protocol or shared
subtitle-policy change. Android does not yet have this profile-language editor.
