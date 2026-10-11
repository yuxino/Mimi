#!/usr/bin/env bash

# Keep every launch explicit: LaunchServices or a parent environment may still
# contain UI-fixture values from a previous development launch.
open_mimi_development_app() {
  local mode="$1"
  local evidence_workspace="$2"
  local canonical_app="$3"
  local ui_test=0
  local auto_start=0
  local compact_settings=0

  if [[ "$mode" == "ui-only" ]]; then
    ui_test=1
    # Only a deliberate opt-in on this invocation may start a synthetic session.
    [[ "${MIMI_AUTO_START:-0}" == "1" ]] && auto_start=1
    [[ "${MIMI_UI_TEST_COMPACT_SETTINGS:-0}" == "1" ]] && compact_settings=1
  elif [[ "$mode" != "live" ]]; then
    echo "error: invalid development launch mode." >&2
    return 2
  fi

  open -n \
    --env "MIMI_UI_TEST=$ui_test" \
    --env "MIMI_AUTO_START=$auto_start" \
    --env "MIMI_UI_TEST_COMPACT_SETTINGS=$compact_settings" \
    --env "MIMI_DEVELOPMENT_EVIDENCE_WORKSPACE=$evidence_workspace" \
    "$canonical_app"
}
