#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != Linux ]]; then
  echo "This test requires Linux and GNOME Keyring." >&2
  exit 2
fi
if [[ "${MIMI_LINUX_KEYRING_SESSION:-}" != 1 ]]; then
  exec dbus-run-session -- env MIMI_LINUX_KEYRING_SESSION=1 "$0"
fi

test_name=settings_store::tests::native_credentials::linux_secret_service_roundtrip
test_list="$(timeout 120s cargo test --locked --manifest-path src-tauri/Cargo.toml \
  --lib "$test_name" -- --exact --ignored --list)"
if ! grep -Fxq "$test_name: test" <<< "$test_list"; then
  echo "Required Linux credential integration test was not discovered: $test_name" >&2
  exit 1
fi

secret_dir="$(mktemp -d -t mimi-linux-keyring.XXXXXX)"
keyring_pid=""
cleanup() {
  if [[ -n "$keyring_pid" ]]; then
    kill "$keyring_pid" 2>/dev/null || true
    wait "$keyring_pid" 2>/dev/null || true
  fi
  rm -rf "$secret_dir"
}
trap cleanup EXIT
export XDG_CONFIG_HOME="$secret_dir/config"
export XDG_DATA_HOME="$secret_dir/data"
export XDG_CACHE_HOME="$secret_dir/cache"
export XDG_RUNTIME_DIR="$secret_dir/runtime"
mkdir -m 700 -p "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_CACHE_HOME" "$XDG_RUNTIME_DIR" "$secret_dir/control"

# This password protects only an ephemeral, synthetic CI keyring. No user
# credential or host keyring is opened; the session bus and data path are new.
printf '%s' 'mimi-ci-synthetic-keyring' | gnome-keyring-daemon \
  --foreground --unlock --components=secrets --control-directory="$secret_dir/control" \
  >"$secret_dir/keyring.log" 2>&1 &
keyring_pid=$!
ready=0
for _ in {1..40}; do
  # Poll the bus first without activating a competing locked daemon.
  if timeout 2s gdbus call --session --dest org.freedesktop.DBus \
    --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.NameHasOwner \
    org.freedesktop.secrets 2>/dev/null | grep -q true \
    && timeout 2s gdbus call --session --dest org.freedesktop.secrets \
    --object-path /org/freedesktop/secrets/collection/login \
    --method org.freedesktop.DBus.Properties.Get \
    org.freedesktop.Secret.Collection Locked 2>/dev/null | grep -q false; then
    ready=1
    break
  fi
  sleep 0.25
done
if [[ "$ready" != 1 ]]; then
  echo "The isolated Secret Service did not unlock its synthetic keyring." >&2
  cat "$secret_dir/keyring.log" >&2
  exit 1
fi
MIMI_TEST_SECRET_SERVICE=1 timeout 120s cargo test --locked --manifest-path src-tauri/Cargo.toml \
  --lib "$test_name" -- --exact --ignored --nocapture
