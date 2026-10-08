# Optional profile language combinations

Service profiles currently share global recognition/translation languages. Separate
Apple Speech or cloud profiles cannot retain independent language combinations;
profile names and credential identities are not the cause of this behavior.

Each desktop profile can now store an optional nonsecret `languagePreset` containing
source and target languages. Missing/null preserves current language normalization.
Temporary language changes never write profile metadata.

The active configuration offers one “Remember current languages” action using the
existing language controls. A saved pair is shown with explicit Update/Remove
actions. Saving does not change current languages. Inactive profiles can remove a
pair but cannot capture another profile's current languages. Independent
development profiles follow the same editable rules. Configuration lists and
tray/overlay pickers show saved pairs alongside the existing names and provider
icons.

Ordinary profile selection applies the exact saved pair through the guarded
lifecycle. Settings can also reapply the active configuration after a temporary
language change. Explicit Apple resource language selection takes precedence and
uses the current target without rewriting the saved pair. Live/paused switches
preserve the existing pause/history and reconnect behavior.

A requested pair is validated against its route before saving. Later route edits
retain stale pairs; activation rejects an incompatible pair instead of silently
changing it or making the whole catalog unreadable. Apple resource readiness is
validated before activation and never triggers automatic downloads. Failed
validation or persistence leaves the previous selection intact. Nullable IPC
patches distinguish an omitted update from explicit removal.

This is desktop profile metadata, not a provider protocol or shared subtitle-policy
change. Android does not yet have a saved profile-language pair editor. Regression
coverage and native/browser verification limits are recorded in the
[integration ledger](../development/integration-runs.md).
