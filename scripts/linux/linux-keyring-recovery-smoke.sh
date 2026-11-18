#!/usr/bin/env bash
set -euo pipefail
if [[ "$(uname -s)" != Linux ]]; then
  echo "This test requires Linux and GNOME Keyring." >&2
  exit 2
fi
test_name=settings_store::tests::native_credentials::linux_secret_service_recovers_without_restart
test_list="$(timeout 120s cargo test --locked --manifest-path src-tauri/Cargo.toml --lib "$test_name" -- --exact --ignored --list)"
if ! grep -Fxq "$test_name: test" <<< "$test_list"; then
  echo "Required Linux recovery integration test was not discovered." >&2
  exit 1
fi
secret_dir="$(mktemp -d -t mimi-linux-keyring-recovery.XXXXXX)"
trap 'rm -rf "$secret_dir"' EXIT
mkdir -m 700 -p "$secret_dir"/{config,data,cache,runtime,control,bus}
# A private bus with no service activation directories ensures the service is
# genuinely absent first. This config never changes the user's session or PAM.
cat > "$secret_dir/session.conf" <<CONFIG
<busconfig>
  <type>session</type>
  <listen>unix:tmpdir=$secret_dir/bus</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
CONFIG
env XDG_CONFIG_HOME="$secret_dir/config" XDG_DATA_HOME="$secret_dir/data" \
  XDG_CACHE_HOME="$secret_dir/cache" XDG_RUNTIME_DIR="$secret_dir/runtime" \
  MIMI_TEST_SECRET_SERVICE_RECOVERY=1 MIMI_TEST_PRIVATE_KEYRING_DIRECTORY="$secret_dir" \
  dbus-run-session --config-file="$secret_dir/session.conf" -- timeout 180s \
  cargo test --locked --manifest-path src-tauri/Cargo.toml --lib "$test_name" -- --exact --ignored --nocapture
