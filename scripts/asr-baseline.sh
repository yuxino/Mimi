#!/usr/bin/env bash
# Explicit, paid, test-only ASR baseline. No app launch and no Keychain fallback.
set -euo pipefail
umask 077
if [[ $# -lt 1 || $# -gt 2 || "$1" != /* ]]; then
  echo 'Usage: scripts/asr-baseline.sh /absolute/public-fixtures/manifest.json [/private/tmp/mimi-debug-benchmark/output]' >&2
  exit 2
fi
if [[ "$(uname -s)" != Darwin ]]; then
  echo 'This baseline requires the macOS saved development profile.' >&2
  exit 2
fi
mimi_bench_root="$(cd "$(dirname "$0")/.." && pwd)"
MIMI_ASR_BENCH_MANIFEST="$1" \
MIMI_ASR_BENCH_ARM=baseline \
MIMI_ASR_BENCH_DENOISE=bypass \
MIMI_ASR_BENCH_PRIVATE_OUTPUT_DIR="${2:-/private/tmp/mimi-debug-benchmark/asr-baseline}" \
cargo test --manifest-path "$mimi_bench_root/src-tauri/Cargo.toml" \
  --features development-debugger --lib \
  audio3_benchmark::manual_same_pcm_asr_comparison -- --ignored --exact --nocapture
