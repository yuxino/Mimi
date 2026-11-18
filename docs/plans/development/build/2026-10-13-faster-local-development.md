# Faster local development

Routine UI edits currently rebuild the frontend, embed it in Rust, then sign,
install and restart the app before the user can see them. Full verification also
checks unrelated platform packaging and release tooling. These are separate
needs and should have separate entry points.

## Preview first

`./scripts/dev-app.sh --watch` keeps the stable signed `/Applications/mimi-dev.app`
and serves React/CSS through Vite on `127.0.0.1:1420`. `--ui-only` retains the
existing native isolation from credentials, providers and capture. Vite does not
load `.env`, an occupied port fails before installation, and stopping the runner
closes only its own server. No new dependency or IPC permission is needed.

The watch configuration points `frontendDist` at the local server, so frontend
edits do not rebuild Rust or write `dist`. Keep `tauri/custom-protocol` to reuse
native dependencies and preserve the bundle's macOS Dock icon path. Only the
watch configuration allows the local HMR connection and React preamble in CSP.
Ordinary `dev-app.sh` still embeds the frontend for offline
native acceptance. Rust/Swift, permissions and Tauri configuration changes need
an incremental signed rebuild; automatic restarts must not interrupt sessions.

## Check only the current edit while iterating

`./scripts/check.sh --quick frontend [source files...]` typechecks, lints and runs
related frontend tests, with cached lint for only the selected TypeScript files.
CSS-only edits run typecheck and diff checks plus native visual inspection;
they do not select unrelated behavioral tests. Without file arguments it runs
all frontend tests.
`--quick desktop [test filter]` checks the desktop Rust adapter;
`--quick shared` retains the shared architecture and Rust/JNI adapter checks.
When portable behavior changes, also run Android's `NativeRuntimeJniTest` through
Gradle; Rust adapter and APK-layout tests do not execute the JVM JNI boundary.
These report their actual scope and are not full acceptance.

Use the no-argument canonical check for cross-layer, build/signing, dependency
changes and release preparation. CI retains behavioral, credential, signing,
provider and desktop/Android parity coverage. Tests of the
production-output checker itself run there once, while every production build
still checks its real output. Remove only proven duplicate or obsolete tests;
test count alone does not establish wasted work.

## Validation

Verify runner cleanup and port-conflict rejection, build the signed UI-only app,
then change and restore a visible CSS value. Observe that the actual native
window updates with the same PID and signing requirement. Record warm timings
and any native proof gaps in the integration ledger. A cold native build still
costs time; no fixed latency promise is made for backend changes.
