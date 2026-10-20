# History summaries, live subtitle projection and obsolete credential paths

## Scope

Continue from `c0e6284d7148ad537ce1517fc9c8b12d738685cf` using offline,
synthetic fixtures and background checks. Do not launch native windows, capture
audio, inspect private configuration or replace installed applications.

## History list

`SessionHistory::list` currently reads each complete `SavedTranscript` only to
return timestamps, entry count, limits and audio-source availability. Parse a
small summary instead, validating subtitle field types without retaining owned
source/translation strings or the full entry vector. Reuse one line buffer.

Keep the existing JSON/JSONL precedence, size limit, recovery after malformed
journal lines, UTF-8 stop behavior, count limit and timestamp semantics. Do not
add a persistent index, cache invalidation policy or file-format migration.
Compare both readers against the same synthetic finalized and interrupted
files. Measure read costs separately from native UI latency and process RSS.

## Live subtitle projection

The multi-source overlay currently builds every per-source history block and
then discards them to select only each live tail. Derive the live block directly
from the tail and last timestamp; share that helper with the single-source path.
The already merged/cropped public history remains the only history projection.

Preserve owner IDs, source order, duplicate text, empty-tail behavior and display
modes. A regression should show that per-source historical text is never read
when projecting live tails, while checking the resulting blocks.

## Dead code and tests

Remove the unreferenced `isChineseSystem` and `LEGACY_SOURCE_LANGUAGE_CASES`
exports after checking all production, test, script and fixture references.
Keep the development module selected dynamically by the Vite release alias.

Remove `SettingsStore`'s old native-to-native Alibaba migration branch. Its flag
is false for all production construction paths; only historical fake-store tests
enable it. Current one-time native-to-file migration belongs exclusively to
`FileCredentialStore` and must remain unchanged.

Preserve native import adapters, compatibility service identifiers, write/read
verification, retry/retirement checkpoints and file-loss protections. Retire
tests that exercise only the unreachable policy; move the useful blank-slot
fallback case to the actual importer before removing its old test. Preserve
cached-access failure and production/development isolation coverage.

## Verification

Run focused behavioral tests, then the canonical check on the combined tree.
Use cross-platform CI for platform compilation and actual Android/JVM/JNI
checks. Record exact synthetic measurements and remaining native/long-session
limits in the integration ledger; do not claim an application-wide memory or
frame-rate improvement from a local projection/reader comparison.
