# Frontend toolchain upgrade

Use Vite 8.3.8, its React plugin 6.3.0 and Vitest 5.1.0 together. Keep
TypeScript, ESLint, jsdom, React and Tailwind on their current versions so the
bundler/test-runner migration can be evaluated independently. The dependency
graph also updates PostCSS within its existing declared range.

- Keep the explicit Safari 16, Chromium 105 and ES2021 compilation targets.
  Vite 8's newer default browser target must not raise Mimi's runtime baseline.
  Keep `.env` loading disabled and production debugger modules excluded.
- Move WebSocket connection options to `server.ws`, retaining the same host
  and ports. Disable Vite's new automatic browser-console forwarding so agent
  environments do not change the development logging boundary.
- Require Node 22.x (22.13+), 24.x or 26+ for development. Vitest 5 drops Node 20;
  this changes contributor tooling, not the installed application's requirements.
- Keep test isolation and the two-worker limit. Set `clearMocks: false`
  explicitly to preserve the prior lifecycle, and discover tests under `src`.
  Type test doubles from the callbacks they replace instead of using an
  unconstrained mock union. Preserve behavioral assertions and fixtures.
- Verify clean installs with npm 10 from CI as well as local npm 11. The initial
  npm 11 lock passed locally but omitted two optional Wasm dependency records
  required by npm 10. Regenerating the lock without an installation restores
  those records and removes orphaned esbuild platform entries; existing package
  versions remain unchanged.
- Related-test selection may omit a dependency behind a complete module mock.
  Verify both complete-suite coverage and selection when the tested component
  itself changes; do not force the old over-inclusive dependency graph.

Compare identical production source with one warm-up and five alternating runs
per toolchain for Vite build, the full frontend build and the full test suite.
Record output bytes and development dependency footprint separately. A faster
build/test loop is not evidence of lower application CPU, RSS or subtitle latency.
The measured candidate uses about 20 MB more development dependencies while
reducing build/test time and compressed JavaScript/CSS; accept that explicit
development trade-off rather than claiming every size metric improves.

Acceptance also includes the signed native watch preview and hot update without
restarting the app, the canonical checks, and CI on the supported hosts. The
minimum WebView targets remain compile targets unless tested on those actual
versions. Keep benchmark and native evidence in the integration ledger.

Migration references: [Vite 8](https://vite.dev/guide/migration),
[Vitest 4](https://v4.vitest.dev/guide/migration.html),
[Vitest 5](https://vitest.dev/guide/migration/), and
[mock lifecycle](https://vitest.dev/config/clearmocks). Development connection
and logging options follow [Vite server options](https://vite.dev/config/server-options).
