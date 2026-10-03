#!/usr/bin/env bash
# Sign only a release APK, using an existing, backed-up release identity.
set -euo pipefail
cd "$(dirname "$0")"

: "${ANDROID_HOME:?Set ANDROID_HOME to the Android SDK directory}"
: "${ANDROID_KEYSTORE_PATH:?Set the path to the existing release keystore}"
: "${ANDROID_KEYSTORE_PASSWORD:?Set the keystore password}"
: "${ANDROID_KEY_ALIAS:?Set the release key alias}"
: "${ANDROID_KEY_PASSWORD:?Set the key password}"
: "${ANDROID_SIGNING_CERT_SHA256:?Set the pinned release certificate SHA-256}"

sdk_tools="$ANDROID_HOME/build-tools/35.0.0"
input="${1:-app/build/outputs/apk/release/app-release-unsigned.apk}"
version="$(sed -n 's/^versionName=//p' version.properties)"
version_code="$(sed -n 's/^versionCode=//p' version.properties)"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]
[[ "$version_code" =~ ^[1-9][0-9]*$ ]]

# Inspect the compiled manifest rather than trusting the filename.
badging="$("$sdk_tools/aapt" dump badging "$input")"
package_line="${badging%%$'\n'*}"
[[ "$package_line" == *"name='app.yuxino.mimi.android'"* ]]
[[ "$package_line" == *"versionCode='$version_code'"* ]]
[[ "$package_line" == *"versionName='$version'"* ]]
if grep -q '^application-debuggable' <<< "$badging"; then
  echo 'Refusing to sign a debuggable APK.' >&2
  exit 1
fi

expected="$(printf '%s' "$ANDROID_SIGNING_CERT_SHA256" | tr -d ':[:space:]' | tr '[:upper:]' '[:lower:]')"
[[ "$expected" =~ ^[a-f0-9]{64}$ ]]
mkdir -p release
work="$(mktemp -d release/.sign-XXXXXX)"
trap 'rm -rf "$work"' EXIT
python_command="${MIMI_PYTHON:-python3}"
"$python_command" ../scripts/verify-shared-core.py --apk "$input"
"$sdk_tools/zipalign" -f -P 16 4 "$input" "$work/aligned.apk"
"$sdk_tools/apksigner" sign \
  --ks "$ANDROID_KEYSTORE_PATH" --ks-key-alias "$ANDROID_KEY_ALIAS" \
  --ks-pass env:ANDROID_KEYSTORE_PASSWORD --key-pass env:ANDROID_KEY_PASSWORD \
  --out "$work/signed.apk" "$work/aligned.apk"
verification="$("$sdk_tools/apksigner" verify --verbose --print-certs "$work/signed.apk")"
actual="$(sed -n 's/^Signer #1 certificate SHA-256 digest: //p' <<< "$verification" | tr '[:upper:]' '[:lower:]')"
if [[ "$actual" != "$expected" ]] || grep -qi 'CN=Android Debug' <<< "$verification"; then
  echo 'Release certificate does not match the pinned non-debug identity.' >&2
  exit 1
fi
"$sdk_tools/zipalign" -c -P 16 4 "$work/signed.apk"
"$python_command" ../scripts/verify-shared-core.py --apk "$work/signed.apk"
output="mimi_${version}_android.apk"
mv "$work/signed.apk" "release/$output"
(cd release && sha256sum "$output" > SHA256SUMS.txt)
printf 'Verified release APK: %s\nCertificate SHA-256: %s\n' "release/$output" "$actual"
