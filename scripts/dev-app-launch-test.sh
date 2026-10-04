#!/usr/bin/env bash
# Regression coverage for per-launch mode selection; no application is opened.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
source "$SCRIPT_DIR/dev-app-launch.sh"

TEST_APP="/synthetic app directory/mimi-dev.app"
TEST_UI_TEST=1
TEST_AUTO_START=1
TEST_WORKSPACE=previous-batch
TEST_OPEN_COUNT=0

# Model an opener that retains environment entries omitted by later launches.
# Explicit values must override both this state and inherited shell variables.
open() {
  [[ "$1" == "-n" ]]
  shift
  while [[ "${1:-}" == "--env" ]]; do
    case "$2" in
      MIMI_UI_TEST=*) TEST_UI_TEST="${2#*=}" ;;
      MIMI_AUTO_START=*) TEST_AUTO_START="${2#*=}" ;;
      MIMI_DEVELOPMENT_EVIDENCE_WORKSPACE=*) TEST_WORKSPACE="${2#*=}" ;;
      *) echo "Unexpected launch environment." >&2; return 1 ;;
    esac
    shift 2
  done
  [[ "$#" == "1" && "$1" == "$TEST_APP" ]]
  TEST_OPEN_COUNT=$((TEST_OPEN_COUNT + 1))
}

MIMI_UI_TEST=1 MIMI_AUTO_START=1 open_mimi_development_app live "" "$TEST_APP"
[[ "$TEST_UI_TEST" == "0" && "$TEST_AUTO_START" == "0" && -z "$TEST_WORKSPACE" ]]

unset MIMI_AUTO_START
MIMI_UI_TEST=0 open_mimi_development_app ui-only named-batch "$TEST_APP"
[[ "$TEST_UI_TEST" == "1" && "$TEST_AUTO_START" == "0" && "$TEST_WORKSPACE" == "named-batch" ]]

MIMI_AUTO_START=1 open_mimi_development_app ui-only synthetic-batch "$TEST_APP"
[[ "$TEST_UI_TEST" == "1" && "$TEST_AUTO_START" == "1" ]]

open_mimi_development_app ui-only "" "$TEST_APP"
[[ "$TEST_UI_TEST" == "1" && "$TEST_AUTO_START" == "0" && -z "$TEST_WORKSPACE" ]]

open_mimi_development_app live "" "$TEST_APP"
[[ "$TEST_UI_TEST" == "0" && "$TEST_AUTO_START" == "0" && -z "$TEST_WORKSPACE" ]]

if open_mimi_development_app invalid "" "$TEST_APP" >/dev/null 2>&1; then
  echo "Invalid launch mode was accepted." >&2
  exit 1
fi
[[ "$TEST_OPEN_COUNT" == "5" ]]
echo "Development launch mode tests passed."
