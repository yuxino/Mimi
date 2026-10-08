#!/usr/bin/env bash
set -euo pipefail

# Local QA packaging for the mimi Tauri application. Produces the native
# bundles for the current platform (macOS: .app + .dmg; Windows: MSI + NSIS EXE)
# under src-tauri/target/local-package/bundle/. Never commit dist/ or signing
# identities.
#
# On macOS the signing identity is selected before Tauri creates either the
# .app or .dmg. This is essential: re-signing only the loose .app afterwards
# leaves the copy already embedded in the DMG with its original identity.
# Local builds require the stable "mimi Local Development" identity for
# permission-sensitive QA. Public releases are prepared on the signing Mac with
# prepare-macos-release.sh using the pinned stable certificate. Older ad-hoc
# releases need an intentional one-time identity migration.

TARGET=""
TARGET_ARGS=()
if [[ $# -eq 2 && "$1" == --target && "$(uname -s)" == Darwin ]]; then
  case "$2" in
    x86_64-apple-darwin|aarch64-apple-darwin)
      TARGET="$2"
      TARGET_ARGS=(--target "$TARGET")
      ;;
    *) echo "Unsupported macOS QA target: $2" >&2; exit 2 ;;
  esac
elif [[ $# -ne 0 ]]; then
  echo "Usage: ./scripts/package-app.sh [--target x86_64-apple-darwin|aarch64-apple-darwin]" >&2
  echo "Use prepare-macos-release.sh for public signed updater artifacts." >&2
  exit 2
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"

export CARGO_HOME="${CARGO_HOME:-$PROJECT_DIR/.cargo-home}"
export npm_config_cache="${npm_config_cache:-$PROJECT_DIR/.npm-cache}"
if [[ "$(uname -s)" == "Darwin" ]]; then
  export MACOSX_DEPLOYMENT_TARGET="13.0"
  APPLE_SIGNING_IDENTITY="$("$SCRIPT_DIR/codesign-identity.sh")"
  export APPLE_SIGNING_IDENTITY
  if [[ "$APPLE_SIGNING_IDENTITY" == "-" ]]; then
    cat >&2 <<'EOF'
error: release packaging requires a stable macOS code-signing identity.

Install the local "mimi Local Development" identity or set
MIMI_CODESIGN_IDENTITY to an explicit code-signing identity (prefer the exact
certificate fingerprint). Ad-hoc packages are rejected because they reset
Screen Recording and Keychain authorization.
EOF
    exit 1
  fi
  echo "using macOS signing identity: $APPLE_SIGNING_IDENTITY"
fi

# Local QA packages are signed with the development identity, not the public
# updater signature. Do not require or use the release updater private key for
# QA; prepare-macos-release.sh creates updater artifacts on the signing Mac.
npm run tauri -- build ${TARGET_ARGS[@]+"${TARGET_ARGS[@]}"} --config '{"bundle":{"createUpdaterArtifacts":false}}' -- --locked --profile local-package

TARGET_DIR="${CARGO_TARGET_DIR:-$PROJECT_DIR/src-tauri/target}"
[[ "$TARGET_DIR" == /* ]] || TARGET_DIR="$PROJECT_DIR/$TARGET_DIR"
BUNDLE_DIR="$TARGET_DIR/${TARGET:+$TARGET/}local-package/bundle"
echo "Bundle produced under: $BUNDLE_DIR"
find "$BUNDLE_DIR" -maxdepth 2 \( -name "*.app" -o -name "*.dmg" -o -name "*.msi" -o -name "*.exe" \) -print 2>/dev/null || true

if [[ "$(uname -s)" == "Darwin" ]]; then
  APP="$(find "$BUNDLE_DIR" -maxdepth 3 -name "*.app" -type d | head -1 || true)"
  [[ -n "$APP" ]] || {
    echo "macOS app bundle was not produced." >&2
    exit 1
  }
  "$SCRIPT_DIR/verify-macos-app.sh" "$APP"
  if [[ -n "$TARGET" ]]; then
    ACTUAL_ARCH="$(lipo -archs "$APP/Contents/MacOS/mimi")"
    EXPECTED_ARCH="${TARGET%%-*}"
    [[ "$EXPECTED_ARCH" != aarch64 ]] || EXPECTED_ARCH=arm64
    [[ "$ACTUAL_ARCH" == "$EXPECTED_ARCH" ]] || {
      echo "Wrong package architecture: expected $EXPECTED_ARCH, got $ACTUAL_ARCH" >&2
      exit 1
    }
    echo "Verified package architecture: $ACTUAL_ARCH"
  fi
  if [[ -e /Applications/mimi.app ]]; then
    cat <<EOF
An existing /Applications/mimi.app was left untouched. This package uses
a stable identity. Older ad-hoc GitHub releases have a different identity. Before any deliberate replacement, run:

  ./scripts/verify-macos-install-identity.sh "$APP" /Applications/mimi.app
EOF
  fi
  cat <<EOF
Local package identity checked. For normal pre-push testing use
./scripts/dev-app.sh. Older ad-hoc installations require an explicit
one-time identity migration; package creation never replaces the installed app.
EOF
fi
