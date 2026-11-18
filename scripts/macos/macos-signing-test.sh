#!/usr/bin/env bash
# Regression coverage without accessing any Keychain item or signing key.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/mimi-signing-test.XXXXXX")"
trap 'rm -rf "$TEST_ROOT"' EXIT
mkdir -p "$TEST_ROOT/bin" "$TEST_ROOT/mimi.app/Contents"
cat > "$TEST_ROOT/mimi.app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>app.yuxino.mimi</string>
<key>NSScreenCaptureUsageDescription</key><string>System audio</string>
<key>NSAudioCaptureUsageDescription</key><string>System audio</string>
<key>NSMicrophoneUsageDescription</key><string>Optional microphone input</string>
<key>CFBundleShortVersionString</key><string>1.0.0</string>
<key>MimiSourceRevision</key><string>1111111111111111111111111111111111111111</string>
</dict></plist>
PLIST
cat > "$TEST_ROOT/bin/codesign" <<'STUB'
#!/usr/bin/env bash
case "$*" in
  *--verify*)
    while [[ $# -gt 0 ]]; do
      if [[ "$1" == -R ]]; then
        [[ "${2:-}" == =* ]] || exit 1
      fi
      shift
    done
    exit "${TEST_VERIFY_EXIT:-0}" ;;

  *--entitlements*)
    if [[ " $* " == *' --xml '* ]]; then
      printf '%s\n' "${TEST_ENTITLEMENTS:-<plist version=\"1.0\"><dict><key>com.apple.security.device.audio-input</key><true/></dict></plist>}"
    else
      printf '[Dict]\n  [Key] com.apple.security.device.audio-input\n  [Value]\n    [Bool] true\n'
    fi ;;
  *--requirements*) printf 'designated => %s\n' "$TEST_REQUIREMENT" ;;
  *) printf 'Identifier=app.yuxino.mimi\nSignature=%s\n' "${TEST_SIGNATURE:-signed}" ;;
esac
STUB
cat > "$TEST_ROOT/bin/security" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "${TEST_IDENTITIES:-}"
STUB
chmod +x "$TEST_ROOT/bin/"*
cat > "$TEST_ROOT/bin/lipo" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "${TEST_ARCH:-arm64}"
STUB
chmod +x "$TEST_ROOT/bin/lipo"
export PATH="$TEST_ROOT/bin:$PATH"
export MIMI_LOCAL_CODESIGN_IDENTITY_FILE="$TEST_ROOT/local-identity.txt"
export MIMI_DEV_CODESIGN_IDENTITY_FILE="$TEST_ROOT/dev-identity.txt"
PIN="$(tr -d '[:space:]' < "$SCRIPT_DIR/macos-release-identity.txt")"
export TEST_REQUIREMENT="identifier \"app.yuxino.mimi\" and certificate root = H\"${PIN}\""
expect_failure() {
  if "$@" >"$TEST_ROOT/output" 2>&1; then
    echo "Expected rejection: $*" >&2
    exit 1
  fi
}
expect_identity() {
  local expected="$1"
  shift
  local actual
  actual="$("$@")" || {
    echo "Identity selection unexpectedly failed: $*" >&2
    exit 1
  }
  if [[ "$actual" != "$expected" ]]; then
    echo "Unexpected signing identity: expected $expected, got $actual" >&2
    exit 1
  fi
}
"$SCRIPT_DIR/verify-macos-app.sh" --release "$TEST_ROOT/mimi.app" >/dev/null
for invalid_entitlements in \
  '<plist version="1.0"><dict/></plist>' \
  '<plist version="1.0"><dict><key>com.apple.security.device.audio-input</key><false/></dict></plist>' \
  '<plist version="1.0"><dict><key>com.apple.security.device.audio-input</key><string>true</string></dict></plist>' \
  'invalid plist'; do
  expect_failure env TEST_ENTITLEMENTS="$invalid_entitlements" "$SCRIPT_DIR/verify-macos-app.sh" --release "$TEST_ROOT/mimi.app"
  grep -Fq 'The signed app must enable the audio-input entitlement' "$TEST_ROOT/output"
done
/usr/libexec/PlistBuddy -c 'Delete :NSMicrophoneUsageDescription' "$TEST_ROOT/mimi.app/Contents/Info.plist"
expect_failure "$SCRIPT_DIR/verify-macos-app.sh" --release "$TEST_ROOT/mimi.app"
grep -Fq 'NSMicrophoneUsageDescription must not be empty.' "$TEST_ROOT/output"
/usr/libexec/PlistBuddy -c 'Add :NSMicrophoneUsageDescription string' "$TEST_ROOT/mimi.app/Contents/Info.plist"
expect_failure "$SCRIPT_DIR/verify-macos-app.sh" --release "$TEST_ROOT/mimi.app"
grep -Fq 'NSMicrophoneUsageDescription must not be empty.' "$TEST_ROOT/output"
/usr/libexec/PlistBuddy -c 'Set :NSMicrophoneUsageDescription Optional microphone input' "$TEST_ROOT/mimi.app/Contents/Info.plist"
"$SCRIPT_DIR/verify-macos-app.sh" --release "$TEST_ROOT/mimi.app" >/dev/null
expect_failure env TEST_VERIFY_EXIT=1 "$SCRIPT_DIR/verify-macos-app.sh" --release "$TEST_ROOT/mimi.app"
expect_failure env TEST_SIGNATURE=adhoc "$SCRIPT_DIR/verify-macos-app.sh" --release "$TEST_ROOT/mimi.app"
expect_failure env TEST_REQUIREMENT='cdhash H"123"' "$SCRIPT_DIR/verify-macos-app.sh" --release "$TEST_ROOT/mimi.app"
expect_failure env TEST_REQUIREMENT='identifier "app.yuxino.mimi" and certificate root = H"0000000000000000000000000000000000000000"' "$SCRIPT_DIR/verify-macos-app.sh" --release "$TEST_ROOT/mimi.app"
expect_failure env TEST_REQUIREMENT="$TEST_REQUIREMENT and cdhash H\"123\"" "$SCRIPT_DIR/verify-macos-app.sh" --release "$TEST_ROOT/mimi.app"
expect_failure env MIMI_CODESIGN_IDENTITY=- "$SCRIPT_DIR/codesign-identity.sh"
expect_failure env MIMI_CODESIGN_IDENTITY= TEST_IDENTITIES= "$SCRIPT_DIR/codesign-identity.sh"
export TEST_IDENTITIES="  1) $PIN \"mimi Local Development\""
[[ "$(MIMI_CODESIGN_IDENTITY= "$SCRIPT_DIR/codesign-identity.sh")" == "$PIN" ]]
expect_failure env MIMI_CODESIGN_IDENTITY= TEST_IDENTITIES="$TEST_IDENTITIES
  2) $PIN \"mimi Local Development\"" "$SCRIPT_DIR/codesign-identity.sh"
# A migrated host pin wins over the self-signed default, never silently falls
# back, and cannot affect the separately pinned public-release verification.
LOCAL_TEST_PIN=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
printf '%s\n' "$LOCAL_TEST_PIN" > "$MIMI_LOCAL_CODESIGN_IDENTITY_FILE"
expect_failure env MIMI_CODESIGN_IDENTITY= "$SCRIPT_DIR/codesign-identity.sh"
export TEST_IDENTITIES="$TEST_IDENTITIES
  2) $LOCAL_TEST_PIN \"Apple Development: Signing test\""
[[ "$(MIMI_CODESIGN_IDENTITY= "$SCRIPT_DIR/codesign-identity.sh")" == "$LOCAL_TEST_PIN" ]]
[[ "$(MIMI_CODESIGN_IDENTITY="$PIN" "$SCRIPT_DIR/codesign-identity.sh")" == "$PIN" ]]
expect_failure env MIMI_CODESIGN_IDENTITY= TEST_IDENTITIES="$TEST_IDENTITIES
  3) $LOCAL_TEST_PIN \"Duplicate certificate\"" "$SCRIPT_DIR/codesign-identity.sh"
printf '%s\n' '-' > "$MIMI_LOCAL_CODESIGN_IDENTITY_FILE"
expect_failure env MIMI_CODESIGN_IDENTITY= "$SCRIPT_DIR/codesign-identity.sh"
rm "$MIMI_LOCAL_CODESIGN_IDENTITY_FILE"
mkdir "$MIMI_LOCAL_CODESIGN_IDENTITY_FILE"
expect_failure env MIMI_CODESIGN_IDENTITY= "$SCRIPT_DIR/codesign-identity.sh"
rmdir "$MIMI_LOCAL_CODESIGN_IDENTITY_FILE"
printf '%s\n' "$LOCAL_TEST_PIN" > "$TEST_ROOT/other-pin"
ln -s "$TEST_ROOT/other-pin" "$MIMI_LOCAL_CODESIGN_IDENTITY_FILE"
expect_failure env MIMI_CODESIGN_IDENTITY= "$SCRIPT_DIR/codesign-identity.sh"
rm "$MIMI_LOCAL_CODESIGN_IDENTITY_FILE"
# Development and formal app pins are independent. A formal migration must not
# silently select another certificate for an existing development installation.
expect_identity "$PIN" env MIMI_CODESIGN_IDENTITY= "$SCRIPT_DIR/codesign-identity.sh" --development
printf '%s\n' "$LOCAL_TEST_PIN" > "$MIMI_LOCAL_CODESIGN_IDENTITY_FILE"
expect_identity "$LOCAL_TEST_PIN" env MIMI_CODESIGN_IDENTITY= "$SCRIPT_DIR/codesign-identity.sh"
expect_identity "$PIN" env MIMI_CODESIGN_IDENTITY= "$SCRIPT_DIR/codesign-identity.sh" --development
expect_failure env MIMI_CODESIGN_IDENTITY= TEST_IDENTITIES= "$SCRIPT_DIR/codesign-identity.sh" --development
DEV_TEST_PIN=BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
printf '%s\n' "$DEV_TEST_PIN" > "$MIMI_DEV_CODESIGN_IDENTITY_FILE"
expect_failure env MIMI_CODESIGN_IDENTITY= "$SCRIPT_DIR/codesign-identity.sh" --development
grep -Fq 'the pinned local signing certificate is unavailable or ambiguous' "$TEST_ROOT/output"
export TEST_IDENTITIES="$TEST_IDENTITIES
  3) $DEV_TEST_PIN \"Apple Development: Dev signing test\""
expect_identity "$DEV_TEST_PIN" env MIMI_CODESIGN_IDENTITY= "$SCRIPT_DIR/codesign-identity.sh" --development
expect_identity "$LOCAL_TEST_PIN" env MIMI_CODESIGN_IDENTITY= "$SCRIPT_DIR/codesign-identity.sh"
# Explicit identity overrides retain precedence, while invalid dev pins never
# fall back to either the formal pin or the self-signed identity.
expect_identity "$PIN" env MIMI_CODESIGN_IDENTITY="$PIN" "$SCRIPT_DIR/codesign-identity.sh" --development
printf '%s\n' '-' > "$MIMI_DEV_CODESIGN_IDENTITY_FILE"
expect_failure env MIMI_CODESIGN_IDENTITY= "$SCRIPT_DIR/codesign-identity.sh" --development
expect_identity "$LOCAL_TEST_PIN" env MIMI_CODESIGN_IDENTITY= "$SCRIPT_DIR/codesign-identity.sh"
expect_identity "$PIN" env MIMI_CODESIGN_IDENTITY="$PIN" "$SCRIPT_DIR/codesign-identity.sh" --development
expect_failure env MIMI_CODESIGN_IDENTITY=- "$SCRIPT_DIR/codesign-identity.sh" --development
rm "$MIMI_DEV_CODESIGN_IDENTITY_FILE"
ln -s "$MIMI_LOCAL_CODESIGN_IDENTITY_FILE" "$MIMI_DEV_CODESIGN_IDENTITY_FILE"
expect_failure env MIMI_CODESIGN_IDENTITY= "$SCRIPT_DIR/codesign-identity.sh" --development
rm "$MIMI_DEV_CODESIGN_IDENTITY_FILE"
printf '%s\n' '-' > "$MIMI_LOCAL_CODESIGN_IDENTITY_FILE"
expect_identity "$PIN" env MIMI_CODESIGN_IDENTITY= "$SCRIPT_DIR/codesign-identity.sh" --development
expect_failure env MIMI_CODESIGN_IDENTITY="$PIN" "$SCRIPT_DIR/codesign-identity.sh" --unknown
"$SCRIPT_DIR/verify-macos-release-source.sh" "$TEST_ROOT/mimi.app" 1111111111111111111111111111111111111111 1.0.0 arm64 >/dev/null
expect_failure "$SCRIPT_DIR/verify-macos-release-source.sh" "$TEST_ROOT/mimi.app" 2222222222222222222222222222222222222222 1.0.0 arm64
expect_failure "$SCRIPT_DIR/verify-macos-release-source.sh" "$TEST_ROOT/mimi.app" 1111111111111111111111111111111111111111 2.0.0 arm64
expect_failure "$SCRIPT_DIR/verify-macos-release-source.sh" "$TEST_ROOT/mimi.app" 1111111111111111111111111111111111111111 1.0.0 x86_64
TEST_ARCH=x86_64 "$SCRIPT_DIR/verify-macos-release-source.sh" "$TEST_ROOT/mimi.app" 1111111111111111111111111111111111111111 1.0.0 x86_64 >/dev/null
echo 'macOS signing safety tests passed.'
