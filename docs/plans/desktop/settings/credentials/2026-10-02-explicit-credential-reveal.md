# Explicit saved credential reveal

Explicit reveal reads credentials from the private local credential file. Normal
profile catalogs, settings snapshots, support diagnostics and events retain their
non-secret contracts. Revealing a saved field is a separate, explicit action.

`profile_reveal_credential` accepts a profile ID and a closed, known-provider
field enum (`apiKey`, `asrApiKey`, `token`, `secretId`, `secretKey`, `appKey`). Both
Tauri capability permissions and a runtime window-label guard restrict it to the
settings window. Store access runs in `spawn_blocking`, using the existing
profile-scoped local-file slots. The result is one string or null returned to
the invoking window; it is never broadcast or logged.

Only the requested field's existing slot is read. An ASR-key reveal does not
read the independent text-service destination. A destination-token reveal must
include the saved text route: official DeepL reveals its separate API key, and a
custom DeepLX destination reveals only its saved optional token. Unsaved route
switches, unrelated provider fields and write-only credential update payloads
cannot be revealed. Existing profile-scoped local slots remain authoritative;
reveal never reopens native-store import or reads environment credentials.

The frontend requires a separate “view saved” click. Revealed values stay in the
input component, remain hidden by default, never become a write draft, and are
cleared on cancellation, collapse, profile change or unmount. A request nonce
prevents late responses from populating another profile. Missing fields and store
failures use short, sanitized feedback.

Focused coverage checks provider/field projection, saved-route matching, missing
and corrupted fields, selected-profile isolation, authoritative-key migration
boundaries, ASR/destination independence, safe store failures and settings-only
permissions. Canonical checks and installed native acceptance are separate
claims; record both against the revision actually tested.
