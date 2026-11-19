#!/usr/bin/env bash
set -euo pipefail

# This check drives a real pointer. It owns both the desktop and the signed
# release installation on an isolated Mac runner.
if [[ $(uname -s) != Darwin || ${RUNNER_ENVIRONMENT:-} != github-hosted || $# -ne 1 ]]; then
  echo "Run the macOS caption drag check on a dedicated hosted Mac with an evidence directory." >&2
  exit 2
fi

suite=$(cd "$(dirname "$0")" && pwd)
evidence=$1
app=/Applications/mimi.app
[[ -d "$app" && ! -e "$evidence" ]]
if pgrep -x mimi >/dev/null; then
  echo "Another Mimi process is already using the test desktop." >&2
  exit 1
fi
"$suite/../verify-macos-app.sh" --release "$app"
mkdir -p "$evidence"
scene=$(mktemp -d "${TMPDIR%/}/mimi-ui-test-drag.XXXXXX")
pid=
cleanup() {
  if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
    kill -TERM "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  fi
  rm -rf "$scene"
}
trap cleanup EXIT
mkdir "$scene/ready"
cat > "$scene/preferences.json" <<'JSON'
{"ui_language":"en","source_language":"en","target_language":"zh","audio_input":"system","subtitle_display_mode":"bilingual","subtitle_animation":false,"subtitle_background_opacity":100,"retain_session_history":false,"record_session_audio":false}
JSON

xcrun swiftc "$suite/caption-drag.swift" "$suite/Capture.swift" -o "$scene/check-drag"
MIMI_UI_TEST=1 MIMI_AUTO_START=1 MIMI_UI_TEST_STANDARD_OVERLAY=1 \
  MIMI_UI_TEST_DUAL_SUBTITLES=1 MIMI_UI_TEST_PREFERENCES_DIR="$scene" \
  MIMI_UI_TEST_FRONTEND_READY_DIR="$scene/ready" MIMI_UI_TEST_SESSION_STATE_FILE="$scene/session" \
  MIMI_UI_TEST_SETTINGS_ACTIVATION_FILE="$scene/settings-activation" \
  "$app/Contents/MacOS/mimi" >"$scene/app.log" 2>&1 &
pid=$!
ready() {
  [[ -f "$scene/ready/settings" && -f "$scene/ready/overlay" && -s "$scene/session" ]] &&
    [[ $(cat "$scene/session") == listening ]]
}
for ((i=0; i<150; i++)); do
  kill -0 "$pid" 2>/dev/null || { echo "Signed Mimi exited during startup." >&2; exit 1; }
  if ready; then break; fi
  sleep 0.2
done
ready || { echo "The two signed windows and synthetic listening session were not ready in 30 seconds." >&2; exit 1; }
[[ ! -e "$scene/settings-activation" ]]
"$scene/check-drag" "$pid" "$evidence"
kill -0 "$pid"
ready && [[ ! -e "$scene/settings-activation" ]] || {
  echo "The drag changed the listening session or explicitly reopened Settings." >&2
  exit 1
}
echo "Signed Mimi was listening before and after the caption drag. Settings did not reopen."
