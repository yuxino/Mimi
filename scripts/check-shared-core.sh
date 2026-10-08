#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR/.."

# Independent crates share only compilation output. Relative default paths
# also work with Windows Cargo invoked from Git Bash.
export CARGO_TARGET_DIR="${MIMI_SHARED_CARGO_TARGET_DIR:-shared/target}"
python_command="${MIMI_PYTHON:-python3}"
if [[ -z "${MIMI_PYTHON:-}" && "${OS:-}" == "Windows_NT" ]]; then
  python_command=python
fi
"$python_command" -B scripts/shared-core-native-test.py
for manifest in shared/mimi-core/Cargo.toml shared/mimi-android-jni/Cargo.toml scripts/updater-signature-verifier/Cargo.toml; do
  cargo fmt --manifest-path "$manifest" -- --check
  cargo clippy --locked --manifest-path "$manifest" --all-targets -- -D warnings
  cargo test --locked --manifest-path "$manifest"
done
