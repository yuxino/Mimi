# Runtime resource boundaries and Android fixture isolation

## Baseline and constraints

Start from clean main `bd778732`. Use offline synthetic cases and background
checks; do not open native windows, capture audio, read private configuration or
replace installed applications. Keep implementation and independent regression
review separate. Merge only after relevant platform checks pass.

## Android historical fixtures

Sixteen historical Kotlin runtime files remain in `src/main` even though live
production sessions enter `SharedRuntimeEngine` and native Rust. Release builds
do not enable shrinking, so test-only protocol engines and clients remain part
of packaged code. The architecture checker currently exempts these whole files.

Move the existing implementations into the JVM test source set without changing
their behavior or deleting their tests. First separate the few actual native-UI
dependencies: translation probe result/cancel types, DeepL endpoint validation,
and OpenAI service metadata. Preserve their values and validation exactly.

Remove production-file exemptions from the shared-runtime guard and add
rejection cases for old filenames and metadata accesses. Extend actual APK
verification to reject the legacy runtime class descriptors in every DEX file.
Preserve Rust/JNI, ABI, alignment and notice checks. This is a source ownership
and packaged-code cleanup, not a claimed reduction in live process RSS.

## Saved-history ownership across asynchronous reads

`session_history_list` snapshots the current recording ID before awaiting its
blocking file scan. A session started during that scan can be returned as saved
history, while a session finalized during the scan can remain incorrectly
excluded. Read the current owner after the scan, immediately before filtering.
Use the same command helper in deterministic tests with synthetic files and a
barrier around the scan. Cover both a new active session and a newly finalized
one; other saved history and deletion behavior stay unchanged.

## Investigation and acceptance

Audit cancellation/reconnect and native resource lifetimes independently. Add a
behavioral fix only after a concrete offline failure is reproduced; record that
case and the smallest ownership boundary repaired here before implementation.

Run focused failures before and after repair, then canonical checks. This Mac has
no configured JDK; validate Kotlin, actual JVM/JNI and both APK variants in CI
before merging. Keep synthetic checks distinct from real-device and long-session
acceptance, and record final evidence in the integration ledger.
