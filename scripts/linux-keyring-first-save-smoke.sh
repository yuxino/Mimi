#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != Linux ]]; then
  echo "This test requires Linux, GNOME Keyring and its native prompt." >&2
  exit 2
fi

test_name=settings_store::tests::linux_secret_service_first_save_and_restart
if [[ "${MIMI_LINUX_FIRST_SAVE_SESSION:-}" != 1 ]]; then
  test_list="$(timeout 120s cargo test --locked --manifest-path src-tauri/Cargo.toml \
    --lib "$test_name" -- --exact --ignored --list)"
  if ! grep -Fxq "$test_name: test" <<< "$test_list"; then
    echo "Required Linux first-save integration test was not discovered." >&2
    exit 1
  fi
  secret_dir="$(mktemp -d -t mimi-linux-first-save.XXXXXX)"
  trap 'rm -rf "$secret_dir"' EXIT
  mkdir -m 700 -p "$secret_dir"/{config,data,cache,runtime,control,bus}
  cat > "$secret_dir/session.conf" <<CONFIG
<busconfig>
  <type>session</type>
  <listen>unix:tmpdir=$secret_dir/bus</listen>
  <auth>EXTERNAL</auth>
  <standard_session_servicedirs/>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
CONFIG
  # Only a new private bus/display/data directory is used. No host keyring or
  # user password is involved; the native prompt still chooses encryption.
  env LC_ALL=C XDG_CONFIG_HOME="$secret_dir/config" XDG_DATA_HOME="$secret_dir/data" \
    XDG_CACHE_HOME="$secret_dir/cache" XDG_RUNTIME_DIR="$secret_dir/runtime" \
    MIMI_LINUX_FIRST_SAVE_SESSION=1 MIMI_TEST_PRIVATE_KEYRING_DIRECTORY="$secret_dir" \
    xvfb-run -a dbus-run-session --config-file="$secret_dir/session.conf" -- "$0"
  exit
fi

secret_dir="$MIMI_TEST_PRIVATE_KEYRING_DIRECTORY"
keyring_pid=""
window_manager_pid=""
test_pid=""
cleanup() {
  for process in "$test_pid" "$keyring_pid" "$window_manager_pid"; do
    if [[ -n "$process" ]]; then
      kill "$process" 2>/dev/null || true
      wait "$process" 2>/dev/null || true
    fi
  done
}
trap cleanup EXIT
openbox >"$secret_dir/openbox.log" 2>&1 &
window_manager_pid=$!
gnome-keyring-daemon --foreground --components=secrets \
  --control-directory="$secret_dir/control" >"$secret_dir/keyring.log" 2>&1 &
keyring_pid=$!
ready=0
for _ in {1..80}; do
  if timeout 2s gdbus call --session --dest org.freedesktop.DBus \
    --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.NameHasOwner \
    org.freedesktop.secrets 2>/dev/null | grep -q true; then
    ready=1
    break
  fi
  sleep 0.25
done
[[ "$ready" == 1 ]] || { echo "Isolated Secret Service did not start." >&2; exit 1; }

MIMI_TEST_SECRET_SERVICE_FIRST_SAVE=write timeout 180s \
  cargo test --locked --manifest-path src-tauri/Cargo.toml --lib "$test_name" \
  -- --exact --ignored --nocapture >"$secret_dir/test.log" 2>&1 &
test_pid=$!
for phase in cancel accept; do
  prompt=""
  for _ in {1..240}; do
    if ! kill -0 "$test_pid" 2>/dev/null; then
      cat "$secret_dir/test.log" >&2
      echo "First-save test exited before the $phase prompt." >&2
      exit 1
    fi
    if [[ -f "$secret_dir/first-save-phase" ]] \
      && [[ "$(cat "$secret_dir/first-save-phase")" == "$phase" ]]; then
      prompt="$(xdotool search --onlyvisible --class '[Gg]cr-prompter' 2>/dev/null | head -1 || true)"
      if [[ -n "$prompt" ]]; then break; fi
    fi
    sleep 0.25
  done
  [[ -n "$prompt" ]] || { echo "Native keyring $phase prompt did not appear." >&2; exit 1; }
  xdotool windowactivate --sync "$prompt"
  if [[ "$phase" == cancel ]]; then
    xdotool key --clearmodifiers Escape
  else
    xdotool type --clearmodifiers 'mimi-ci-synthetic-first-save'
    xdotool key Tab
    xdotool type --clearmodifiers 'mimi-ci-synthetic-first-save'
    xdotool key Return
  fi
done
status=0
wait "$test_pid" || status=$?
test_pid=""
cat "$secret_dir/test.log"
exit "$status"
