#!/usr/bin/env bash
set -euo pipefail
[[ "$(uname -s)" == Linux ]] || {
  echo 'Linux packaging requires Linux.' >&2
  exit 2
}
[[ "$(uname -m)" == x86_64 ]] || {
  echo 'Public Linux packages target x86_64.' >&2
  exit 2
}
cd "$(dirname "$0")/../.."

# Tauri downloads the AppRun launcher with mode 770. linuxdeploy renames it to
# AppRun.wrapped, retaining that mode, so unrelated users cannot execute it.
# Seed the same upstream launcher with distribution-safe permissions before
# Tauri copies it into the AppDir. Keep this before updater signing.
export XDG_CACHE_HOME="${XDG_CACHE_HOME:-$HOME/.cache}"
launcher="$XDG_CACHE_HOME/tauri/AppRun-x86_64"
APPRUN_URL="https://github.com/tauri-apps/binary-releases/releases/download/apprun-old/AppRun-x86_64"
EXPECTED_SHA256="f30140a43a0a59e46db21bdefdf749b9e9f2c6946e92afabbacf98b8ae73fb4f"

verify_apprun_sha256() {
  local target="$1"
  local actual
  actual="$(sha256sum "$target" | awk '{print $1}')" || return 1
  [[ "$actual" == "$EXPECTED_SHA256" ]]
}

mkdir -p "$(dirname "$launcher")"
if [[ -s "$launcher" ]] && ! verify_apprun_sha256 "$launcher"; then
  echo "Cached AppRun-x86_64 checksum mismatch; removing cached copy and re-downloading." >&2
  rm -f "$launcher"
fi

if [[ ! -s "$launcher" ]]; then
  temporary="$(mktemp "${launcher}.XXXXXX")"
  trap 'rm -f "$temporary"' EXIT
  curl --fail --location --retry 3 \
    "$APPRUN_URL" \
    --output "$temporary"
  if ! verify_apprun_sha256 "$temporary"; then
    actual_sha256="$(sha256sum "$temporary" | awk '{print $1}')"
    echo "AppRun-x86_64 SHA-256 mismatch: expected $EXPECTED_SHA256, got $actual_sha256" >&2
    exit 1
  fi
  chmod 755 "$temporary"
  mv "$temporary" "$launcher"
  trap - EXIT
fi

if ! verify_apprun_sha256 "$launcher"; then
  actual_sha256="$(sha256sum "$launcher" | awk '{print $1}')"
  echo "AppRun-x86_64 SHA-256 verification failed: expected $EXPECTED_SHA256, got $actual_sha256" >&2
  exit 1
fi
chmod 755 "$launcher"
npm run tauri -- build "$@" -- --locked
