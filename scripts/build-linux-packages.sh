#!/usr/bin/env bash
set -euo pipefail
[[ "$(uname -s)" == Linux ]] || { echo 'Linux packaging requires Linux.' >&2; exit 2; }
[[ "$(uname -m)" == x86_64 ]] || { echo 'Public Linux packages target x86_64.' >&2; exit 2; }
cd "$(dirname "$0")/.."
# WebKit loads GLES at runtime, so linuxdeploy's ELF dependency scan misses it.
# Copy only Ubuntu's vendor-neutral GLES entry points. EGL, GLdispatch and the
# actual Mesa/NVIDIA drivers must stay with the host's graphics stack.
gles_library=/usr/lib/x86_64-linux-gnu/libGLESv2.so.2
[[ "$(dpkg-query -W -f='${source:Package}' libgles2:amd64)" == libglvnd ]] || {
  echo 'AppImage packaging requires the distribution libgles2 (libglvnd) package.' >&2
  exit 1
}
python3 scripts/check-appimage-gles.py --library "$gles_library"
gles_config='{"bundle":{"linux":{"appimage":{"files":{"/usr/lib/libGLESv2.so.2":"/usr/lib/x86_64-linux-gnu/libGLESv2.so.2","/usr/share/doc/mimi/libgles2-copyright":"/usr/share/doc/libgles2/copyright"}}}}}'
# Tauri downloads this launcher with mode 770. linuxdeploy renames it to
# AppRun.wrapped, retaining that mode, so unrelated users cannot execute it.
# Seed the same upstream launcher with distribution-safe permissions before
# Tauri copies it into the AppDir. Keep this before updater signing.
export XDG_CACHE_HOME="${XDG_CACHE_HOME:-$HOME/.cache}"
launcher="$XDG_CACHE_HOME/tauri/AppRun-x86_64"
mkdir -p "$(dirname "$launcher")"
if [[ ! -s "$launcher" ]]; then
  temporary="$(mktemp "${launcher}.XXXXXX")"
  trap 'rm -f "$temporary"' EXIT
  curl --fail --location --retry 3 \
    https://github.com/tauri-apps/binary-releases/releases/download/apprun-old/AppRun-x86_64 \
    --output "$temporary"
  chmod 755 "$temporary"
  mv "$temporary" "$launcher"
fi
chmod 755 "$launcher"
npm run tauri -- build --config "$gles_config" "$@" -- --locked
