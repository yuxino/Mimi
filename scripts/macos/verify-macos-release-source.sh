#!/usr/bin/env bash
# Run only after bundle signature verification. The source marker is sealed
# in Info.plist by the maintainer certificate, not an unsigned sidecar.
set -euo pipefail
[[ $# -eq 4 ]] || { echo "Usage: $0 APP EXPECTED_REVISION EXPECTED_VERSION EXPECTED_ARCH" >&2; exit 2; }
APP="$1"
[[ "$2" =~ ^[0-9a-f]{40}$ ]] || exit 1
[[ "$4" == arm64 || "$4" == x86_64 ]] || exit 2
"$(dirname "$0")/verify-macos-app.sh" --release "$APP"
[[ "$(lipo -archs "$APP/Contents/MacOS/mimi")" == "$4" ]] || {
  echo "macOS package architecture does not match its release asset." >&2
  exit 1
}
[[ "$(/usr/libexec/PlistBuddy -c 'Print :MimiSourceRevision' "$APP/Contents/Info.plist")" == "$2" ]] || {
  echo "Signed macOS package source does not match the release commit." >&2
  exit 1
}
[[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$APP/Contents/Info.plist")" == "$3" ]] || {
  echo "Signed macOS package version does not match the release tag." >&2
  exit 1
}
