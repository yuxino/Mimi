#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != Linux ]]; then
  echo "This test requires Linux and PulseAudio." >&2
  exit 2
fi
test_prefix=audio::linux::tests::native_
test_names=(
  audio::linux::tests::native_monitor_capture_is_pcm16_and_restarts
  audio::linux::tests::native_microphone_captures_only_explicit_input_and_restarts
  audio::linux::tests::native_dual_inputs_keep_audio_separate_and_restart
  audio::linux::tests::native_default_output_switch_pins_monitor_until_restart
  audio::linux::tests::native_removed_monitor_fails_and_recovers_without_fallback
  audio::linux::tests::native_server_disconnect_fails_and_recovers
)
test_list="$(timeout 120s cargo test --locked --manifest-path src-tauri/Cargo.toml \
  --lib "$test_prefix" -- --ignored --list)"
for test_name in "${test_names[@]}"; do
  if ! grep -Fxq "$test_name: test" <<< "$test_list"; then
    echo "Required Linux audio integration test was not discovered: $test_name" >&2
    exit 1
  fi
done
audio_dir="$(mktemp -d -t mimi-linux-audio.XXXXXX)"
pulse_pid=""
cleanup() {
  if [[ -n "$pulse_pid" ]]; then
    kill "$pulse_pid" 2>/dev/null || true
    wait "$pulse_pid" 2>/dev/null || true
  fi
  rm -rf "$audio_dir"
}
trap cleanup EXIT
export MIMI_TEST_AUDIO_DIRECTORY="$audio_dir"
printf 'mimi isolated synthetic audio smoke\n' > "$audio_dir/isolated-server"
export PULSE_SERVER="unix:$audio_dir/pulse.sock"
export PULSE_RUNTIME_PATH="$audio_dir/runtime"
mkdir -m 700 "$PULSE_RUNTIME_PATH"
# Deliberately make the default source unrelated to the default output. The
# test must capture mimi-output.monitor, never the microphone/default source.
pulseaudio --daemonize=no --exit-idle-time=-1 --use-pid-file=no -n \
  --load="module-native-protocol-unix socket=$audio_dir/pulse.sock auth-anonymous=1" \
  --load="module-null-sink sink_name=mimi-output" \
  --load="module-null-sink sink_name=mimi-microphone" \
  --load="module-remap-source source_name=mimi-input master=mimi-microphone.monitor" \
  --log-target="file:$audio_dir/pulse.log" &
pulse_pid=$!
ready=0
for _ in {1..40}; do
  if timeout 2s pactl info >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 0.25
done
if [[ "$ready" != 1 ]]; then
  cat "$audio_dir/pulse.log" >&2
  exit 1
fi
timeout 2s pactl set-default-sink mimi-output
timeout 2s pactl set-default-source mimi-microphone.monitor
timeout 180s cargo test --locked --manifest-path src-tauri/Cargo.toml \
  --lib "$test_prefix" -- --ignored --nocapture --test-threads=1 \
  --skip native_server_disconnect_fails_and_recovers
# Server lifecycle runs last in a fresh process after the shell-owned server
# has exited. The test owns every daemon it disconnects and restarts.
kill "$pulse_pid"
wait "$pulse_pid" || true
pulse_pid=""
timeout 120s cargo test --locked --manifest-path src-tauri/Cargo.toml \
  --lib audio::linux::tests::native_server_disconnect_fails_and_recovers \
  -- --exact --ignored --nocapture
