# Repository ownership and configuration

## Platform and product boundaries

```text
src/                         Desktop React UI
  windows/
    overlay/                 Caption reading and overlay interaction
    overlay-control/         Quick session controls
    settings/                Configuration and saved-session UI
  components/                Controls shared across these surfaces
  lib/                       Frontend IPC, state, models and utilities
src-tauri/
  src/
    lib.rs                   Tauri composition, plugins, tray and app events
    commands.rs              Desktop IPC boundary
    settings_store/          Desktop preferences, profiles and credential storage
    session_manager/         Native session integration helpers
    session_manager.rs       Desktop session lifecycle coordinator
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

The frontend's three window folders already reflect product surfaces. Keep
their established UI and shared components together; a directory cleanup is
not a reason to change their interaction or duplicate state. The product
website lives in the separate `yuxino-labs/mimi-web` repository.

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
