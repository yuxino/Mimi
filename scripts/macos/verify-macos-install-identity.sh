#!/usr/bin/env bash
set -euo pipefail

# Fail-closed preflight for replacing an installed macOS formal app. TCC and
# Keychain continuity depend on the complete designated requirement, not the
# visible app name, version, or bundle identifier alone.

usage() {
  echo "Usage: ./scripts/macos/verify-macos-install-identity.sh NEW_APP INSTALLED_APP" >&2
  exit 2
}

[[ $# -eq 2 ]] || usage
[[ "$(uname -s)" == "Darwin" ]] || {
  echo "macOS install identity checks must run on macOS." >&2
  exit 1
}

NEW_APP="$1"
INSTALLED_APP="$2"

case "$NEW_APP" in
  /*.app) ;;
  *) usage ;;
esac
case "$INSTALLED_APP" in
  /*.app) ;;
  *) usage ;;
esac

[[ -d "$NEW_APP" && ! -L "$NEW_APP" ]] || {
  echo "Expected a real new app bundle: $NEW_APP" >&2
  exit 1
}

bundle_identifier() {
  /usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' \
    "$1/Contents/Info.plist" 2>/dev/null
}

designated_requirement() {
  codesign --display --requirements - "$1" 2>&1 \
    | /usr/bin/sed -n 's/^#*[[:space:]]*designated => //p'
}

"$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/verify-macos-app.sh" "$NEW_APP"
NEW_IDENTIFIER="$(bundle_identifier "$NEW_APP")"
[[ "$NEW_IDENTIFIER" == "app.yuxino.mimi" ]] || {
  echo "The new app is not the formal mimi bundle: $NEW_IDENTIFIER" >&2
  exit 1
}
NEW_REQUIREMENT="$(designated_requirement "$NEW_APP")"

if [[ ! -e "$INSTALLED_APP" ]]; then
  echo "No installed formal app exists; identity migration is not applicable."
  echo "New designated requirement: $NEW_REQUIREMENT"
  exit 0
fi
[[ -d "$INSTALLED_APP" && ! -L "$INSTALLED_APP" ]] || {
  echo "Installed app path is not a real app bundle: $INSTALLED_APP" >&2
  exit 1
}
codesign --verify --deep --strict "$INSTALLED_APP"
INSTALLED_IDENTIFIER="$(bundle_identifier "$INSTALLED_APP")"
[[ "$INSTALLED_IDENTIFIER" == "$NEW_IDENTIFIER" ]] || {
  echo "Bundle identifiers differ; refusing to replace the installed app." >&2
  echo "Installed: $INSTALLED_IDENTIFIER" >&2
  echo "New:       $NEW_IDENTIFIER" >&2
  exit 1
}
INSTALLED_REQUIREMENT="$(designated_requirement "$INSTALLED_APP")"

if [[ "$INSTALLED_REQUIREMENT" == "$NEW_REQUIREMENT" ]]; then
  echo "Install identity is continuous: $NEW_REQUIREMENT"
  exit 0
fi

if [[ "${MIMI_ALLOW_IDENTITY_CHANGE:-0}" == "1" ]]; then
  cat >&2 <<EOF
warning: explicitly allowing a macOS code-identity migration.

Installed: ${INSTALLED_REQUIREMENT:-<invalid or unsigned>}
New:       ${NEW_REQUIREMENT:-<invalid or unsigned>}

Screen & System Audio Recording and Keychain access may require one final
authorization. Do not use this override for routine local testing.
EOF
  exit 0
fi

cat >&2 <<EOF
error: refusing to replace mimi with a different macOS code identity.

Installed: ${INSTALLED_REQUIREMENT:-<invalid or unsigned>}
New:       ${NEW_REQUIREMENT:-<invalid or unsigned>}

The replacement would make macOS request Screen Recording and Keychain
authorization again. Use ./scripts/dev-app.sh for pre-push testing. A deliberate
certificate migration requires one explicit run with MIMI_ALLOW_IDENTITY_CHANGE=1.
EOF
exit 1
