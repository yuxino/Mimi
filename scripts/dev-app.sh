#!/usr/bin/env bash
set -euo pipefail

# Stable macOS development launcher for mimi.
#
# A bare/ad-hoc binary receives a new designated requirement after rebuilds,
# so macOS can treat it as a new application and ask for capture permission
# again. This launcher builds a real, stably signed dev bundle, installs it at
# one canonical path, and verifies the exact process that was opened.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd -P)"
TARGET_DIR="${CARGO_TARGET_DIR:-$PROJECT_DIR/src-tauri/target}"
if [[ "$TARGET_DIR" != /* ]]; then
  TARGET_DIR="$PROJECT_DIR/$TARGET_DIR"
fi
BUILD_DIR="$TARGET_DIR/local-dev"
BUILD_APP="$BUILD_DIR/mimi-dev.app"
DEV_TAURI_CONFIG="$PROJECT_DIR/src-tauri/tauri.dev.conf.json"
RELEASE_TAURI_CONFIG="$PROJECT_DIR/src-tauri/tauri.conf.json"
CANONICAL_APP="${MIMI_DEV_APP_PATH:-/Applications/mimi-dev.app}"
BUNDLE_IDENTIFIER="app.yuxino.mimi.dev"
RELEASE_BUNDLE_IDENTIFIER="app.yuxino.mimi"
MODE="live"
SHOULD_LAUNCH=1
EVIDENCE_WORKSPACE=""

usage() {
  cat <<'EOF'
Usage: ./scripts/dev-app.sh [--ui-only] [--no-launch] [--evidence-workspace NAME]

  --ui-only   Open local UI fixtures without credentials, network, or audio capture.
  --no-launch Build and install the canonical development app without opening it.
  --evidence-workspace NAME  Select a private dev evidence batch; recordings remain off until explicitly enabled.

Live development always uses a stable signing identity and a fixed app path.
The default is /Applications/mimi-dev.app. Override it only with another path
that will remain stable:

  MIMI_DEV_APP_PATH="$HOME/Applications/mimi-dev.app" ./scripts/dev-app.sh
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --ui-only)
      MODE="ui-only"
      ;;
    --no-launch)
      SHOULD_LAUNCH=0
      ;;
    --evidence-workspace)
      if [[ $# -lt 2 || ! "$2" =~ ^[A-Za-z0-9][A-Za-z0-9_-]{0,47}$ ]]; then
        echo "error: evidence workspace must be 1–48 ASCII letters, digits, underscores or hyphens, starting with a letter or digit." >&2
        exit 2
      fi
      EVIDENCE_WORKSPACE="$2"
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "error: scripts/dev-app.sh is the stable macOS launcher." >&2
  echo "Use npm run tauri:dev on Windows." >&2
  exit 2
fi

APP_VERSION="$(/usr/bin/plutil -extract version raw "$RELEASE_TAURI_CONFIG")"
if [[ ! "$APP_VERSION" =~ ^[0-9]+([.][0-9]+){2}([-+][0-9A-Za-z.-]+)?$ ]]; then
  echo "error: invalid application version in $RELEASE_TAURI_CONFIG" >&2
  exit 1
fi

designated_requirement() {
  codesign --display --requirements - "$1" 2>&1 \
    | /usr/bin/sed -n 's/^designated => //p'
}

process_executable_path() {
  local pid="$1"

  /usr/sbin/lsof -a -p "$pid" -d txt -Fn 2>/dev/null \
    | /usr/bin/sed -n 's/^n//p' \
    | /usr/bin/head -n 1 \
    || true
}

exact_executable_pids() {
  local expected="$1"
  local executable
  local pid

  while IFS= read -r pid; do
    [[ -z "$pid" ]] && continue
    executable="$(process_executable_path "$pid")"
    if [[ "$executable" == "$expected" ]]; then
      echo "$pid"
    fi
  done < <(/usr/bin/pgrep -U "$(id -u)" -x mimi || true)
}

terminate_exact_executable() {
  local executable="$1"
  local pid
  local remaining=()

  while IFS= read -r pid; do
    [[ -z "$pid" ]] && continue
    kill -TERM "$pid"
  done < <(exact_executable_pids "$executable")

  for _ in {1..50}; do
    remaining=()
    while IFS= read -r pid; do
      [[ -n "$pid" ]] && remaining+=("$pid")
    done < <(exact_executable_pids "$executable")
    [[ ${#remaining[@]} -eq 0 ]] && return 0
    sleep 0.1
  done

  echo "error: mimi did not stop cleanly (pid ${remaining[*]})." >&2
  echo "Quit that app and run the development launcher again." >&2
  return 1
}

running_conflicting_mimi_processes() {
  local allowed_executable="$1"
  local app_bundle
  local bundle_identifier
  local executable
  local pid

  while IFS= read -r pid; do
    executable="$(process_executable_path "$pid")"
    [[ -z "$executable" || "$executable" == "$allowed_executable" ]] && continue
    app_bundle="${executable%/Contents/MacOS/*}"
    [[ "$app_bundle" == "$executable" ]] && continue
    bundle_identifier="$(
      /usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' \
        "$app_bundle/Contents/Info.plist" 2>/dev/null || true
    )"
    if [[ "$bundle_identifier" == "$BUNDLE_IDENTIFIER" \
      || "$bundle_identifier" == "$RELEASE_BUNDLE_IDENTIFIER" ]]; then
      printf '%s\t%s\n' "$pid" "$executable"
    fi
  done < <(/usr/bin/pgrep -U "$(id -u)" -x mimi || true)
}

case "$CANONICAL_APP" in
  /*/*.app) ;;
  *)
    echo "error: MIMI_DEV_APP_PATH must be an absolute .app path." >&2
    exit 2
    ;;
esac

if [[ -L "$CANONICAL_APP" ]]; then
  echo "error: the canonical development app cannot be a symbolic link." >&2
  exit 1
fi

REQUESTED_INSTALL_PARENT="$(dirname "$CANONICAL_APP")"
CANONICAL_APP_NAME="$(basename "$CANONICAL_APP")"
if [[ ! -d "$REQUESTED_INSTALL_PARENT" || ! -w "$REQUESTED_INSTALL_PARENT" ]]; then
  echo "error: development app directory is not writable: $REQUESTED_INSTALL_PARENT" >&2
  echo "Choose a stable writable path with MIMI_DEV_APP_PATH." >&2
  exit 1
fi
INSTALL_PARENT="$(cd "$REQUESTED_INSTALL_PARENT" && pwd -P)"
CANONICAL_APP="$INSTALL_PARENT/$CANONICAL_APP_NAME"
case "$INSTALL_PARENT" in
  *.app | *.app/*)
    echo "error: canonical development app cannot be nested inside another app bundle." >&2
    exit 1
    ;;
esac
if [[ -L "$CANONICAL_APP" ]]; then
  echo "error: the canonical development app cannot be a symbolic link." >&2
  exit 1
fi
if [[ "$CANONICAL_APP" == "$BUILD_APP" ]]; then
  echo "error: canonical development path cannot be the disposable build bundle." >&2
  exit 1
fi

LOCK_FILE="$INSTALL_PARENT/.mimi-dev.lock"
STAGING_OWNER_MARKER=".mimi-dev-install-owner-v1"
if [[ -e "$LOCK_FILE" && ! -f "$LOCK_FILE" ]] || [[ -L "$LOCK_FILE" ]]; then
  echo "error: development lock path is not a regular file: $LOCK_FILE" >&2
  exit 1
fi
exec 9>"$LOCK_FILE"
if ! /usr/bin/lockf -s -t 0 9; then
  echo "error: another mimi development install is running." >&2
  exec 9>&-
  exit 1
fi

STAGING_DIR=""
INSTALL_COMMITTED=0

staging_owned_by_launcher() {
  local staging="$1"
  local marker="$staging/$STAGING_OWNER_MARKER"

  [[ -d "$staging" && ! -L "$staging" && -f "$marker" && ! -L "$marker" ]] \
    && [[ "$(/usr/bin/stat -f '%u' "$staging")" == "$(id -u)" ]] \
    && printf '%s' "$CANONICAL_APP" | /usr/bin/cmp -s - "$marker"
}

cleanup_install() {
  local preserve_staging=0

  if [[ -n "$STAGING_DIR" && -d "$STAGING_DIR" ]]; then
    if ! staging_owned_by_launcher "$STAGING_DIR"; then
      echo "warning: unrecognized staging directory preserved: $STAGING_DIR" >&2
      exec 9>&- || true
      return
    fi
    if [[ -d "$STAGING_DIR/previous.app" && ! -e "$CANONICAL_APP" ]]; then
      if ! mv "$STAGING_DIR/previous.app" "$CANONICAL_APP"; then
        preserve_staging=1
        echo "error: previous app could not be restored; backup preserved at:" >&2
        echo "  $STAGING_DIR/previous.app" >&2
      fi
    elif [[ -d "$STAGING_DIR/previous.app" && "$INSTALL_COMMITTED" != "1" ]]; then
      preserve_staging=1
      echo "warning: uncommitted previous app preserved at:" >&2
      echo "  $STAGING_DIR/previous.app" >&2
    fi
    if [[ "$preserve_staging" == "0" ]]; then
      rm -rf "$STAGING_DIR"
    fi
  fi
  exec 9>&- || true
}
trap cleanup_install EXIT

for stale_staging in "$INSTALL_PARENT"/.mimi-dev-install.*; do
  [[ -d "$stale_staging" ]] || continue
  if ! staging_owned_by_launcher "$stale_staging"; then
    echo "warning: unrelated staging-like directory preserved: $stale_staging" >&2
    continue
  fi
  if [[ -d "$stale_staging/previous.app" ]]; then
    codesign --verify --deep --strict "$stale_staging/previous.app"
    RECOVERY_IDENTIFIER="$(
      /usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' \
        "$stale_staging/previous.app/Contents/Info.plist"
    )"
    if [[ "$RECOVERY_IDENTIFIER" != "$BUNDLE_IDENTIFIER" ]]; then
      echo "error: interrupted install backup has an unexpected bundle identifier." >&2
      exit 1
    fi
    if [[ ! -e "$CANONICAL_APP" ]]; then
      if ! mv "$stale_staging/previous.app" "$CANONICAL_APP"; then
        echo "error: interrupted install backup could not be restored:" >&2
        echo "  $stale_staging/previous.app" >&2
        exit 1
      fi
      echo "Recovered interrupted development install: $CANONICAL_APP"
    elif ! codesign --verify --deep --strict "$CANONICAL_APP" 2>/dev/null; then
      FAILED_APP="$stale_staging/failed.app"
      if ! mv "$CANONICAL_APP" "$FAILED_APP" \
        || ! mv "$stale_staging/previous.app" "$CANONICAL_APP"; then
        echo "error: interrupted install could not be rolled back; files preserved at:" >&2
        echo "  $stale_staging" >&2
        exit 1
      fi
      echo "Recovered previous app after an interrupted invalid install: $CANONICAL_APP"
    fi
  fi
  rm -rf "$stale_staging"
done

if [[ "${MIMI_DEV_RECOVERY_ONLY:-0}" == "1" ]]; then
  exit 0
fi

IDENTITY="$("$SCRIPT_DIR/codesign-identity.sh")"
if [[ "$IDENTITY" == "-" ]]; then
  cat >&2 <<'EOF'
error: development launch requires a stable code-signing identity.

Install the local "mimi Local Development" identity in your login Keychain,
or provide another stable identity explicitly:

  MIMI_CODESIGN_IDENTITY="Apple Development: Your Name" ./scripts/dev-app.sh

Ad-hoc signing is rejected because macOS treats each rebuilt binary as a new
app and may ask for capture permission again.
EOF
  exit 1
fi

cd "$PROJECT_DIR"
export CARGO_HOME="${CARGO_HOME:-$PROJECT_DIR/.cargo-home}"
export npm_config_cache="${npm_config_cache:-$PROJECT_DIR/.npm-cache}"
export MACOSX_DEPLOYMENT_TARGET="13.0"
export MIMI_DEBUG_REVISION="$(git rev-parse HEAD)"
if [[ -n "$(git status --porcelain)" ]]; then
  export MIMI_DEBUG_TREE_STATE="dirty"
else
  export MIMI_DEBUG_TREE_STATE="clean"
fi

npm run build:dev
TAURI_CONFIG="$(<"$DEV_TAURI_CONFIG")" cargo build --profile local-dev \
  --locked \
  --features tauri/custom-protocol,devtools,development-debugger,local-dev-credentials \
  --manifest-path src-tauri/Cargo.toml

rm -rf "$BUILD_APP"
mkdir -p "$BUILD_APP/Contents/MacOS" "$BUILD_APP/Contents/Resources"
cp "$BUILD_DIR/mimi" "$BUILD_APP/Contents/MacOS/mimi"
cp "$PROJECT_DIR/src-tauri/icons/icon.icns" "$BUILD_APP/Contents/Resources/icon.icns"
cp "$PROJECT_DIR/THIRD_PARTY_NOTICES.md" "$BUILD_APP/Contents/Resources/THIRD_PARTY_NOTICES.md"
chmod 755 "$BUILD_APP/Contents/MacOS/mimi"

cat > "$BUILD_APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>mimi dev</string>
  <key>CFBundleDisplayName</key>
  <string>mimi dev</string>
  <key>CFBundleIdentifier</key>
  <string>app.yuxino.mimi.dev</string>
  <key>CFBundleExecutable</key>
  <string>mimi</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleShortVersionString</key>
  <string>${APP_VERSION}-dev</string>
  <key>CFBundleVersion</key>
  <string>1</string>
  <key>CFBundleIconFile</key>
  <string>icon.icns</string>
  <key>LSApplicationCategoryType</key>
  <string>public.app-category.utilities</string>
  <key>LSMinimumSystemVersion</key>
  <string>13.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>NSScreenCaptureUsageDescription</key>
  <string>mimi uses ScreenCaptureKit only to capture system audio for live subtitles.</string>
  <key>NSAudioCaptureUsageDescription</key>
  <string>mimi captures system audio to create live subtitles. Audio is saved locally only when you enable audio recording.</string>
  <key>NSMicrophoneUsageDescription</key>
  <string>mimi uses your microphone for live subtitles only when you select Microphone as the audio input and start subtitles.</string>
</dict>
</plist>
PLIST

plutil -lint "$BUILD_APP/Contents/Info.plist" >/dev/null
codesign --force --deep --timestamp=none \
  --entitlements "$PROJECT_DIR/src-tauri/Entitlements.plist" \
  --sign "$IDENTITY" "$BUILD_APP"
codesign --verify --deep --strict "$BUILD_APP"

NEW_REQUIREMENT="$(designated_requirement "$BUILD_APP")"
if [[ -z "$NEW_REQUIREMENT" || "$NEW_REQUIREMENT" == cdhash\ * ]]; then
  echo "error: refusing to launch an app without a stable code identity." >&2
  exit 1
fi

if [[ -e "$CANONICAL_APP" && ! -d "$CANONICAL_APP" ]]; then
  echo "error: canonical app path exists but is not an app bundle: $CANONICAL_APP" >&2
  exit 1
fi
if [[ -d "$CANONICAL_APP" ]]; then
  EXISTING_REQUIREMENT="$(designated_requirement "$CANONICAL_APP" || true)"
  if [[ "$EXISTING_REQUIREMENT" != "$NEW_REQUIREMENT" ]]; then
    if [[ "${MIMI_ALLOW_IDENTITY_CHANGE:-0}" != "1" ]]; then
      cat >&2 <<EOF
error: $CANONICAL_APP has a different code identity.

Existing: ${EXISTING_REQUIREMENT:-<invalid or unsigned>}
New:      $NEW_REQUIREMENT

Replacing it may require one final macOS authorization. If that is intentional,
run once with MIMI_ALLOW_IDENTITY_CHANGE=1.
EOF
      exit 1
    fi
    echo "warning: replacing a different code identity; macOS may authorize it once more." >&2
  fi
fi

STAGING_DIR="$(mktemp -d "$INSTALL_PARENT/.mimi-dev-install.XXXXXX")"
printf '%s' "$CANONICAL_APP" > "$STAGING_DIR/$STAGING_OWNER_MARKER"
chmod 600 "$STAGING_DIR/$STAGING_OWNER_MARKER"
STAGED_APP="$STAGING_DIR/new.app"
PREVIOUS_APP="$STAGING_DIR/previous.app"
/usr/bin/ditto "$BUILD_APP" "$STAGED_APP"
codesign --verify --deep --strict "$STAGED_APP"
if [[ "$(designated_requirement "$STAGED_APP")" != "$NEW_REQUIREMENT" ]]; then
  echo "error: staged app code identity changed during installation." >&2
  exit 1
fi

terminate_exact_executable "$CANONICAL_APP/Contents/MacOS/mimi"

if [[ -d "$CANONICAL_APP" ]]; then
  mv "$CANONICAL_APP" "$PREVIOUS_APP"
fi
if ! mv "$STAGED_APP" "$CANONICAL_APP"; then
  if [[ -d "$PREVIOUS_APP" ]] && ! mv "$PREVIOUS_APP" "$CANONICAL_APP"; then
    echo "error: install failed and the previous app backup was preserved at:" >&2
    echo "  $PREVIOUS_APP" >&2
    exit 1
  fi
  echo "error: could not install the development app." >&2
  exit 1
fi

if ! codesign --verify --deep --strict "$CANONICAL_APP" \
  || [[ "$(designated_requirement "$CANONICAL_APP")" != "$NEW_REQUIREMENT" ]]; then
  if ! rm -rf "$CANONICAL_APP"; then
    echo "error: invalid installed app could not be removed; backup preserved at:" >&2
    echo "  $PREVIOUS_APP" >&2
    exit 1
  fi
  if [[ -d "$PREVIOUS_APP" ]] && ! mv "$PREVIOUS_APP" "$CANONICAL_APP"; then
    echo "error: previous app could not be restored; backup preserved at:" >&2
    echo "  $PREVIOUS_APP" >&2
    exit 1
  fi
  echo "error: installed app code identity does not match the built app." >&2
  exit 1
fi
INSTALL_COMMITTED=1
rm -rf "$PREVIOUS_APP"
# The signed bundle is a disposable copy of the incremental build binary.
# Keep one discoverable dev application; retaining this copy makes Spotlight
# offer a second launch target with the same bundle identifier.
rm -rf "$BUILD_APP"

echo "Installed stable development app: $CANONICAL_APP"
echo "Designated requirement: $NEW_REQUIREMENT"

if [[ "$SHOULD_LAUNCH" == "1" ]]; then
  CONFLICTING_PROCESSES=()
  while IFS= read -r conflict; do
    [[ -n "$conflict" ]] && CONFLICTING_PROCESSES+=("$conflict")
  done < <(running_conflicting_mimi_processes "$CANONICAL_APP/Contents/MacOS/mimi")
  if [[ ${#CONFLICTING_PROCESSES[@]} -gt 0 ]]; then
    echo "error: another running mimi copy could own the shortcut or request permissions:" >&2
    for conflict in "${CONFLICTING_PROCESSES[@]}"; do
      echo "  $conflict" >&2
    done
    echo "Quit those copies, then run the development launcher again." >&2
    exit 1
  fi

  if [[ "$MODE" == "ui-only" ]]; then
    open -n --env MIMI_UI_TEST=1 --env "MIMI_DEVELOPMENT_EVIDENCE_WORKSPACE=$EVIDENCE_WORKSPACE" "$CANONICAL_APP"
  else
    open -n --env "MIMI_DEVELOPMENT_EVIDENCE_WORKSPACE=$EVIDENCE_WORKSPACE" "$CANONICAL_APP"
  fi

  RUNNING_CANONICAL_PIDS=()
  for _ in {1..50}; do
    RUNNING_CANONICAL_PIDS=()
    while IFS= read -r pid; do
      [[ -n "$pid" ]] && RUNNING_CANONICAL_PIDS+=("$pid")
    done < <(exact_executable_pids "$CANONICAL_APP/Contents/MacOS/mimi")
    [[ ${#RUNNING_CANONICAL_PIDS[@]} -gt 0 ]] && break
    sleep 0.1
  done
  if [[ ${#RUNNING_CANONICAL_PIDS[@]} -ne 1 ]]; then
    terminate_exact_executable "$CANONICAL_APP/Contents/MacOS/mimi"
    echo "error: canonical development app did not start as one exact process." >&2
    exit 1
  fi
  echo "Opened exact development app: $CANONICAL_APP (pid ${RUNNING_CANONICAL_PIDS[0]})"
  echo "Development evidence workspace: ${EVIDENCE_WORKSPACE:-default}"
  if [[ "$MODE" == "ui-only" ]]; then
    echo "UI-only mode does not access credentials, network services, or system audio."
  fi
fi
