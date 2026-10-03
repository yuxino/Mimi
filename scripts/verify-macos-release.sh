#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 || "$1" != /* || "$2" != /* ]]; then
  echo "Usage: $0 /absolute/path/to/mimi.app /absolute/path/to/mimi.dmg" >&2
  exit 2
fi

APP="$1"
DMG="$2"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

[[ -f "$DMG" ]] || {
  echo "Missing DMG: $DMG" >&2
  exit 1
}

"$SCRIPT_DIR/verify-macos-app.sh" --release "$APP"
python3 "$SCRIPT_DIR/check-production-native-debugger.py" "$APP/Contents/MacOS/mimi"

MOUNT_DIR="$(mktemp -d "${TMPDIR:-/tmp}/mimi-release-dmg.XXXXXX")"
cleanup_mount() {
  hdiutil detach "$MOUNT_DIR" -quiet >/dev/null 2>&1 || true
  rmdir "$MOUNT_DIR" >/dev/null 2>&1 || true
}
trap cleanup_mount EXIT

hdiutil attach "$DMG" -nobrowse -readonly -mountpoint "$MOUNT_DIR" >/dev/null
EMBEDDED_APPS=()
while IFS= read -r -d '' candidate; do
  EMBEDDED_APPS+=("$candidate")
done < <(find "$MOUNT_DIR" -maxdepth 2 -name '*.app' -type d -print0)
[[ "${#EMBEDDED_APPS[@]}" == "1" ]] || {
  echo "The DMG must contain exactly one app bundle." >&2
  exit 1
}
EMBEDDED_APP="${EMBEDDED_APPS[0]}"
[[ "$(basename "$EMBEDDED_APP")" == "mimi.app" && -d "$EMBEDDED_APP" && ! -L "$EMBEDDED_APP" ]] || {
  echo "The DMG contains an unexpected app bundle." >&2
  exit 1
}
"$SCRIPT_DIR/verify-macos-app.sh" --release "$EMBEDDED_APP"

designated_requirement() {
  codesign --display --requirements - "$1" 2>&1 \
    | sed -n 's/^#*[[:space:]]*designated => //p'
}

LOOSE_REQUIREMENT="$(designated_requirement "$APP")"
EMBEDDED_REQUIREMENT="$(designated_requirement "$EMBEDDED_APP")"
[[ -n "$LOOSE_REQUIREMENT" && "$LOOSE_REQUIREMENT" == "$EMBEDDED_REQUIREMENT" ]] || {
  echo "The DMG contains an app with a different designated requirement." >&2
  exit 1
}

code_directory_hash() {
  codesign --display --verbose=4 "$1" 2>&1 \
    | sed -n 's/^CDHash=//p' \
    | head -1
}

LOOSE_CDHASH="$(code_directory_hash "$APP")"
EMBEDDED_CDHASH="$(code_directory_hash "$EMBEDDED_APP")"
[[ -n "$LOOSE_CDHASH" && "$LOOSE_CDHASH" == "$EMBEDDED_CDHASH" ]] || {
  echo "The DMG does not contain the app from this build." >&2
  exit 1
}

LOOSE_VERSION="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$APP/Contents/Info.plist")"
EMBEDDED_VERSION="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$EMBEDDED_APP/Contents/Info.plist")"
[[ -n "$LOOSE_VERSION" && "$LOOSE_VERSION" == "$EMBEDDED_VERSION" ]] || {
  echo "The DMG contains an app with a different version." >&2
  exit 1
}

cleanup_mount
trap - EXIT

echo "Verified stable-signed macOS release: $DMG"
