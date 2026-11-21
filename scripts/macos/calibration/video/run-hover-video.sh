#!/usr/bin/env bash
set -euo pipefail

if [[ $(uname -s) != Darwin || ${RUNNER_ENVIRONMENT:-} != github-hosted ]]; then
  echo "Run this on an isolated GitHub-hosted Mac with the offline UI-test build." >&2
  exit 2
fi
tools=$(cd "$(dirname "$0")" && pwd)
repo=${GITHUB_WORKSPACE:-$PWD}
suite="$repo/scripts/macos/tests"
app=/Applications/mimi.app
identifier=app.yuxino.mimi
signer=$(tr -d '[:space:]' < "$suite/../macos-release-identity.txt")
if [[ ${1:-} == --dev && $# -ge 3 && $2 =~ ^[[:xdigit:]]{40}$ ]]; then
  app=/Applications/mimi-dev.app
  identifier=app.yuxino.mimi.dev
  signer=$2
  shift 2
fi
animation=false
if [[ ${1:-} == --animation ]]; then animation=true; shift; fi
policy=observe
if [[ ${1:-} == --require-first-click ]]; then policy=require-first-click; shift; fi
if [[ $# -ne 2 || ( $1 != layout && $1 != history && $1 != language ) || -z $2 || $2 == -* ]]; then
  echo "Usage: $0 [--dev CERTIFICATE_SHA1] [--animation] [--require-first-click] layout|history|language EVIDENCE_DIR" >&2
  exit 2
fi
mode=$1
evidence=$2
recorder=${MIMI_HOVER_RECORDER:-$tools/recorder}
[[ -d "$app" && ! -e "$evidence" && ( $mode == language || -x "$recorder" ) ]]
if pgrep -x mimi >/dev/null; then
  echo "Another Mimi process already owns the test desktop." >&2
  exit 1
fi
if [[ $identifier == app.yuxino.mimi ]]; then
  bash "$suite/../verify-macos-app.sh" --release "$app"
else
  [[ $(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app/Contents/Info.plist") == "$identifier" ]]
  codesign --verify --deep --strict "$app"
  codesign --verify --strict -R "=identifier \"$identifier\" and certificate leaf = H\"$signer\"" "$app"
fi
mkdir -p "$evidence"
evidence=$(cd "$evidence" && pwd)
{
  echo "verified identifier=$identifier certificatePinSHA1=$signer scene=$mode subtitleAnimation=$animation policy=$policy"
  shasum -a 256 "$app/Contents/MacOS/mimi"
} | tee "$evidence/input.log"
scene=$(mktemp -d "${TMPDIR%/}/mimi-ui-test-hover.XXXXXX")
pid=
cleanup() {
  if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
    kill -TERM "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  fi
  if [[ -f "$scene/language-surface.jsonl" ]]; then
    cp "$scene/language-surface.jsonl" "$evidence/language-surface.jsonl"
  fi
  rm -rf "$scene"
}
trap cleanup EXIT
mkdir "$scene/ready"
cat > "$scene/preferences.json" <<JSON
{"ui_language":"en","source_language":"en","target_language":"zh","audio_input":"system","subtitle_display_mode":"bilingual","font_size":16,"subtitle_animation":$animation,"subtitle_background_opacity":100,"retain_session_history":false,"record_session_audio":false}
JSON
python3 "$tools/build-hover-input.py" "$suite" "$scene/check-hover"
scene_flag=MIMI_UI_TEST_TRANSLATION_LAYOUT=1
if [[ $mode == history ]]; then scene_flag=MIMI_UI_TEST_CAPTION_HISTORY=1; fi
env MIMI_UI_TEST=1 MIMI_UI_TEST_LANGUAGE_PROBE=1 MIMI_AUTO_START=1 MIMI_UI_TEST_STANDARD_OVERLAY=1 "$scene_flag" \
  MIMI_UI_TEST_PREFERENCES_DIR="$scene" MIMI_UI_TEST_FRONTEND_READY_DIR="$scene/ready" \
  MIMI_UI_TEST_SESSION_STATE_FILE="$scene/session" MIMI_UI_TEST_SETTINGS_ACTIVATION_FILE="$scene/settings-activation" \
  "$app/Contents/MacOS/mimi" > "$scene/app.log" 2>&1 &
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
ready || { echo "The two signed UI-test windows and listening state were not ready after 30 seconds." >&2; exit 1; }
[[ ! -e "$scene/settings-activation" ]]
"$scene/check-hover" "$pid" "$identifier" "$scene" "$evidence" "$mode" "$recorder" "$policy" 2>&1 | tee -a "$evidence/input.log"
kill -0 "$pid"
ready && [[ ! -e "$scene/settings-activation" ]]
echo "Finished the $mode interaction with Settings closed and listening active." | tee -a "$evidence/input.log"
