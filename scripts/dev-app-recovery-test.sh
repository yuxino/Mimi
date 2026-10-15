#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/mimi-dev-recovery-test.XXXXXX")"
trap 'rm -rf "$TEST_ROOT"' EXIT
TEST_ROOT="$(cd "$TEST_ROOT" && pwd -P)"

CANONICAL_APP="$TEST_ROOT/mimi-dev.app"
UNRELATED="$TEST_ROOT/.mimi-dev-install.notes"
MISMATCHED="$TEST_ROOT/.mimi-dev-install.mismatch"
OWNED="$TEST_ROOT/.mimi-dev-install.owned"
MARKER=".mimi-dev-install-owner-v1"

mkdir -p "$UNRELATED" "$MISMATCHED" "$OWNED"
printf 'important' > "$UNRELATED/data"
printf '/another/path/mimi-dev.app' > "$MISMATCHED/$MARKER"
printf '%s' "$CANONICAL_APP" > "$OWNED/$MARKER"

MIMI_DEV_APP_PATH="$CANONICAL_APP" MIMI_DEV_RECOVERY_ONLY=1 \
  "$SCRIPT_DIR/dev-app.sh" >/dev/null

[[ -f "$UNRELATED/data" ]]
[[ -d "$MISMATCHED" ]]
[[ ! -e "$OWNED" ]]

# Preparing a candidate must leave even launcher-owned interrupted installs
# untouched. Recovery is an installation operation, not a build operation.
mkdir -p "$OWNED"
printf '%s' "$CANONICAL_APP" > "$OWNED/$MARKER"
rm -f "$TEST_ROOT/.mimi-dev.lock"
MIMI_DEV_APP_PATH="$CANONICAL_APP" MIMI_DEV_RECOVERY_ONLY=1 \
  "$SCRIPT_DIR/dev-app.sh" --build-only >/dev/null
[[ -d "$OWNED" ]]
[[ ! -e "$TEST_ROOT/.mimi-dev.lock" ]]
[[ -f "$UNRELATED/data" ]]
