#!/usr/bin/env bash
set -euo pipefail

# Ubuntu 22.04 is the oldest supported build baseline with WebKitGTK 4.1.
# Keep this shared by checks and release builds so packaging cannot miss a
# library that the Rust-only jobs happened to install.
sudo apt-get update
sudo apt-get install --no-install-recommends -y \
  build-essential cmake clang libclang-dev curl wget file libwebkit2gtk-4.1-dev libxdo-dev libssl-dev \
  libayatana-appindicator3-dev librsvg2-dev libpulse-dev libfuse2 libgles2 \
  pulseaudio pulseaudio-utils dbus-x11 gnome-keyring gcr xvfb xauth x11-utils xdotool wmctrl openbox xdg-utils
