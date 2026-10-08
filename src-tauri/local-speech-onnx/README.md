# Bundled ONNX speech worker

Mimi builds a native C++17 worker against **sherpa-onnx 1.13.8**. Its fixed
release archives, byte lengths and SHA-256 digests are in
`runtime-assets.json`. The release's Linux x64 ONNX Runtime reports **1.28.2**.
The C API header and nlohmann/json **3.12.0** header are separately hashed.
Python 3 and CMake are build tools only: neither is installed or invoked by
the application. No model/export repository code is executed.

`build.py TARGET` verifies dependencies, builds the worker, runs native parser
tests and stages the worker plus its libraries in `../binaries/local-speech-onnx`.
Tauri packages that directory as an application resource. The signed Mac dev
launcher copies the same directory. Unsupported targets get no worker and keep
the custom local-program entry. Cross-architecture compilation does not count
as native test acceptance.

Initial managed models are SenseVoiceSmall int8 (2024-07-17) and Qwen3-ASR
0.6B int8 (2026-03-25). Model manifests live in `src/core/local_speech_catalog.json`;
only explicit model-library downloads retrieve weights. The existing MLX model
IDs/directories remain unchanged. CPU inference is the shared baseline.

The worker consumes the [Mimi stdio protocol](../../docs/local-programs.md).
It loads one recognizer for the process lifetime, creates/releases one stream
per turn, accepts at most eight seconds of PCM16/16 kHz audio, and bounds input
lines, result strings and output lines. It never captures audio, writes audio
or transcripts to disk, uses credentials, downloads anything, or opens a server.
The parent owns queue limits, readiness/inference health budgets, generation and
revision filtering, pause/clear cancellation and child reaping. Stderr is discarded;
exceptions produce only the protocol's sanitized `error` event.

SenseVoice supports auto/Chinese/English/Japanese/Korean/Cantonese and returns
its detected language. Qwen accepts explicit language hints through
`SherpaOnnxOfflineStreamSetOption`, using upstream full language names. In this
pinned release, Qwen strips its generated language prefix without filling
`result.lang`: auto mode therefore omits language metadata; it never fabricates
a detected language. Explicit mode returns the configured language. Translators
that require a known source language should use explicit mode.

Ordinary worker/Rust tests do not download weights. The ignored Rust
`local_models::tests::accept_real_model_download` requires an explicit
`MIMI_MODEL_ACCEPTANCE_DIR` and model ID for maintainer download acceptance.
Native desktop capture, visible subtitles, signing and sustained latency are
separate from worker loading/inference and parser tests.
