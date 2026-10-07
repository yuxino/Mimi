#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" || "$(uname -m)" != "arm64" ]]; then
  echo "==> skipping native Apple Translation tests (requires macOS arm64)"
  exit 0
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$PROJECT_DIR/src-tauri/target}"
TEST_DIR="$TARGET_DIR/apple-translation-tests"
mkdir -p "$TEST_DIR"

echo "==> native Apple Translation lifetime tests (no models or downloads)"
xcrun swiftc -parse-as-library -swift-version 6 -warnings-as-errors \
  -D MIMI_TRANSLATION_TESTS -target arm64-apple-macosx13.0 \
  -sdk "$(xcrun --sdk macosx --show-sdk-path)" \
  -module-cache-path "$TEST_DIR/swift-module-cache" \
  "$PROJECT_DIR/src-tauri/apple-translation/Bridge.swift" \
  "$PROJECT_DIR/src-tauri/apple-translation/tests/BridgeTests.swift" \
  -o "$TEST_DIR/bridge-tests"
"$TEST_DIR/bridge-tests"

# Older supported macOS versions must be able to load Mimi without either
# Translation framework. Keep this assertion alongside the runtime tests.
otool -l "$TEST_DIR/bridge-tests" | python3 -c '
import re, sys
commands = re.split(r"Load command \d+", sys.stdin.read())
for name in ("Translation.framework", "_Translation_SwiftUI.framework"):
    matching = [command for command in commands if "/" + name + "/" in command]
    assert len(matching) == 1 and "LC_LOAD_WEAK_DYLIB" in matching[0], name + " must be weak-linked"
print("native_translation_weak_links=passed")
'
