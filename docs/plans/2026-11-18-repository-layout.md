# Repository layout decisions

## Repository-wide readability scope

The continued review covers React entry points and feature modules, desktop
Rust IPC/session adapters, the shared Rust runtime/core/JNI crates, Android
native UI/capture/persistence and their build/test configuration. The organizing
rule is a coherent responsibility and lifecycle, not a file-size threshold.
Normal entry points, mutable-state/resource owners, transitions, side effects
and real failure paths must be discoverable from source and the owning guide.

Keep the accepted subtitle UI and all wire/storage contracts intact. Prefer
direct feature imports over forwarding-only layers. Keep straightforward local
logic local, and retain genuine OS/network/storage/cancellation regressions.
No hypothetical failure or new abstraction is introduced just to split a file.
Desktop and Android continue entering the same shared business implementation.

Independent feature work is reviewed separately and integrated on one exact
source tree. Source moves retain the behavior inventory; lifecycle extractions
must make their ownership contract explicit and pass the existing caller and
boundary regressions before the combined acceptance run.

## Context

The clean main baseline `d2f3a822` kept 13,139 lines in `settings_store.rs`,
combining store construction, preference schema, catalog lifecycle, credential
editing, native compatibility, configuration assembly, atomic writes and tests.
Native shell adapters were scattered across the desktop source root. Explicit
Dev/watch/CI configuration sat beside convention-discovered Tauri files.

The existing frontend windows, Android source sets, shared Rust crates and
feature-specific native audio/window directories already express useful
boundaries. PR #374's accepted subtitle UI is now part of main `d5bd301c`;
the continued reorganization uses that baseline and preserves its interaction.

## Decisions

1. Keep `settings_store` as the public module boundary and use `mod.rs` for
   construction. Split operations into preference, profile, credential,
   configuration, storage-contract, legacy-native and persistence modules.
   Inherent `impl SettingsStore` blocks share the same state and locks; helpers
   remain visible only inside this module. Avoid new services or a new storage
   abstraction merely to divide a file. Existing persistence schemas, account
   names, write order, migration checkpoints and consent rules stay intact.
2. Put native application startup and lifetime adapters under
   `platform/{macos,windows,linux}`. Keep native audio and window interaction
   beside their features rather than moving every OS implementation into a
   generic platform tree. Preserve their platform/test compilation gates.
3. Put explicitly selected Tauri overrides under `src-tauri/config/` and update
   every caller. Keep convention-discovered base/platform configs, capabilities,
   permissions and packaging inputs at their existing locations. Keep frontend
   tool configuration under root `config/`, Android configuration in `android/`
   and portable fixtures/copy in `shared/`. Runtime app data is private and is
   unrelated to repository build configuration.
4. Keep normal frontend/store/IPC and Rust client/JNI entry points, while grouping
   actual implementations by product responsibility. Extract lifecycles with
   real state or resources: profile connection checks, per-input native capture,
   preview/final translation work, native probe registries and session teardown.
   Preserve the existing owners/locks and direct handler registration; do not
   manufacture forwarding services or a second reducer/transport.

## Trade-offs and review

The settings split adds explicit sibling imports and some `pub(super)` helpers;
this retains the original settings-module visibility without exporting those
helpers to unrelated desktop code. Existing tests move with their responsibility
and share synthetic fixtures. Linux smoke scripts and their child-process exact
filter must follow the new test module path or they cannot discover the test.

A universal top-level `features/` layout would add path churn across both
platforms and the accepted subtitle surfaces. Moving all Tauri JSON into one directory
would break automatic discovery. Splitting solely at a line limit would obscure
ownership, especially in shared provider modules whose protocols and regression
fixtures should be reviewed together. None of those alternatives is adopted.

The maintained map and loading rules live in
[repository ownership](../development/repository-layout.md). Validate source
moves with the existing focused behavior suites and complete canonical checks;
native startup evidence and Windows/Linux acceptance remain separate claims.
