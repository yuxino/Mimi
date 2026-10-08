# Local speech models

## Goal and user flow

Separate cloud services, built-in local recognition and manually configured
connections. On Apple silicon Macs, offer Qwen3-ASR 0.6B (light), Qwen3-ASR
1.7B (standard). Each model has one download
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
Intel and older macOS retain existing services and report this runtime as
unavailable. This change does not add capture sources or change credentials.

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
