#!/usr/bin/env bash
set -euo pipefail

# Local feedback by area. The no-argument canonical check remains the complete
# verification for broad changes; quick checks do not package or launch an app.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_DIR"

usage() {
  echo "Usage: ./scripts/check.sh --quick frontend [src/files...] | --quick desktop [test-filter] | --quick shared" >&2
  exit 2
}

scope="${1:-}"
[[ $# -gt 0 ]] || usage
shift

# Validate the complete request before starting any checks. A misspelled area,
# source path or extra filter must not silently skip verification.
typescript_files=()
case "$scope" in
  frontend)
    for file in "$@"; do
      file="${file#"$PROJECT_DIR/"}"
      file="${file#./}"
      case "$file" in
        src/*) ;;
        *) echo "Frontend quick-check paths must be existing files under src/: $file" >&2; exit 2 ;;
      esac
      if [[ ! -f "$file" || "$file" == *'/../'* ]]; then
        echo "Invalid frontend source file: $file" >&2
        exit 2
      fi
      case "$file" in
        *.ts | *.tsx) typescript_files+=("$file") ;;
        *.css) ;;
        *) echo "Frontend quick-check paths must be TypeScript or CSS files: $file" >&2; exit 2 ;;
      esac
    done
    ;;
  desktop)
    [[ $# -le 1 ]] || usage
    [[ "${1:-}" != -* ]] || usage
    ;;
  shared)
    [[ $# -eq 0 ]] || usage
    ;;
  *) usage ;;
esac

case "$scope" in
  frontend)
    echo "==> shared provider copy"
    python3 -B "$SCRIPT_DIR/../providers/sync-provider-copy.py" --check
    echo "==> frontend typecheck"
    npm run typecheck
    if [[ ${#typescript_files[@]} -gt 0 ]]; then
      echo "==> frontend lint for the selected TypeScript files"
      npx --no-install eslint --config config/eslint.config.js --cache --cache-strategy content --cache-location node_modules/.cache/eslint "${typescript_files[@]}"
      echo "==> frontend tests related to the selected source files"
      npx --no-install vitest --config config/vite.config.ts related --run --maxWorkers=2 "${typescript_files[@]}"
    elif [[ $# -eq 0 ]]; then
      echo "==> frontend lint"
      npm run lint
      echo "==> frontend tests"
      npm run test
    else
      echo "==> CSS-only scope: typecheck and diff; behavioral tests are not selected"
    fi
    ;;
  desktop)
    echo "==> cargo fmt --check"
    cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
    # cargo test compiles the app and tests already; a separate cargo check
    # would repeat compilation with a different dependency feature graph.
    if [[ $# -gt 0 ]]; then
      echo "==> desktop tests matching: $1"
      test_list="$(cargo test --quiet --locked --manifest-path src-tauri/Cargo.toml "$1" -- --list)"
      if [[ "$test_list" != *': test'* ]]; then
        echo "No desktop tests matched: $1" >&2
        exit 1
      fi
      cargo test --locked --manifest-path src-tauri/Cargo.toml "$1"
    else
      echo "==> desktop tests"
      cargo test --locked --manifest-path src-tauri/Cargo.toml
    fi
    ;;
  shared)
    echo "==> shared architecture, Rust and JNI adapter checks"
    bash "$SCRIPT_DIR/../shared/check-shared-core.sh"
    echo "Portable behavior changes also require the Android JVM JNI suite (see AGENTS.md)."
    ;;
esac

echo "==> git diff --check"
git diff --check
git diff --cached --check
if [[ "$scope" == "frontend" && $# -gt 0 && ${#typescript_files[@]} -eq 0 ]]; then
  echo "Quick frontend CSS scope completed: typecheck and diff only. Full verification: ./scripts/check.sh"
else
  echo "Quick $scope checks passed. Full verification: ./scripts/check.sh"
fi
