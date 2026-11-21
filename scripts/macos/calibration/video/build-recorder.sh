#!/usr/bin/env bash
set -euo pipefail
[[ $(uname -s) == Darwin ]]
tools=$(cd "$(dirname "$0")" && pwd)
xcrun swiftc -parse-as-library -swift-version 5 -target arm64-apple-macosx15.0 \
  "$tools/WindowRecorder.swift" -o "$tools/recorder"
