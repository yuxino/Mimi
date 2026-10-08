# Local speech models

## Goal and user flow

Separate cloud services, built-in local recognition and manually configured
connections. Offer a shared CPU/ONNX baseline with SenseVoiceSmall int8 and
Qwen3-ASR 0.6B int8. Retain Qwen3-ASR 0.6B/1.7B MLX as the Apple silicon
optimization, preserving existing model IDs, directories and configurations.
Each model has one download
action with size, progress, cancellation and retry; installed models can be
used or explicitly deleted. No Python, server address, account or API key is
required for recognition. Translation remains an independent existing route;
local recognition must not imply that a cloud translator runs offline.

## Architecture and alternatives

Use a pinned MLX Audio Swift helper bundled with the Mac app. Rust owns the
allowlisted, revision-pinned model catalog, verified downloads, private storage,
installation state and helper lifetime. Load only an explicitly installed local
directory; discovery, application startup and session start never download.
Keep helper transport private to the parent process over bounded standard I/O.
Reuse the existing standalone recognition and independent translation pipeline.

A Python environment would add interpreter/package setup and recovery to the
user flow. Requiring a separately running custom server would leave the main
setup burden unsolved. The Swift helper adds build dependencies and Mac-only
support, but keeps installation and recognition self-contained. Windows, Linux,
Intel and older macOS retain existing services and report this MLX runtime as
unavailable. This change does not add capture sources or change credentials.

On unsupported devices, the library names the built-in Qwen restriction and
hides missing, unusable catalog rows. The existing user-owned model entry
remains directly actionable; installed models and ongoing operations stay
visible for cleanup. Privacy copy uses “this computer” on every platform.
The original MLX implementation alone does not provide Windows support; the
shared ONNX extension below has independent per-model availability.

## Shared ONNX extension (2026-10-08)

- Bundle a native C++17 stdio worker with sherpa-onnx 1.13.8; the verified Linux
  x64 release reports ONNX Runtime 1.28.2. Fixed release asset hashes and lengths,
  plus separately hashed C API/nlohmann-json 3.12.0 headers, are checked at build
  time. Python/CMake are maintainer build dependencies, never user requirements.
  Runtime/model discovery, startup and recognition perform no downloads.
- CPU baseline targets: Linux x64, Windows x64/ARM64 and macOS Intel/Apple silicon.
  Unbundled targets keep existing providers and user-owned local programs. App
  availability requires its worker and both native libraries, independently of
  MLX's macOS 14/Apple silicon gate. Compile/asset support is not native acceptance.
- Use the maintainer's SenseVoiceSmall int8 2024-07-17 export as the initial
  five-language baseline and Qwen's 0.6B int8 2026-03-25 format. Each fixed HF
  revision/file has an expected length and SHA-256; large-file hashes were
  matched to LFS object IDs and verified after actual download. Tokenizer nested
  paths are allowlisted and reject traversal/symlinked parents. Downloads retain
  private staging, cancel/failure cleanup and atomic activation. Windows staging
  uses the existing owner-only DACL utility without credential access.
- Load one recognizer for the worker lifetime; release per-turn streams/results.
  Reuse the existing bounded queue, turn gate, leases, cancellation and revision
  filtering. Preserve whisper.cpp and user-owned Mimi workers unchanged. Windows
  uses the MT runtime assets and static worker CRT, avoiding a user VC runtime
  install; wide command-line paths are converted to UTF-8.
- SenseVoice returns detected language. Qwen's language hint is a stream option;
  the pinned implementation drops its generated language scaffold without
  returning `result.lang`. Explicit mode returns the configured language; auto
  mode omits unknown language metadata. Never infer it from arbitrary transcript
  text. Text translation stays independent with no silent cloud fallback.
- Reuse the existing library/actions and all seven locales. Per-model availability
  governs downloads and use in both the library and profile list; incompatible
  installed models remain deletable. Do not recommend all new models as faster
  or more accurate merely because upstream provides them.

Dependency digests must be computed from raw upstream download bytes, never a
text/API-rendered copy or newline-normalized buffer. A clean dependency fetch
is required for acceptance; a previously seeded build cache is insufficient.
Build-time download regressions verify failed integrity checks do not publish
or leave staging files.

## Hy-MT2 follow-up boundary

Hy-MT2 is text translation, never a speech model. The official 1.8B GGUF repo
revision `a0c709d9fac510f2c807aa3af52872340dc37a4a` contains Q4_K_M/Q6_K/Q8_0
and uses Apache-2.0. A concrete first follow-up is a persistent, keyless,
bounded text worker using conventional Q4_K_M plus a pinned llama.cpp build,
with the official translation prompt/chat template and independent readiness,
stop/clear/revision handling. The current upstream stable candidate is llama.cpp
v0.6.0 (`8345f333951c661d166b00e6f9362e553768f292`); it includes
`hunyuan-dense`, but no Hy-MT2 model was run here, so this is not a verified
runtime/model pairing and no usable translation option is exposed.

The 1.25-bit STQ1_0 route needs llama.cpp PR #22836, still open/unmerged on
2026-10-08 (head `1e411d8f5a1e23525fa3265dfb4bd76265465397`). Its changes
provide CPU generic handling plus ARM NEON optimization; they do not establish
Metal/CUDA support or native Windows/Intel performance. Do not assume ordinary
released llama.cpp binaries can load STQ. Any low-bit variant needs its actual
GGUF tensor types, pinned kernel/model hashes and platform smoke tests verified
separately. Hy-ASR has no confirmed downloadable local weights in this task and
must not appear as an offline built-in. None of these investigations block ASR.

Sources: [sherpa Qwen export](https://k2-fsa.github.io/sherpa/onnx/qwen3-asr/export.html),
[SenseVoice](https://k2-fsa.github.io/sherpa/onnx/sense-voice/index.html),
[Hy-MT2](https://github.com/Tencent-Hunyuan/Hy-MT2),
[STQ kernel PR](https://github.com/ggml-org/llama.cpp/pull/22836).

## Resource and lifecycle rules

- Download only fixed HTTPS repositories, revisions and files. Verify SHA-256
  and expected lengths before atomically publishing an installation. Never
  execute downloaded model content or resolve arbitrary paths from IPC.
- Keep progress in the model library, errors by their action, and transient
  success in the existing toast. Do not make progress block unrelated settings.
- Cancel or failed downloads remove their partial files. Deletion removes only
  Mimi-managed files, requires confirmation, and is rejected while leased by a
  recognition session. Saved profiles remain recoverable by downloading again.
- Bound audio, pending inference, result text and subprocess messages. Stop,
  pause, clear and reconnection retain generation/revision protection. Do not
  log recognized text, audio, credentials or raw model/runtime exceptions.
- Apple-managed Speech/Translation assets retain their native management flow.
  Do not claim ownership of or delete third-party model caches.

## Verification and limits

- The canonical `./scripts/check.sh` covers formatting, strict Clippy, Rust
  tests, frontend typecheck/lint/tests/build and the diff checks.
- Explicit maintainer download acceptance verified both real pinned Qwen
  checkpoints, cancellation/retry, cleanup and refusal to delete a leased
  model. Ordinary tests never download weights.
- Temporary synthetic-speech fixtures tested the signed canonical development app's
  helper with synthetic English, Chinese and Japanese speech, repeated turns,
  clear, ping while stdin stays open, and finish. These fixtures and benchmark runners stay outside the repository;
  inference timing evidence belongs in the PR verification record.
- The canonical `/Applications/mimi-dev.app` completed a real system-audio
  session with automatic recognition of all three fixture languages, original
  subtitles, no microphone and no saving/recording. Startup was about 1.4 s.
  Stop released the worker and model lease. Native UI checks covered one-click
  use, downloaded/missing states, disabled deletion during use, confirmation,
  real deletion, download progress and cancellation with partial-file cleanup.
- Download IPC runs asynchronously so its spawned jobs own a Tokio context.
  The helper uses POSIX pipe reads so short commit/clear/ping messages do not
  wait for a full read buffer or EOF.
- The signed `.app` and `.dmg` are verified separately from the installed dev
  app. SwiftPM bundles live in `Contents/Resources`, with a pinned resource
  accessor patch. The formal installed application is never replaced.
- Turn segmentation currently reuses the bounded RMS gate: 600 ms preroll,
  600 ms silence and at most 8 s per turn, with at most 2 s of queued PCM.
  This is not a neural VAD or a music filter. Game dialogue with background
  music, full translated live sessions, Intel, Windows and Linux native
  acceptance remain outside this Mac acceptance.

Voxtral Mini 4B Realtime was evaluated but is excluded from the initial catalog:
a 4.45-second English fixture still took about 10.6 seconds on the M5 with the
pinned runtime after warmup and batched incremental inference. Keep the initial
choices useful for live subtitles rather than advertising upstream latency.

Official model cards: [Qwen3-ASR](https://huggingface.co/Qwen/Qwen3-ASR-1.7B),
[Voxtral](https://huggingface.co/mistralai/Voxtral-Mini-4B-Realtime-2602).
Runtime: [MLX Audio Swift](https://github.com/Blaizzy/mlx-audio-swift).
Quantized community checkpoints must be verified independently; upstream
benchmark latency does not establish Mimi's end-to-end performance.
