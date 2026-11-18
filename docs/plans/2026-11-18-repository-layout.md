# Repository layout implementation plan

**Goal:** Give desktop settings, native shell adapters and build configuration
clear owners without changing product behavior or the accepted subtitle UI.

**Architecture:** Preserve `src/`, `src-tauri/`, `android/` and `shared/` as the
platform boundaries. Keep portable rules in the existing shared crates. Split
desktop settings by responsibility behind its existing Rust module API, group
native shell adapters by platform, and separate explicitly selected Tauri
overrides from files that Tauri discovers by name.

**Tech stack:** Rust modules, Tauri v2 JSON configuration, React/TypeScript,
existing shell/Python/Node checks.

## Baseline and constraints

- Start from refreshed main `d2f3a822` in an isolated task worktree. Preserve the
  original checkout's dirty source, documentation and assets.
- PR #374 owns subtitle UI and native window interactions. Do not redesign or
  move its UI. Refresh main before committing and reconcile its entry-point
  changes without dropping either task's work.
- `settings_store.rs` has 13,050 lines: preference models, profile lifecycle,
  credential editing, legacy OS adapters, configuration assembly, persistence
  and tests. Its existing `file_credentials.rs` remains the local-file owner.
- Native Dock, quit, startup and updater-cleanup adapters are separate files
  scattered across the desktop source root.
- Explicit development/watch/CI overrides are mixed with Tauri's conventionally
  discovered base and platform configuration. Root frontend tool configuration
  is already grouped under `config/` and should remain there.

## Reviewable steps

1. Move only the explicit Tauri overrides into `src-tauri/config/`; update npm,
   signed preview, CI, contribution docs and the embedded identity regression.
   Retain `tauri.conf.json`, `tauri.windows.conf.json` and
   `tauri.linux.conf.json` at their discovery locations. Run launcher tests and
   configuration parsing checks before committing.
2. Convert `settings_store.rs` to `settings_store/mod.rs`. Extract preference
   models, catalog operations, credential operations, runtime configuration,
   legacy native-store adapters and atomic JSON persistence into named modules.
   Move existing tests with their responsibilities, retaining fixtures and
   feature/platform gates. Keep the existing external module API and narrow
   internal visibility. Run the settings suite before committing.
3. Group native shell code under `platform/{macos,windows,linux}/`; update
   call sites and source links. Keep native audio and window adapters with
   their owning features. Run the platform regressions before committing.
4. Maintain a concise directory/ownership guide with before/after examples and
   configuration loading rules. Reconcile latest main, run complete canonical
   verification and inspect the signed credential-free development preview.

## Verification and delivery

- Record test inventory before/after the settings split; move existing behavior
  and tests rather than inventing new behavior to simplify the layout.
- Run `CARGO_INCREMENTAL=0 ./scripts/check.sh` after final source/config edits,
  reusing canonical dependency and target caches. Run focused checks per step.
- Native preview must preserve the independent Dev identity. Coordinate app
  ownership before launching; do not interrupt another active session.
- Review exact outgoing commits, credentials/content/artifact exclusions and
  unchanged original-worktree state. This cross-module reorganization needs a
  concrete review before merging into main. Do not publish a release.
- Remove task-only temporary outputs after verification; retain source and
  review evidence until the branch is merged or handed off.
