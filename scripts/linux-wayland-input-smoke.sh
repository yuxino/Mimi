#!/usr/bin/env bash
set -euo pipefail

# Explicitly opt-in native protocol fixture. Never attach to an existing desktop
# or alter its settings, input permissions, keyring, or compositor.
[[ "$(uname -s)" == Linux ]] || { echo "Linux required" >&2; exit 2; }
project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir"
python3 -B scripts/check-linux-input-region-trace-test.py
for command in weston dbus-run-session cargo python3 timeout; do
  command -v "$command" >/dev/null || { echo "Missing test dependency: $command" >&2; exit 2; }
done
if [[ "${MIMI_WAYLAND_INPUT_BUS:-}" != 1 ]]; then
  exec dbus-run-session -- env MIMI_WAYLAND_INPUT_BUS=1 "$0"
fi

# Keep the already-selected toolchain/cache when HOME becomes private.
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
work="$(mktemp -d -t mimi-wayland-input.XXXXXX)"
weston_pid=""
cleanup() {
  if [[ -n "$weston_pid" ]]; then
    kill "$weston_pid" 2>/dev/null || true
    wait "$weston_pid" 2>/dev/null || true
  fi
  rm -rf "$work"
}
trap cleanup EXIT
export HOME="$work/home"
export XDG_CONFIG_HOME="$work/config"
export XDG_DATA_HOME="$work/data"
export XDG_CACHE_HOME="$work/cache"
export XDG_RUNTIME_DIR="$work/runtime"
mkdir -m 700 -p "$HOME" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_CACHE_HOME" "$XDG_RUNTIME_DIR"
unset DISPLAY WAYLAND_DISPLAY
export NO_AT_BRIDGE=1
export LIBGL_ALWAYS_SOFTWARE=1
weston --backend=headless-backend.so --use-pixman --no-config --idle-time=0 \
  --socket=mimi-input-wayland >"$work/weston.log" 2>&1 &
weston_pid=$!
ready=0
for _ in {1..80}; do
  if ! kill -0 "$weston_pid" 2>/dev/null; then break; fi
  if [[ -S "$XDG_RUNTIME_DIR/mimi-input-wayland" ]]; then ready=1; break; fi
  sleep 0.1
done
if [[ "$ready" != 1 ]]; then
  cat "$work/weston.log" >&2
  echo "Isolated Weston did not become ready; no existing desktop fallback is allowed" >&2
  exit 1
fi
export WAYLAND_DISPLAY=mimi-input-wayland
export GDK_BACKEND=wayland
export XDG_SESSION_TYPE=wayland
export MIMI_NATIVE_INPUT_SMOKE=1
if ! WAYLAND_DEBUG=client timeout 180s cargo test --locked --manifest-path src-tauri/Cargo.toml \
  windows::linux_input_region::tests::native_input_region_lifecycle -- \
  --ignored --exact --nocapture --test-threads=1 >"$work/trace.log" 2>&1; then
  cat "$work/trace.log" >&2
  exit 1
fi
if ! python3 scripts/check-linux-input-region-trace.py "$work/trace.log"; then
  cat "$work/trace.log" >&2
  exit 1
fi
