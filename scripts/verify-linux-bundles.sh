#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != Linux || $# -lt 1 || $# -gt 2 ]]; then
  echo "usage (Linux): $0 <version> [--signed]" >&2
  exit 2
fi
version="$1"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "Invalid version" >&2; exit 2; }
if [[ $# == 2 && "$2" != --signed ]]; then
  echo "Unknown option: $2" >&2
  exit 2
fi
bundle_dir=src-tauri/target/release/bundle
deb="$bundle_dir/deb/mimi_${version}_amd64.deb"
appimage="$bundle_dir/appimage/mimi_${version}_amd64.AppImage"
[[ -s "$deb" && -s "$appimage" && -x "$appimage" ]]
[[ "$(dpkg-deb --field "$deb" Package)" == mimi ]]
[[ "$(dpkg-deb --field "$deb" Version)" == "$version" ]]
[[ "$(dpkg-deb --field "$deb" Architecture)" == amd64 ]]
dpkg-deb --field "$deb" Depends | tr ',' '\n' | grep -Eq '^ *libpulse0( |$)'
dpkg-deb --field "$deb" Depends | tr ',' '\n' | grep -Eq '^ *xdg-utils( |$)'
file "$appimage" | grep -q 'ELF 64-bit.*x86-64'

# Inspect the final compressed artifact, not the AppDir left by the bundler.
# -x alone tests the build user's access and misses mode 770 on AppRun.wrapped.
appimage="$(realpath "$appimage")"
extracted="$(mktemp -d -t mimi-appimage-permissions.XXXXXX)"
trap 'rm -rf "$extracted"' EXIT
(cd "$extracted" && "$appimage" --appimage-extract >/dev/null)
python3 scripts/check-appimage-permissions.py "$extracted/squashfs-root"
python3 scripts/check-appimage-gles.py "$extracted/squashfs-root"

if [[ "${2:-}" == --signed ]]; then
  [[ -s "$appimage.sig" ]]
  public_key="$(node -p 'require("./src-tauri/tauri.conf.json").plugins.updater.pubkey')"
  cargo run --release --locked --manifest-path scripts/updater-signature-verifier/Cargo.toml \
    -- "$public_key" "$appimage.sig" "$appimage"
fi

# Install the actual .deb and smoke both package formats independently.
sudo apt-get install --reinstall --no-install-recommends -y "$(realpath "$deb")"
packaged_binary_sha="$(dpkg-deb --fsys-tarfile "$deb" | tar -xOf - usr/bin/mimi | sha256sum | cut -d ' ' -f1)"
installed_binary_sha="$(sha256sum /usr/bin/mimi | cut -d ' ' -f1)"
[[ "$packaged_binary_sha" == "$installed_binary_sha" ]] || {
  echo "Installed Mimi does not match the package under test." >&2
  exit 1
}
# Xlib thread initialization regressions can be intermittent. Fail on the
# first failure; these are independent starts, not retries that hide a crash.
for executable in /usr/bin/mimi "$appimage"; do
  for attempt in 1 2 3; do
    echo "Linux native launch $attempt/3: $executable"
    GDK_SYNCHRONIZE=1 ./scripts/linux-ci-smoke.sh "$executable"
  done
done
echo "Linux packages verified: $deb and $appimage"
