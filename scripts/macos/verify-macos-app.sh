#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "Usage: $0 [--release] /absolute/path/to/mimi.app" >&2
  exit 2
}

RELEASE=0
case "${1:-}" in
  --release)
    RELEASE=1
    shift
    ;;
esac
[[ $# -eq 1 ]] || usage

APP="$1"
[[ "$(uname -s)" == "Darwin" ]] || {
  echo "macOS bundle verification must run on macOS." >&2
  exit 1
}
[[ "$APP" == /* && -d "$APP" ]] || {
  echo "Expected an existing absolute .app path: $APP" >&2
  exit 1
}

PLIST="$APP/Contents/Info.plist"
[[ -f "$PLIST" ]] || {
  echo "Missing Info.plist: $PLIST" >&2
  exit 1
}

read_plist() {
  /usr/libexec/PlistBuddy -c "Print :$1" "$PLIST" 2>/dev/null
}

IDENTIFIER="$(read_plist CFBundleIdentifier)"
SCREEN_USAGE="$(read_plist NSScreenCaptureUsageDescription)"
AUDIO_USAGE="$(read_plist NSAudioCaptureUsageDescription)"
MICROPHONE_USAGE="$(read_plist NSMicrophoneUsageDescription || true)"

[[ "$IDENTIFIER" == "app.yuxino.mimi" ]] || {
  echo "Unexpected bundle identifier: $IDENTIFIER" >&2
  exit 1
}
[[ -n "$SCREEN_USAGE" ]] || {
  echo "NSScreenCaptureUsageDescription must not be empty." >&2
  exit 1
}
[[ -n "$AUDIO_USAGE" ]] || {
  echo "NSAudioCaptureUsageDescription must not be empty." >&2
  exit 1
}
[[ -n "$MICROPHONE_USAGE" ]] || {
  echo "NSMicrophoneUsageDescription must not be empty." >&2
  exit 1
}

codesign --verify --deep --strict "$APP"

ENTITLEMENTS="$(codesign --display --entitlements - --xml "$APP" 2>/dev/null)"
if ! python3 -c 'import plistlib, sys
try:
    valid = plistlib.loads(sys.stdin.buffer.read()).get("com.apple.security.device.audio-input") is True
except (ValueError, TypeError, AttributeError, plistlib.InvalidFileException):
    valid = False
sys.exit(0 if valid else 1)' <<<"$ENTITLEMENTS"; then
  echo "The signed app must enable the audio-input entitlement for microphone capture." >&2
  exit 1
fi

SIGNATURE_DETAILS="$(codesign --display --verbose=4 "$APP" 2>&1)"
REQUIREMENT="$(
  codesign --display --requirements - "$APP" 2>&1 \
    | sed -n 's/^#*[[:space:]]*designated => //p'
)"
grep -Fq "Identifier=app.yuxino.mimi" <<<"$SIGNATURE_DETAILS" || {
  echo "The signing identifier is not app.yuxino.mimi." >&2
  exit 1
}
if { \
  grep -Fq "Signature=adhoc" <<<"$SIGNATURE_DETAILS" \
    || [[ -z "$REQUIREMENT" || "$REQUIREMENT" == cdhash\ * ]]; \
}; then
  echo "Ad-hoc or build-specific signatures are forbidden for mimi app bundles." >&2
  exit 1
fi

if [[ "$RELEASE" == "1" ]]; then
  fingerprint="$(tr -d '[:space:]' < "$(dirname "$0")/macos-release-identity.txt")"
  [[ "$fingerprint" =~ ^[0-9A-F]{40}$ ]] || {
    echo "Invalid pinned macOS release certificate fingerprint." >&2
    exit 1
  }
  expected="identifier \"app.yuxino.mimi\" and certificate root = H\"$fingerprint\""
  codesign --verify --strict -R "=$expected" "$APP"
  # Pin the entire requirement too: a build-specific extra condition would
  # pass the certificate check but still break TCC continuity.
  [[ "$(printf '%s' "$REQUIREMENT" | tr '[:upper:]' '[:lower:]')" == "$(printf '%s' "$expected" | tr '[:upper:]' '[:lower:]')" ]] || {
    echo "The release designated requirement differs from the stable policy." >&2
    exit 1
  }
fi

echo "Verified macOS app: $APP"
