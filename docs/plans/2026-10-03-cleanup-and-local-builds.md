# Cleanup and faster signed development builds

Remove code only after checking production references, including dynamic names
and same-file calls. Keep provider protocols, credentials, capture, bounded
queues, cancellation and final-subtitle regressions intact.

The display-only provider-language metadata module has no product consumers.
Remove it and tests of its unused API; retain the Rust model/Lite-table parity
assertion in the actual route-aware capability tests. Remove the old unused
stream-word segmentation helpers, phase opacity/amplitude fields and color
helper, unused translation-mode label table, and duplicate capability tests.
The shared settings header test keeps all lifecycle transitions, while category
and locale coverage stays in SettingsNavigation. Remove the redundant cross
product of lifecycle/category/locale renders and recovery updates to which
SettingsView does not subscribe; these tested a static header repeatedly.

Move update-environment detection out of the React component module. The
component then supports Fast Refresh without a mixed-export warning, and
portable-distribution tests no longer load the settings UI dependency tree.
Preserve the distribution checks and their failure cases.

The signed development launcher previously used production size optimization,
single codegen unit and full LTO. Use a separate `local-dev` profile inheriting
Cargo's development settings, with incremental compilation and line-table debug
information. Embed the frontend with `build:dev`, skipping repeated type/icon
checks and minification and including source maps for the WebView inspector.
Production builds and the canonical check retain their existing verification
and optimization. The fixed dev identifier, signing identity, install recovery
and canonical `/Applications/mimi-dev.app` path remain in effect. Honor an
explicit `CARGO_TARGET_DIR` when copying the compiled binary into its bundle.

Verification: run canonical checks, build/install/launch the signed UI-only dev
bundle and verify its process path and designated requirement. Measure cold and
warm builds separately; the new profile needs its own first dependency build.
Keep reusable caches and intermediate artifacts. Limit the default frontend test
run to two isolated workers: the suite has many jsdom windows, and unrestricted
parallel heaps contend with native compilers during local work. Preserve test
isolation and the existing deadlines; do not hide failures by extending them.
