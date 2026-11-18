# Explicit Tauri overrides

These files are selected with `--config` relative to the repository root:

| File | Purpose | Entry point |
| --- | --- | --- |
| `development.json` | Independent Dev product name and app identifier | `npm run tauri:dev`, `scripts/dev-app.sh` |
| `watch.json` | Same Dev identity with the local hot-reload frontend | `scripts/dev-app.sh --watch` |
| `ci.json` | Build test bundles without generating updater artifacts | CI and documented local CI builds |

Tauri merges these overrides into the base configuration. Paths used by the
base bundle still resolve from `src-tauri/`; moving an override does not move
icons, capabilities, signing inputs or output directories.

Keep `tauri.conf.json`, `tauri.windows.conf.json` and `tauri.linux.conf.json`
in `src-tauri/`: Tauri discovers those names automatically. Keep frontend tool
configuration in the repository's `config/` directory and Android configuration
in `android/`. Runtime preferences and credentials belong in private app data,
never in any of these repository configuration directories.
