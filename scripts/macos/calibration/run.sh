#!/bin/bash
set -euo pipefail

mode=run
if [[ $# -gt 0 && $1 == --compile-only ]]; then mode=compile; shift; fi
if [[ $# != 1 ]]; then echo "Usage: bash run.sh [--compile-only] OUTPUT_DIR" >&2; exit 2; fi
if [[ $mode == run && ${RUNNER_ENVIRONMENT:-} != github-hosted ]]; then
    echo "The native input calibration runs only on an isolated GitHub-hosted desktop." >&2
    exit 1
fi
suite=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$1"
output=$(cd "$1" && pwd)
build=$(mktemp -d "$output/.appkit-build.XXXXXX")
fixture=
cleanup() {
    if [[ -n $fixture ]]; then
        kill -TERM "$fixture" 2>/dev/null || true
        wait "$fixture" 2>/dev/null || true
    fi
    rm -rf "$build"
}
trap cleanup EXIT
xcrun swiftc -parse-as-library -warnings-as-errors -target arm64-apple-macos15.0 -module-cache-path "$build/cache" \
    "$suite/DragCalibration.swift" -o "$build/drag-calibration"
if [[ $mode == compile ]]; then echo "Compiled and linked the arm64 macOS 15 calibration."; exit 0; fi

"$build/drag-calibration" window "$build/ready" >"$output/appkit-window.log" 2>&1 &
fixture=$!
for (( i=0; i<150; i++ )); do
    if [[ -s $build/ready ]] || ! kill -0 "$fixture" 2>/dev/null; then break; fi
    sleep 0.1
done
if [[ ! -s $build/ready ]]; then cat "$output/appkit-window.log" >&2; exit 1; fi
status=0
"$build/drag-calibration" driver "$fixture" "$(cat "$build/ready")" >"$output/appkit-driver.log" 2>&1 || status=$?
cat "$output/appkit-window.log" "$output/appkit-driver.log"
if [[ $status != 0 ]]; then exit "$status"; fi
enters=$(grep -c 'app performDrag-enter' "$output/appkit-window.log" || true)
if [[ $enters != 2 ]]; then echo "Expected both body presses to enter the fixture view, saw $enters." >&2; exit 1; fi
