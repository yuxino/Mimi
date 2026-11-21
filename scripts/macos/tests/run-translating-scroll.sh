#!/usr/bin/env bash
set -euo pipefail

# Own an isolated desktop, never an already-running Mimi or its preferences.
if [[ $(uname -s) != Darwin || ${RUNNER_ENVIRONMENT:-} != github-hosted ]]; then
  echo "Run the translating-scroll check on a dedicated hosted Mac." >&2
  exit 2
fi
suite=$(cd "$(dirname "$0")" && pwd)
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
display_modes=false
if [[ ${1:-} == --display-modes ]]; then display_modes=true; shift; fi
if [[ $# -ne 1 || -z $1 || $1 == -* ]]; then
  echo "Usage: $0 [--dev CERTIFICATE_SHA1] [--animation] [--display-modes] EVIDENCE_DIR" >&2
  exit 2
fi
evidence=$1
[[ -d "$app" && ! -e "$evidence" ]]
if pgrep -x mimi >/dev/null; then
  echo "Another Mimi process is already using the test desktop." >&2
  exit 1
fi
if [[ $identifier == app.yuxino.mimi ]]; then
  "$suite/../verify-macos-app.sh" --release "$app"
else
  [[ $(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app/Contents/Info.plist") == "$identifier" ]]
  codesign --verify --deep --strict "$app"
  codesign --verify --strict -R "=identifier \"$identifier\" and certificate leaf = H\"$signer\"" "$app"
fi
mkdir -p "$evidence"
{
  echo "Verified identifier=$identifier certificatePinSHA1=$signer subtitleAnimation=$animation"
  shasum -a 256 "$app/Contents/MacOS/mimi"
} | tee "$evidence/translating-scroll.log"
scene=$(mktemp -d "${TMPDIR%/}/mimi-ui-test-translating.XXXXXX")
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
cat > "$scene/preferences.json" <<JSON
{"ui_language":"en","source_language":"en","target_language":"zh","audio_input":"system","subtitle_display_mode":"bilingual","font_size":16,"subtitle_animation":$animation,"subtitle_background_opacity":100,"retain_session_history":false,"record_session_audio":false}
JSON
xcrun swiftc "$suite/translating-scroll.swift" "$suite/DisplayModes.swift" "$suite/Capture.swift" -o "$scene/check-translating-scroll"
MIMI_UI_TEST=1 MIMI_AUTO_START=1 MIMI_UI_TEST_STANDARD_OVERLAY=1 \
  MIMI_UI_TEST_TRANSLATION_LAYOUT=1 MIMI_UI_TEST_PREFERENCES_DIR="$scene" \
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
ready || { echo "Signed UI-test windows and listening state were not ready in 30 seconds." >&2; exit 1; }
[[ ! -e "$scene/settings-activation" ]]
check_args=("$pid" "$identifier" "$scene" "$evidence")
if [[ $display_modes == true ]]; then check_args+=(--display-modes); fi
"$scene/check-translating-scroll" "${check_args[@]}" 2>&1 | tee -a "$evidence/translating-scroll.log"
kill -0 "$pid"
ready && [[ ! -e "$scene/settings-activation" ]]
echo "Verified synthetic translating-scroll scene stayed listening with Settings closed." | tee -a "$evidence/translating-scroll.log"
