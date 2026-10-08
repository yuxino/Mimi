#!/usr/bin/env bash
# Produce release assets on the maintainer Mac; never export the signing key.
set -euo pipefail
[[ $# -eq 0 && "$(uname -s)" == Darwin ]] || {
  echo "Usage: ./scripts/prepare-macos-release.sh (on the signing Mac)" >&2
  exit 2
}
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
cd "$SCRIPT_DIR/.."
[[ -z "$(git status --porcelain)" ]] || {
  echo "Commit the reviewed release source before preparing signed assets." >&2
  exit 1
}
export APPLE_SIGNING_IDENTITY="$(tr -d '[:space:]' < scripts/macos-release-identity.txt)"
[[ "$APPLE_SIGNING_IDENTITY" =~ ^[0-9A-F]{40}$ ]] || exit 1
export MACOSX_DEPLOYMENT_TARGET=13.0
export CARGO_HOME="${CARGO_HOME:-$PWD/.cargo-home}"
export npm_config_cache="${npm_config_cache:-$PWD/.npm-cache}"
TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/src-tauri/target}"
if [[ "$TARGET_DIR" != /* ]]; then
  TARGET_DIR="$PWD/$TARGET_DIR"
fi
export CARGO_TARGET_DIR="$TARGET_DIR"
[[ "$(uname -m)" == arm64 ]] || {
  echo "Prepare both macOS release architectures on the Apple silicon signing Mac." >&2
  exit 1
}
TEMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/mimi-release-config.XXXXXX")"
trap 'rm -rf "$TEMP_DIR"' EXIT
REVISION="$(git rev-parse HEAD)"
python3 - "$TEMP_DIR" "$REVISION" <<'PY'
import json, pathlib, plistlib, sys
folder = pathlib.Path(sys.argv[1])
with open('src-tauri/Info.plist', 'rb') as f:
    info = plistlib.load(f)
info['MimiSourceRevision'] = sys.argv[2]
with (folder / 'Info.plist').open('wb') as f:
    plistlib.dump(info, f)
(folder / 'tauri.conf.json').write_text(json.dumps({
    'build': {'beforeBuildCommand': ''},
    'bundle': {'createUpdaterArtifacts': False, 'macOS': {'infoPlist': str(folder / 'Info.plist')}}
}))
PY
VERSION="$(node -p 'require("./package.json").version')"
# Both architectures embed the same Safari-targeted frontend from this exact
# checkout. Build and validate it once before either native build starts.
TAURI_ENV_PLATFORM=darwin npm run build
for arch in arm64 x86_64; do
  if [[ "$arch" == arm64 ]]; then
    # Preserve the established native ARM cache and archive name for updates.
    TARGET_ARGS=()
    BUNDLE="$TARGET_DIR/release/bundle"
    DMG_ARCH=aarch64
    ARCHIVE=mimi.app.tar.gz
  else
    TARGET_ARGS=(--target x86_64-apple-darwin)
    BUNDLE="$TARGET_DIR/x86_64-apple-darwin/release/bundle"
    DMG_ARCH=x64
    ARCHIVE=mimi_x64.app.tar.gz
  fi
  npm run tauri -- build ${TARGET_ARGS[@]+"${TARGET_ARGS[@]}"} --config "$TEMP_DIR/tauri.conf.json" -- --locked
  APP="$BUNDLE/macos/mimi.app"
  DMG="$BUNDLE/dmg/mimi_${VERSION}_${DMG_ARCH}.dmg"
  ./scripts/verify-macos-release.sh "$APP" "$DMG"
  ./scripts/verify-macos-release-source.sh "$APP" "$REVISION" "$VERSION" "$arch"
  # CI applies the existing updater signature after verifying both architectures.
  COPYFILE_DISABLE=1 tar -czf "$BUNDLE/macos/$ARCHIVE" -C "$BUNDLE/macos" mimi.app
  python3 scripts/extract-macos-updater.py "$BUNDLE/macos/$ARCHIVE" "$TEMP_DIR/extracted-$arch"
  ./scripts/verify-macos-release-source.sh "$TEMP_DIR/extracted-$arch/mimi.app" "$REVISION" "$VERSION" "$arch"
  ./scripts/verify-macos-release.sh "$TEMP_DIR/extracted-$arch/mimi.app" "$DMG"
  echo "Verified $arch release assets for v$VERSION at $REVISION:"
  echo "  $DMG"
  echo "  $BUNDLE/macos/$ARCHIVE"
done
echo "Follow docs/development/macos-release-signing.md to stage all four assets."
echo "This script does not upload, publish, or install."
