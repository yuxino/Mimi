#!/usr/bin/env bash
set -euo pipefail

# Canonical automated check for the mimi Tauri application:
# Rust formatting + clippy (warnings as errors) + the full unit-test suite,
# then the frontend lint/test pipeline and typechecked production build, then a
# whitespace/error check on the diff.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"

# Local tool caches (the home caches on CI machines can be root-owned).
export CARGO_HOME="${CARGO_HOME:-$PROJECT_DIR/.cargo-home}"
export npm_config_cache="${npm_config_cache:-$PROJECT_DIR/.npm-cache}"

bash "$SCRIPT_DIR/dev-app-launch-test.sh"
bash "$SCRIPT_DIR/check-apple-translation.sh"

if [[ "$(uname -s)" == "Darwin" ]]; then
  echo "==> development install recovery safety"
  "$SCRIPT_DIR/dev-app-recovery-test.sh"
  "$SCRIPT_DIR/macos-signing-test.sh"
  python3 "$SCRIPT_DIR/extract-macos-updater-test.py"
fi

python3 "$SCRIPT_DIR/check-appimage-permissions-test.py"
python3 "$SCRIPT_DIR/check-appimage-gles-test.py"
python3 -B "$SCRIPT_DIR/check-linux-input-region-trace-test.py"
python3 -B "$SCRIPT_DIR/analyze-development-case-test.py"
python3 -B "$SCRIPT_DIR/run-development-batch-test.py"
python3 -B "$SCRIPT_DIR/ci-scope-test.py"
python3 -B "$SCRIPT_DIR/prepare-macos-release-test.py"
bash "$SCRIPT_DIR/check-shared-core.sh"

echo "==> cargo fmt --check"
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check

echo "==> cargo clippy -D warnings"
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings

echo "==> cargo test"
cargo test --locked --manifest-path src-tauri/Cargo.toml

if [[ "$(uname -s)" == "Darwin" ]]; then
  echo "==> compile development service probes (no service requests)"
  cargo test --locked --manifest-path src-tauri/Cargo.toml --features development-debugger --lib --no-run
fi

# Windows compile-level verification when explicitly requested (the MSVC
# target's C dependencies cannot cross-compile from macOS without an external
# Windows SDK; CI runs the full Rust suite on native x64 and ARM64 Windows).
if [[ "${MIMI_CHECK_WINDOWS:-0}" == "1" ]]; then
  echo "==> cargo check (windows-msvc)"
  cargo check --locked --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc
else
  echo "==> skipping windows-msvc check (set MIMI_CHECK_WINDOWS=1 to enable)"
fi

echo "==> frontend lint"
npm run lint

echo "==> frontend tests"
npm run test

echo "==> updater release manifest tests"
node --test scripts/updater-manifest.node.mjs

echo "==> frontend typecheck and build"
npm run build

echo "==> git diff --check"
git diff --check
git diff --cached --check

echo "All checks passed."
