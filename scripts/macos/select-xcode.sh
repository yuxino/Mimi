#!/usr/bin/env bash
set -euo pipefail

# Select an installed Xcode >= 26 for Apple Speech compilation on macOS runners.
# Prefer the active xcode-select default when its major version is already >= 26;
# otherwise pick the highest installed stable Xcode 26+ bundle under /Applications.

MIN_MAJOR=26
APPLICATIONS_DIR="${MIMI_XCODE_APPLICATIONS_DIR:-/Applications}"

xcode_version_for_dev_dir() {
  local dev_dir="$1"
  local app_dir=""
  local plist=""
  local version=""

  [[ -d "$dev_dir" ]] || return 1

  if [[ "$dev_dir" == */Contents/Developer ]]; then
    app_dir="${dev_dir%/Contents/Developer}"
    plist="$app_dir/Contents/Info.plist"
    if [[ -f "$plist" && -x /usr/libexec/PlistBuddy ]]; then
      version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$plist" 2>/dev/null || true)"
      if [[ -n "$version" ]]; then
        printf '%s\n' "$version"
        return 0
      fi
    fi
  fi

  version="$(DEVELOPER_DIR="$dev_dir" xcodebuild -version 2>/dev/null | awk 'NR == 1 && $1 == "Xcode" { print $2 }' || true)"
  if [[ -n "$version" ]]; then
    printf '%s\n' "$version"
    return 0
  fi

  return 1
}

version_meets_minimum() {
  local version="$1"
  local major="${version%%.*}"
  [[ "$major" =~ ^[0-9]+$ ]] && (( major >= MIN_MAJOR ))
}

selected_dev=""
selected_version=""

default_dev="$(xcode-select -p 2>/dev/null || true)"
if [[ -n "$default_dev" ]] && default_version="$(xcode_version_for_dev_dir "$default_dev")"; then
  if version_meets_minimum "$default_version"; then
    selected_dev="$default_dev"
    selected_version="$default_version"
  fi
fi

if [[ -z "$selected_dev" ]]; then
  candidates=()
  shopt -s nullglob
  for app in "$APPLICATIONS_DIR"/Xcode*.app; do
    dev_dir="$app/Contents/Developer"
    [[ -d "$dev_dir" ]] || continue
    version="$(xcode_version_for_dev_dir "$dev_dir" || true)"
    if [[ -n "$version" ]] && version_meets_minimum "$version"; then
      stability=1
      base_name="$(basename "$app")"
      if [[ "$base_name" == *[Bb]eta* || "$base_name" == *RC* || "$base_name" == *Release_Candidate* ]]; then
        stability=0
      fi
      candidates+=("${stability}"$'\t'"${version}"$'\t'"${dev_dir}")
    fi
  done
  shopt -u nullglob

  if (( ${#candidates[@]} > 0 )); then
    best="$(printf '%s\n' "${candidates[@]}" | sort -t $'\t' -k1,1n -k2,2V | tail -n 1)"
    selected_version="$(printf '%s' "$best" | cut -f2)"
    selected_dev="$(printf '%s' "$best" | cut -f3)"
  fi
fi

if [[ -z "$selected_dev" || ! -d "$selected_dev" ]]; then
  echo "No Xcode >= ${MIN_MAJOR} installation found (checked xcode-select -p and ${APPLICATIONS_DIR}/Xcode*.app)." >&2
  exit 1
fi

echo "Selected Xcode ${selected_version} at ${selected_dev}"
if [[ -n "${GITHUB_ENV:-}" ]]; then
  echo "DEVELOPER_DIR=${selected_dev}" >> "$GITHUB_ENV"
fi
