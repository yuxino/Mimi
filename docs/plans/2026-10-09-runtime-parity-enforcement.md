# Shared runtime maintenance enforcement

The user requires future PC subtitle/provider fixes to reach Android without
another manual copy. Both production paths already build `shared/mimi-core` and
`shared/mimi-runtime`; this change makes known ways of breaking that relationship
fail local checks and CI.

## Boundaries

- Desktop portable core/protocol/client files and AudioSendPipeline stay pure
  re-exports. Module lists cannot redirect them with a `path` attribute or contain
  local implementations. A new desktop provider client needs shared source.
- Android's production session enters SharedRuntimeEngine from MimiService.
  New Java/Kotlin provider engines/transports and references to legacy clients
  fail the guard. Historical Kotlin wire fixtures retain an explicit file list;
  their engine calls cannot become a production fallback. The old independent
  text-client factory is moved intact to `src/test`.
- Both Cargo dependency graphs use repository paths to the same shared crates.
  Gradle native inputs also track future `build.rs` and Cargo configuration.
- The architecture guard runs from canonical/shared checks, Android preBuild,
  host/ABI builds, direct build-shared-core and desktop CI's unconditional scope
  job. Android pull-request/main CI triggers on desktop backend/frontend source,
  shared source, scripts and workflows, retaining debug/release/JNI/APK checks.

This is a source architecture guard, not a Rust/Kotlin parser or a semantic parity
proof. It covers the concrete fork patterns tested by mutations. Platform-native
capture, credentials, trust/proxy, lifecycle and rendering stay native. A desktop
UI feature still needs Android applicability review and native acceptance.

## Verification

Run `python3 -B scripts/check-shared-runtime.py` and
`python3 -B scripts/check-shared-runtime-test.py`. Regressions change real source /
build inputs in disposable directories: local desktop logic, redirected modules,
legacy fallback/import alias, independent Kotlin/Java engines and transports,
external Cargo runtime, omitted shared build inputs and narrowed/removed CI gates
must fail. Comments, literal labels, test-only fixtures, native adapters that
accept ProviderEngine and changes solely in shared Rust remain valid.

Before local acceptance run canonical, Android debug/release JVM/JNI/lint and APK
checks. After integration, confirm both CI workflows execute for a shared or PC
source change. This cloud follow-up does not rerun the old delivery's tests or
claim remote CI, Gradle compilation, new APK or device acceptance.
