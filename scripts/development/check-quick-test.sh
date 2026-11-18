#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/mimi-quick-check.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT

# An invalid request must fail before compiling or running a different suite.
reject_before_checks() {
  local status=0
  bash "$SCRIPT_DIR/../check.sh" "$@" >"$TEST_DIR/output" 2>&1 || status=$?
  if [[ "$status" -ne 2 || "$(cat "$TEST_DIR/output")" == *'==>'* ]]; then
    cat "$TEST_DIR/output" >&2
    echo "Quick-check request did not fail before checks: $*" >&2
    exit 1
  fi
}

reject_before_checks --unknown
reject_before_checks --quick
reject_before_checks --quick unknown
reject_before_checks --quick frontend src/does-not-exist.ts
reject_before_checks --quick frontend package.json
reject_before_checks --quick frontend src/../package.json
reject_before_checks --quick desktop --ignored
reject_before_checks --quick desktop first second
reject_before_checks --quick shared unexpected
echo "Quick-check argument safety passed."
