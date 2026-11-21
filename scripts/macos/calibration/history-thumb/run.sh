#!/bin/bash
set -euo pipefail
[[ $(uname -s) == Darwin && $RUNNER_ENVIRONMENT == github-hosted && $# == 1 ]]
evidence=$1
[[ ! -e "$evidence" ]]
app=/Applications/mimi.app
suite=scripts/macos/tests
qa=scripts/macos/calibration/history-thumb
[[ -d "$app" ]]
bash scripts/macos/verify-macos-release-source.sh "$app" 764710845f25e17e91ce803e3904bd25ec685719 1.7.5 arm64
if pgrep -x mimi >/dev/null; then echo 'The test desktop already has a Mimi session.' >&2; exit 1; fi
mkdir -p "$evidence"
shasum -a 256 "$app/Contents/MacOS/mimi" | tee "$evidence/executable.sha256"
scene=$(mktemp -d "$TMPDIR/mimi-ui-test-thumb.XXXXXX")
pid=''
cleanup() {
  if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then kill -TERM "$pid"; wait "$pid" || true; fi
  rm -rf "$scene"
}
trap cleanup EXIT
mkdir "$scene/ready"
cat > "$scene/preferences.json" <<'JSON'
{"ui_language":"en","source_language":"en","target_language":"zh","audio_input":"system","subtitle_display_mode":"bilingual","font_size":16,"subtitle_animation":false,"subtitle_background_opacity":100,"retain_session_history":false,"record_session_audio":false}
JSON
python3 "$qa/build-driver.py" "$suite" "$scene/observe"
env MIMI_UI_TEST=1 MIMI_AUTO_START=1 MIMI_UI_TEST_STANDARD_OVERLAY=1 MIMI_UI_TEST_CAPTION_HISTORY=1 \
 MIMI_UI_TEST_PREFERENCES_DIR="$scene" MIMI_UI_TEST_FRONTEND_READY_DIR="$scene/ready" \
 MIMI_UI_TEST_SESSION_STATE_FILE="$scene/session" MIMI_UI_TEST_SETTINGS_ACTIVATION_FILE="$scene/settings-activation" \
 "$app/Contents/MacOS/mimi" >"$scene/app.log" 2>&1 &
pid=$!
ready() { [[ -f "$scene/ready/settings" && -f "$scene/ready/overlay" && -f "$scene/session" &&
             $(cat "$scene/session") == listening && -f "$scene/caption-history-count" ]]; }
for ((i=0; i<150; ++i)); do
  kill -0 "$pid" 2>/dev/null || { echo 'The signed test app exited.' >&2; exit 1; }
  if ready; then break; fi
  sleep 0.2
done
if ! ready || [[ $(cat "$scene/caption-history-count") != 18 || -e "$scene/settings-activation" ]]; then echo "Isolated caption fixture did not start" >&2; exit 1; fi
"$scene/observe" "$pid" "$scene" "$evidence" 2>&1 | tee "$evidence/gesture.log"
ready && [[ $(cat "$scene/caption-history-count") == 20 && ! -e "$scene/settings-activation" ]]
