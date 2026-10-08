# Build and validation throughput

Keep the same lint, test, native smoke, signing and publication gates while
reducing compilation work and dependency installation.

- Desktop and shared Rust development/test builds emit line tables for useful
  filename/line backtraces, rather than full type/variable debugging data. The
  canonical signed `local-dev` profile retains its explicit debug level.
  ESLint skips Rust target directories and the Git-ignored scratch directory,
  avoiding traversal or accidental parsing of generated compiler files.
- Local QA installers use a separate `local-package` profile inheriting release
  settings, with ThinLTO and 16 codegen units. Public artifacts keep the existing
  size-focused release settings, signing identity and updater requirements.
  QA outputs live under `local-package/bundle`, including an explicit target
  triple when requested; `CARGO_TARGET_DIR` remains supported.
- Windows ARM64 Rust-only validation does not set up Node or install JavaScript
  packages. Acceptance-package and release runs still install the frontend.
  npm installs prefer the existing download cache and omit audit/funding output.
- Acceptance-package and public-package jobs share compatible Rust dependency
  caches; their caches remain separate from ordinary Rust check/test jobs.
- macOS release preparation builds and validates the Safari-targeted production
  frontend once, before either architecture. Both native builds embed that same
  output. A frontend failure prevents both native builds, and a native failure
  stops preparation. Never reuse an arbitrary earlier `dist` or build frontend
  output concurrently in the same checkout.
- Updater signature verification is a small standalone Rust executable with
  the exact existing base64 and Minisign dependencies and streaming algorithm.
  Running it no longer compiles the desktop application, Swift bridges or audio
  dependencies. macOS verifies its downloaded signed assets directly; its
  unused extra frontend build is removed. Rust checks cover the verifier, and
  CLI regressions cover valid Tauri signatures, tampering, wrong keys and errors.

Use matched cold compilation and artifact sizes when comparing profiles. A
GitHub runner timing includes cache download, queueing and runner variance;
report it separately from local compilation. Release preparation ordering tests
use fake native build commands and do not establish signed package acceptance.

Initial local evidence (Apple silicon, Rust 1.95.0, dependencies already
downloaded, separate empty target directories, one sample per configuration):

| `cargo test --no-run` | Before | After |
| --- | ---: | ---: |
| Wall time | 110.77 s | 80.16 s |
| Debug output directory | 3.94 GiB | 3.08 GiB |

The standalone verifier's cold release compilation took 1.01 s on this Mac.
Native cold compilation took approximately 171 s with the public release
profile versus 126 s with the QA profile (Cargo-reported build times, same
production frontend, independent empty profile outputs, one sample each).
The QA executable was 11.91 MiB versus 8.75 MiB for the public release profile;
this size trade-off applies only to local QA. Signed APP/DMG verification and
default/relative target-directory packaging passed without replacing installed
or running apps. These measurements do not predict hosted runner totals.

The profile settings follow [Cargo's debug information options](https://doc.rust-lang.org/cargo/reference/profiles.html#debug).
Package cache sharing uses [rust-cache's shared key](https://github.com/Swatinem/rust-cache#example-usage),
which retains the action's toolchain and environment compatibility keys.
