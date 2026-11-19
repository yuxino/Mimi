#!/usr/bin/env bash
# Temporary hosted-only signing session. Never launches Mimi or changes TCC.
set -euo pipefail
set +x
umask 077

fail() { echo "error: $*" >&2; exit 1; }
[[ "${GITHUB_ACTIONS:-}" == true && "${RUNNER_ENVIRONMENT:-}" == github-hosted && "${RUNNER_OS:-}" == macOS ]] || fail 'Requires a disposable GitHub-hosted macOS runner.'
[[ "$(uname -s)" == Darwin && "$(/usr/bin/sw_vers -productVersion)" == 15.* ]] || fail 'Requires macOS 15.'
[[ "${GITHUB_REPOSITORY:-}" == yuxino/Mimi ]] || fail 'Requires the Mimi repository.'
[[ "${GITHUB_RUN_ID:-}" =~ ^[0-9]+$ && "${GITHUB_RUN_ATTEMPT:-}" =~ ^[0-9]+$ ]] || fail 'Missing run identity.'
[[ -d "${RUNNER_TEMP:-}" && "$RUNNER_TEMP" == /* && ! -L "$RUNNER_TEMP" ]] || fail 'Invalid runner temporary directory.'

session="$RUNNER_TEMP/mimi-caption-build-$GITHUB_RUN_ID-$GITHUB_RUN_ATTEMPT"
keychain="$session/signing.keychain-db"
certificate="$session/certificate.pem"
app=/Applications/mimi-dev.app
identifier=app.yuxino.mimi.dev
owner="$GITHUB_RUN_ID:$GITHUB_RUN_ATTEMPT:${GITHUB_JOB:-job}"

owned_session() {
  [[ -d "$session" && ! -L "$session" && -f "$session/owner" && ! -L "$session/owner" &&
     "$(/usr/bin/stat -f '%u' "$session")" == "$(id -u)" && "$(cat "$session/owner")" == "$owner" ]]
}

restore_search_list() {
  local original=() path
  while IFS= read -r path; do original+=("$path"); done < "$session/original-keychains"
  [[ ${#original[@]} -gt 0 ]] || return 1
  /usr/bin/security list-keychains -d user -s "${original[@]}"
}

cleanup() {
  [[ -e "$session" ]] || return 0
  owned_session || { echo 'Unrecognized signing session preserved.' >&2; return 1; }
  rm -f "$session/key.pem" "$session/identity.p12"
  if [[ -f "$session/identity.sha1" && -d "$app" ]]; then
    local fingerprint
    fingerprint="$(cat "$session/identity.sha1")"
    if [[ -n "$(/usr/sbin/lsof -t "$app/Contents/MacOS/mimi" 2>/dev/null || true)" ]]; then
      echo 'Test app is still in use; signing session preserved for job disposal.' >&2
      return 1
    fi
    if /usr/bin/codesign --verify --strict -R "=identifier \"$identifier\" and certificate leaf = H\"$fingerprint\"" "$app" 2>/dev/null; then
      rm -rf "$app"
    fi
  fi
  local result=0
  if [[ -e "$keychain" ]]; then
    restore_search_list || result=1
    /usr/bin/security delete-keychain "$keychain" || result=1
  fi
  [[ "$result" == 0 ]] || { echo 'Signing cleanup incomplete; private session preserved.' >&2; return 1; }
  rm -rf "$session"
}

case "${1:-}" in
  --cleanup) [[ $# == 1 ]] || fail 'Usage: hosted-build.sh --cleanup'; cleanup; exit ;;
  --no-launch) [[ $# == 2 && "$2" =~ ^[0-9a-f]{40}$ ]] || fail 'Usage: hosted-build.sh --no-launch FULL_SHA'; sha="$2" ;;
  *) fail 'Use --no-launch FULL_SHA, or --cleanup.' ;;
esac

[[ -d "${GITHUB_WORKSPACE:-}/.git" || -f "${GITHUB_WORKSPACE:-}/.git" ]] || fail 'Missing source checkout.'
cd "$GITHUB_WORKSPACE"
[[ "$(git rev-parse HEAD)" == "$sha" && -z "$(git status --porcelain)" ]] || fail 'Source must be the exact clean requested SHA.'
export DEVELOPER_DIR=/Applications/Xcode_26.3.app/Contents/Developer
[[ -d "$DEVELOPER_DIR" && "$(/usr/bin/xcodebuild -version | head -1)" == 'Xcode 26.3' ]] || fail 'Xcode 26.3 is required.'
[[ -x scripts/dev-app.sh ]] || fail 'Missing normal development builder.'
[[ -z "$(/usr/sbin/lsof -t "$app/Contents/MacOS/mimi" 2>/dev/null || true)" ]] || fail 'Close the test app before rebuilding.'

on_exit() {
  local status=$?
  if [[ "$status" != 0 ]]; then cleanup || true; fi
  exit "$status"
}
trap on_exit EXIT

if [[ ! -e "$session" ]]; then
  [[ ! -e "$app" && ! -L "$app" ]] || fail 'A pre-existing development app must not be replaced by this new identity.'
  mkdir -m 700 "$session"
  printf '%s' "$owner" > "$session/owner"
  /usr/bin/security list-keychains -d user | python3 -c 'import shlex,sys; print("\n".join(shlex.split(sys.stdin.read())))' > "$session/original-keychains"
  [[ -s "$session/original-keychains" ]] || fail 'Cannot preserve the original search list.'
  keychain_password="$(/usr/bin/openssl rand -hex 24)"
  archive_password="$(/usr/bin/openssl rand -hex 24)"
  printf '::add-mask::%s\n::add-mask::%s\n' "$keychain_password" "$archive_password"
  name="Mimi Hosted Caption $GITHUB_RUN_ID-$GITHUB_RUN_ATTEMPT"
  cat > "$session/certificate.cnf" <<EOF
[req]
prompt=no
distinguished_name=subject
x509_extensions=signing
[subject]
CN=$name
[signing]
basicConstraints=critical,CA:FALSE
keyUsage=critical,digitalSignature
extendedKeyUsage=codeSigning
subjectKeyIdentifier=hash
EOF
  /usr/bin/openssl req -new -newkey rsa:2048 -nodes -x509 -sha256 -days 2 \
    -config "$session/certificate.cnf" -keyout "$session/key.pem" -out "$certificate" 2> "$session/openssl.log"
  /usr/bin/openssl pkcs12 -export -name "$name" -inkey "$session/key.pem" -in "$certificate" \
    -out "$session/identity.p12" -passout "pass:$archive_password"
  /usr/bin/security create-keychain -p "$keychain_password" "$keychain"
  /usr/bin/security set-keychain-settings -lut 21600 "$keychain"
  /usr/bin/security unlock-keychain -p "$keychain_password" "$keychain"
  /usr/bin/security import "$session/identity.p12" -k "$keychain" -P "$archive_password" -x -T /usr/bin/codesign >/dev/null
  # Apple's security(1) requires apple: for codesign; the trusted-app ACL above
  # remains /usr/bin/codesign only. This keychain contains just this new key.
  /usr/bin/security set-key-partition-list -S apple: -s -t private -k "$keychain_password" "$keychain" >/dev/null
  /usr/bin/openssl x509 -in "$certificate" -noout -fingerprint -sha1 | sed 's/.*=//' | tr -d ':\r\n ' > "$session/identity.sha1"
  [[ "$(cat "$session/identity.sha1")" =~ ^[0-9A-F]{40}$ ]] || fail 'Invalid test certificate fingerprint.'
  # create-keychain can update the search list. Preserve every old path and
  # add only the new private keychain for this signing session; restore on exit.
  original=()
  while IFS= read -r path; do original+=("$path"); done < "$session/original-keychains"
  /usr/bin/security list-keychains -d user -s "$keychain" "${original[@]}"
  rm -f "$session/key.pem" "$session/identity.p12"
  unset keychain_password archive_password
else
  owned_session || fail 'Signing session ownership mismatch.'
fi

export MIMI_CODESIGN_IDENTITY="$(cat "$session/identity.sha1")"
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export CARGO_TARGET_DIR="$GITHUB_WORKSPACE/src-tauri/target"
export npm_config_cache="${npm_config_cache:-$HOME/.npm}"
export MIMI_DEV_APP_PATH="$app"
unset MIMI_DEV_WATCH MIMI_DEV_RECOVERY_ONLY MIMI_ALLOW_IDENTITY_CHANGE
npm ci --prefer-offline --no-audit --no-fund
./scripts/dev-app.sh --ui-only --no-launch
/usr/bin/codesign --verify --deep --strict "$app"
/usr/bin/codesign --verify --strict -R "=identifier \"$identifier\" and certificate leaf = H\"$MIMI_CODESIGN_IDENTITY\"" "$app"
requirement="$(/usr/bin/codesign -d -r- "$app" 2>&1 | sed -n 's/^designated => //p')"
[[ -n "$requirement" && "$requirement" != cdhash\ * ]] || fail 'Missing stable designated requirement.'
binary_sha="$(/usr/bin/shasum -a 256 "$app/Contents/MacOS/mimi" | awk '{print $1}')"
python3 - "$session/build-$sha.json" "$sha" "$MIMI_CODESIGN_IDENTITY" "$binary_sha" "$requirement" <<'PY'
import json,sys
path,sha,certificate,binary,requirement=sys.argv[1:]
with open(path,'w') as f:
    json.dump(dict(sourceSha=sha,certificateSha1=certificate,binarySha256=binary,
        app='/Applications/mimi-dev.app',identifier='app.yuxino.mimi.dev',profile='local-dev',
        builderArgs=['--ui-only','--no-launch'],launched=False,xcode='26.3',designatedRequirement=requirement),f)
PY
printf 'Built SHA=%s profile=local-dev app=%s certificateSHA1=%s binarySHA256=%s; no launch performed.\n' "$sha" "$app" "$MIMI_CODESIGN_IDENTITY" "$binary_sha"
if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  printf 'source_sha=%s\nidentity_sha1=%s\napp_path=%s\nreceipt=%s\n' "$sha" "$MIMI_CODESIGN_IDENTITY" "$app" "$session/build-$sha.json" >> "$GITHUB_OUTPUT"
fi
# Keep only the encrypted keychain + public certificate/receipt until the
# same-job native check (and optional second SHA). Always run --cleanup last.
