#!/usr/bin/env bash
set -euo pipefail

# Ubuntu 22.04 is the oldest supported build baseline with WebKitGTK 4.1.
# Keep this shared by checks and release builds so packaging cannot miss a
# library that the Rust-only jobs happened to install.
# Hosted runners can list an unreachable Azure HTTP mirror before Ubuntu's
# HTTPS archive. Avoid a fresh connection timeout for every package index.
if [[ "${GITHUB_ACTIONS:-}" == "true" && -f /etc/apt/apt-mirrors.txt ]]; then
  sudo sed -i 's|http://azure.archive.ubuntu.com/ubuntu|https://archive.ubuntu.com/ubuntu|g' \
    /etc/apt/apt-mirrors.txt
fi
APT_OPTIONS=(-o Acquire::Retries=2 -o Acquire::http::Timeout=20 -o Acquire::https::Timeout=20)
sudo apt-get "${APT_OPTIONS[@]}" update
sudo apt-get "${APT_OPTIONS[@]}" install --no-install-recommends -y \
  build-essential cmake clang libclang-dev curl wget file libwebkit2gtk-4.1-dev libxdo-dev libssl-dev \
  libayatana-appindicator3-dev librsvg2-dev libpulse-dev libfuse2 libgles2 \
  pulseaudio pulseaudio-utils dbus-x11 gnome-keyring gcr xvfb xauth x11-utils xdotool wmctrl openbox xdg-utils weston
