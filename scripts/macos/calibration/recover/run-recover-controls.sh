#!/usr/bin/env bash
set -euo pipefail
if [[ $(uname -s) != Darwin || ${RUNNER_ENVIRONMENT:-} != github-hosted ]]; then
  echo 'Use an isolated hosted Mac with preapproved AX, input and capture permissions.' >&2
  exit 2
fi
if [[ $# != 4 || $1 != --dev || ! $2 =~ ^[[:xdigit:]]{40}$ || ! $3 =~ ^(lock|settings)$ || -e $4 ]]; then
  echo 'Usage: run-recover-controls.sh --dev CERT_SHA1 lock|settings NEW_EVIDENCE_DIR' >&2
  exit 2
fi
suite=$(cd "$(dirname "$0")" && pwd)
signer=$2
scenario=$3
evidence=$4
app=/Applications/mimi-dev.app
[[ -d $app && -f scripts/macos/tests/OverlayHoverControls.swift && -f scripts/macos/tests/Capture.swift ]]
if pgrep -x mimi >/dev/null; then echo 'Another Mimi owns the desktop.' >&2; exit 1; fi
[[ $(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app/Contents/Info.plist") == app.yuxino.mimi.dev ]]
codesign --verify --deep --strict "$app"
codesign --verify --strict -R "=identifier \"app.yuxino.mimi.dev\" and certificate leaf = H\"$signer\"" "$app"
mkdir -p "$evidence"
shasum -a 256 "$app/Contents/MacOS/mimi" > "$evidence/executable.sha256"
scene=$(mktemp -d "${TMPDIR%/}/mimi-ui-test-recover.XXXXXX")
pid=
cleanup() {
  if [[ -n $pid ]] && kill -0 "$pid" 2>/dev/null; then kill -TERM "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
  if [[ -f $scene/app.log ]]; then cp "$scene/app.log" "$evidence/app.log"; fi
  rm -rf "$scene"
}
trap cleanup EXIT
mkdir "$scene/ready"
cat > "$scene/preferences.json" <<'JSON'
{"ui_language":"en","source_language":"en","target_language":"zh","audio_input":"system","subtitle_display_mode":"bilingual","font_size":16,"subtitle_animation":false,"subtitle_background_opacity":100,"overlay_locked":false,"retain_session_history":false,"record_session_audio":false}
JSON
xcrun swiftc -warnings-as-errors "$suite/RecoverControls.swift" scripts/macos/tests/OverlayHoverControls.swift scripts/macos/tests/Capture.swift -o "$scene/check-recover"
MIMI_UI_TEST=1 MIMI_AUTO_START=1 MIMI_UI_TEST_STANDARD_OVERLAY=1 \
  MIMI_UI_TEST_TRANSLATION_LAYOUT=1 MIMI_UI_TEST_PREFERENCES_DIR="$scene" \
  MIMI_UI_TEST_FRONTEND_READY_DIR="$scene/ready" MIMI_UI_TEST_SESSION_STATE_FILE="$scene/session" \
  MIMI_UI_TEST_SETTINGS_ACTIVATION_FILE="$scene/settings-activation" \
  "$app/Contents/MacOS/mimi" > "$scene/app.log" 2>&1 &
pid=$!
ready() { [[ -f $scene/ready/settings && -f $scene/ready/overlay && -f $scene/translation-layout-state.json && -s $scene/session ]] && [[ $(cat "$scene/session") == listening ]]; }
for ((i=0; i<150; i++)); do
  kill -0 "$pid" 2>/dev/null || { echo 'Fixture exited before ready.' >&2; exit 1; }
  if ready; then break; fi
  sleep 0.2
done
ready && [[ ! -e $scene/settings-activation ]] || { echo 'Fixture not ready in 30s.' >&2; exit 1; }
"$scene/check-recover" "$pid" app.yuxino.mimi.dev "$scene" "$evidence" "$scenario" 2>&1 | tee "$evidence/check.log"
ready && kill -0 "$pid"
python3 - "$scene" "$scenario" "$evidence" <<'PY'
import json,pathlib,sys
scene=pathlib.Path(sys.argv[1]); kind=sys.argv[2]; output=pathlib.Path(sys.argv[3])
p=json.loads((scene/'preferences.json').read_text())
assert p['overlay_locked'] is False
assert p['audio_input']=='system' and not p['retain_session_history'] and not p['record_session_audio']
marker=scene/'settings-activation'
assert (not marker.exists()) if kind=='lock' else marker.read_text()=='focus-requested'
(output/'final-state.json').write_text(json.dumps({'overlayLocked':p['overlay_locked'],'session':(scene/'session').read_text(),'settingsActivation':marker.read_text() if marker.exists() else None},indent=2)+'\n')
PY
