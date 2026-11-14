#!/usr/bin/env bash
set -euo pipefail

audio_dir="$(mktemp -d -t mimi-pipewire.XXXXXX)"
pids=()
cleanup() {
  local exit_status=$?
  local pid
  for pid in "${pids[@]}"; do
    kill "$pid" 2>/dev/null || true
  done
  for pid in "${pids[@]}"; do
    wait "$pid" 2>/dev/null || true
  done
  if (( exit_status != 0 )); then
    for log in "$audio_dir"/*.log; do
      [[ ! -f "$log" ]] || cat "$log" >&2
    done
  fi
  rm -rf -- "$audio_dir"
}
trap cleanup EXIT

export MIMI_TEST_AUDIO_DIRECTORY="$audio_dir"
export PULSE_RUNTIME_PATH="$audio_dir/runtime"
export PULSE_SERVER="unix:$audio_dir/pulse.sock"
export XDG_CONFIG_HOME="$audio_dir/config"
export XDG_STATE_HOME="$audio_dir/state"
export XDG_RUNTIME_DIR="$audio_dir/runtime"
export PIPEWIRE_RUNTIME_DIR="$XDG_RUNTIME_DIR"
unset PIPEWIRE_REMOTE
mkdir -m 700 -p "$XDG_CONFIG_HOME/pipewire" "$XDG_STATE_HOME" "$XDG_RUNTIME_DIR"
printf 'mimi isolated synthetic audio smoke\n' > "$audio_dir/isolated-server"
sed "s|\"unix:native\"|\"$PULSE_SERVER\"|" /usr/share/pipewire/pipewire-pulse.conf \
  > "$XDG_CONFIG_HOME/pipewire/pipewire-pulse.conf"

pipewire > "$audio_dir/core.log" 2>&1 &
pids+=("$!")
for _ in {1..40}; do
  [[ ! -S "$XDG_RUNTIME_DIR/pipewire-0" ]] || break
  sleep 0.25
done
test -S "$XDG_RUNTIME_DIR/pipewire-0"
wireplumber > "$audio_dir/session-manager.log" 2>&1 &
pids+=("$!")
pipewire-pulse > "$audio_dir/pulse.log" 2>&1 &
pids+=("$!")
for _ in {1..40}; do
  if [[ -S "$audio_dir/pulse.sock" ]] && timeout 2s pactl info >/dev/null 2>&1; then
    break
  fi
  sleep 0.25
done
test -S "$audio_dir/pulse.sock"
server_info="$(LC_ALL=C timeout 2s pactl info)"
grep '^Server Name: .*on PipeWire ' <<< "$server_info"

timeout 5s pactl load-module module-null-sink sink_name=mimi-output
timeout 5s pactl load-module module-null-sink sink_name=mimi-unrelated
timeout 2s pactl set-default-sink mimi-output
timeout 2s pactl set-default-source mimi-unrelated.monitor
test "$(timeout 2s pactl get-default-sink)" = mimi-output
test "$(timeout 2s pactl get-default-source)" = mimi-unrelated.monitor

# Use the maintained PCM and restart test with its original assertions.
test_name=audio::linux::tests::native_monitor_capture_is_pcm16_and_restarts
tests="$(timeout 120s cargo test --locked --manifest-path src-tauri/Cargo.toml \
  --lib "$test_name" -- --exact --ignored --list)"
grep -Fxq "$test_name: test" <<< "$tests"
timeout 30s cargo test --locked --manifest-path src-tauri/Cargo.toml \
  --lib "$test_name" -- --exact --ignored --nocapture
