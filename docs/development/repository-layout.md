# Repository ownership and configuration

## Platform and product boundaries

```text
src/                         Desktop React UI
  windows/
    overlay/                 Caption reading and overlay interaction
    overlay-control/         Quick session controls
    settings/                Settings window and shared settings controls
      services/              Profiles, credential drafts and connection checks
      audio/, appearance/    Input/language and subtitle appearance controls
      history/, updates/     Saved sessions and software updates
      support/, development/ Diagnostics and explicitly enabled dev tools
      guide/                 Onboarding and setup guidance
  components/                Controls shared across these surfaces
  lib/
    ipc.ts, types.ts          Native IPC contract and payload types
    store.ts, i18n.ts         Snapshot/acknowledgement and language entry points
    audio/, providers/       Input/capture and provider/translation capabilities
    profiles/, session/      Catalog projection and session state/errors
    subtitles/, overlay/     Caption presentation and native overlay helpers
    settings/                Preference-state projection
src-tauri/
  src/
    lib.rs                   Tauri composition, plugins, tray and app events
    commands.rs              IPC state and stable payload entry point
    commands/                Settings, profiles, session, overlay and app handlers
    settings_store/          Desktop preferences, profiles and credential storage
    session_manager.rs       Desktop session state owner and construction
    session_manager/         Lifecycle, lane ownership, selection and publication
    audio/                   Native capture and audio adapters
    windows/                 Native overlay/window interaction
    windows.rs               Window-management entry point
    platform/
      macos/                 Dock presentation and native quit delegate
      windows/               Startup ownership and updater cleanup
      linux/                 X11 initialization before Tauri/GTK
    core/, clients/          Shared re-exports and desktop-specific adapters
  config/                    Explicit Dev/watch/CI Tauri overrides
android/                     Native Android UI, capture, credentials and build
shared/
  mimi-core/                 Common subtitle state and alignment
  mimi-runtime/              Common providers, transport and session policies
  mimi-android-jni/          Android bridge to the same Rust implementation
  *.json                     Shared copy, catalogs and behavioral contracts
config/                      Frontend build/type/style/lint tool configuration
scripts/                     Build, signing, checks, probes and maintenance tools
```

Group a native adapter with the feature that owns it: audio capture belongs in
`audio/`, overlay pointer handling belongs in `windows/`, and shell startup or
application lifetime belongs in `platform/<os>/`. This avoids a second generic
platform tree that would separate every feature from its OS adapters. Keep
Tauri types out of the shared crates and the desktop `core/`/`clients/` modules.
See [platform parity](platform-parity.md) for the shared runtime boundary and
intentional Android differences.

The three frontend windows remain normal product entry points. Settings feature
folders and `lib` domains organize implementations; they do not introduce barrel
files or a second store. The product website lives in the separate
`yuxino-labs/mimi-web` repository.

## Frontend state and configuration editing

`store.ts` owns native bootstrap, event subscription, snapshot ordering and save
acknowledgement. Its browser-only catalog projection has explicit inputs/outputs
in `lib/profiles/browserProfilePreview.ts`; it does not imitate native lifecycle
or credential storage. IPC names and types remain discoverable at `lib/ipc.ts`
and `lib/types.ts`.

The services page coordinates profile actions. `ProviderCredentialEditor`
owns the credential draft/form boundary, `ProviderPicker` owns add confirmation,
and `useProfileConnectionChecks` owns the mounted page's request IDs, timeout,
stage results and invalidation when credentials, route/proxy or session state
change. Rendering copy is projected by `serviceProfilePresentation`; no extra
store, transport or business-rule implementation is introduced.

The existing window layouts, CSS cascade, language keys and generated-copy
entry points remain unchanged. Source moves must also update template-string
dynamic imports, mock paths, test fixtures and Gradle's generator inputs.

## Desktop IPC and session ownership

`commands.rs` owns `AppState` and stable snapshot/navigation type names. The
actual handlers live in `commands/{settings,profiles,connection,session,overlay,
app,help,apple_resources}.rs`. `lib.rs` registers those paths directly with
`generate_handler!`; the external command names, payloads and capability
permissions remain the same. Bootstrap/save and post-save native feedback stay
together in `commands/settings.rs`. The app module owns navigation and the
normal stop/finalize/exit boundary.

`session_manager.rs` constructs one `SessionManager`, which retains its original
shared controller, locks and state. Inherent implementation blocks divide the
actual work without adding forwarding methods or additional session owners:

| Module | Owns |
| --- | --- |
| `lifecycle.rs` | Manual start/stop/pause/resume, establishment and failed-start/resume handling |
| `lanes.rs` | Each source's capture/client/pipeline/pump resources, generation-tagged installation and retirement |
| `ownership.rs` | Request epochs, admission guards, generation-bound work and teardown gates |
| `selection.rs` | Transactional audio/profile/language/mode changes and their conditional reconnect |
| `events.rs` | Provider application under the content gate and shared per-source translation deadlines |
| `recovery.rs` | Native failure handoff, health tasks and generation-owned recovery admission |
| `history.rs` | Opt-in file append/finalization, recording metadata, bounded paging and export |
| `publication.rs` | Payload projection, ordered snapshots and coalesced native window publication |
| `diagnostics.rs` | Content-free capture observations, status/failure journal and support facts |
| `fixtures.rs` | Explicit credential-free UI fixtures, without native capture or provider connections |

Keep portable recovery/provider/subtitle policy in the shared crates. Preserve
the content gate around Clear/application/publication, the cleanup owner that
survives a cancelled waiter, generation/task checks, and final drain ordering.
The module owning an operation must show its real side effects and failures;
do not replace those operations with a new layer of pass-through calls.

## Shared Rust and Android adapters

`mimi-runtime/clients/high_quality_client.rs` remains the public high-quality
client and owns one `Arc<Mutex<PipelineState>>`. Its private `recognition`,
`preview`, `final_translation`, `request_budget` and `text_translation` modules
implement the respective flows. Replaceable previews and durable ordered finals
retain their separate work/ownership while sharing the existing request budget.
The desktop facade still re-exports this exact implementation.

`mimi-android-jni/src/runtime.rs` retains the operation dispatcher, session
handles, PCM and connect/finish lifecycle. `runtime/configuration.rs` owns native
configuration adaptation, `contracts.rs` owns offline contract views and
`probes.rs` owns the independent bounded probe registry. `StageNetworkInput`
resolves the same immutable speech/text network snapshot for session and probe
creation. JNI names, operation strings, size limits and wire schemas stay stable.

Android's `captions/` package owns native caption rendering, typography, layout
and preview. The app/activity, native resources and shared runtime entry remain
in their existing adapters. `capture/CaptureLane.kt` owns an input's recorder,
worker, capture state and health observations; `MimiService` still owns generation
checks and prepares all selected lanes before starting any. It stops every
recorder before joining/releasing lanes and finishing their shared runtimes.
`prepare()` declares its already-granted `RECORD_AUDIO` requirement; a revoked
permission still propagates through the service's existing startup cleanup.

## Desktop settings

`settings_store/mod.rs` constructs the store and exposes the existing public
types and methods. Its `impl SettingsStore` blocks are divided by responsibility:

| Module | Owns |
| --- | --- |
| `preferences.rs` | Non-secret preference schema, defaults, language normalization and preference writes |
| `profiles.rs` | Catalog validation, profile creation/edit/selection/deletion and metadata rollback |
| `credentials.rs` | Credential editor/reveal operations, independent destinations and lookup cache |
| `configuration.rs` | Listening configuration and independent speech/text draft or saved-configuration probes |
| `secret_store.rs` | Storage contract, availability state and sanitized error mapping |
| `file_credentials.rs` | Private local credential document and verified legacy import/retirement |
| `native_credentials.rs` | Compatibility adapter to old native stores; only legacy import uses it in production |
| `persistence.rs` | Atomic file replacement, directory sync and identical-preference-file checks |
| `tests/` | Synthetic behavior regressions grouped by responsibility, with common fixtures in `mod.rs` |

Keep locking, write order, read-back verification, migration checkpoints and
rollback in the owning operations. The split does not introduce another store,
change persisted JSON or file names, or grant normal credential operations
access to native stores. Internal helper visibility stays within
`settings_store`; callers continue using `settings_store::SettingsStore`.

Feature-gated paid/manual probes retain their explicit opt-in and app-scoped
local-file restrictions. Linux Secret Service smoke scripts name the relocated
tests exactly and still reject a missing test before creating their isolated
synthetic keyring.

## Configuration by purpose

| Location | Loading rule |
| --- | --- |
| Root `package.json`, lockfile, `index.html`, `tsconfig.json` | npm/browser/editor entry points |
| `config/` | Frontend tools selected by npm scripts; root `tsconfig.json` discovers its projects |
| `src-tauri/tauri.conf.json` | Tauri's automatically discovered base configuration |
| `src-tauri/tauri.{windows,linux}.conf.json` | Tauri's automatically discovered platform overrides |
| `src-tauri/config/{development,watch,ci}.json` | Explicit `--config` overrides; see [loading details](../../src-tauri/config/README.md) |
| `src-tauri/capabilities/`, `permissions/` | Tauri's native security inputs, kept at their discovery locations |
| `src-tauri/{Info,Entitlements}.plist`, icons and installer theme | Platform packaging/signing inputs; their existing paths and identities are preserved |
| `android/` | Gradle, Android manifest, resource and native build inputs |
| `shared/*.json` | Portable provider/copy/fixture inputs; generated platform resources follow their existing generators |
| `.github/workflows/` | CI and release automation |
| Private app data | Runtime preferences/profile metadata and a separate private credential file; never repository config or `.env` |

Do not move convention-discovered files merely to make a folder smaller. An
explicit configuration move must update its npm/script/CI entry points, native
embedded test references and documented commands. Keep canonical target/cache
locations; a source-directory change does not require a new dependency cache.

## Verification

Use `./scripts/check.sh --quick desktop settings_store` while iterating on the
store. Native shell regressions run under `platform::`; Linux compatibility
scripts still require Linux and their private synthetic Secret Service.

After cross-module or configuration changes, run the complete
`CARGO_INCREMENTAL=0 ./scripts/check.sh`. Coordinate the signed Dev app and
port 1420 before switching worktrees or running the watch-server regression.
Use the actual Dev bundle to verify startup and local state persistence; passing
source tests does not establish Windows/Linux native acceptance or live provider
quality. See [local regression rules](common-regressions.md).
