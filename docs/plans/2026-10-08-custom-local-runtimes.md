# Custom local runtimes

**Goal:** Keep built-in model download/use/delete simple while letting users connect their own service or run a local speech program on desktop platforms.

**Architecture:** Model installation, inference and subtitle/translation processing remain independent. Add a keyless local-program profile alongside built-in MLX models, with explicit executable/model pickers. The first adapters are whisper.cpp CLI and the existing bounded Mimi JSON-lines worker protocol. Files selected by the user remain user-owned and are never deleted by model management. Runtime startup happens only for an explicit check or session, never during discovery or preference loading.

**Tech stack:** Existing Rust/Tokio process and tempfile support, Tauri native file dialogs, React settings primitives and seven UI locales. No new runtime dependency or automatically installed interpreter.

## Decisions and trade-offs

- Reuse the current private stdio client for user-provided compatible workers. Keep process spawning separate from the fixed model catalog, so future native engines do not need another subtitle implementation.
- Adapt whisper.cpp CLI explicitly: write one bounded 16 kHz PCM16 turn to a private temporary WAV, execute without a shell, collect bounded output, and remove temporary audio on completion/cancellation. CLI recognition is sentence-based and reloads the model per turn; users needing persistent streaming can use a compatible worker or realtime service. Do not describe CLI timing as streaming-model performance.
- Start the custom entry with two clear choices, connect a service or run a program. Preserve the existing OpenAI Realtime/DashScope protocol adapters; protocol details stay in help. Loopback services may omit authentication, while remote endpoints still require a key.
- Local programs require an absolute executable path, a supported adapter and valid model files. Arguments are separate values, never an interpolated shell command. Bound output/audio/queue lengths and inference time; stale generations/revisions cannot publish results. Stop cancels and reaps owned children. Do not log paths, arguments, speech, or raw process errors.
- Windows and Linux reuse these Rust adapters; built-in MLX remains Apple-silicon-only. Native platform acceptance must be reported separately from compilation/CI.

## Implementation and acceptance

1. Add validated local-program metadata, keyless configuration resolution and focused tests for malformed paths/arguments, language capabilities and secret-store isolation.
2. Extract reusable worker launch configuration and add bounded whisper.cpp inference, lifecycle/cancellation and protocol tests.
3. Add program/model native pickers, explicit save/retry and independent recognition checks. Reuse translation configuration, transient toasts and destructive configuration confirmation. Guard edits while a session is active.
4. Make the custom entry action-oriented; support optional loopback authentication without changing remote requirements or destination-change credential isolation.
5. Run focused regressions and `./scripts/check.sh`; build `/Applications/mimi-dev.app`, inspect empty/error/ready/paused/long-content flows and validate a real synthetic system-audio session. Keep one-off fixtures outside Git, restore user settings, update the existing PR without merging, and remove disposable fixtures/build products.

whisper.cpp CLI contract: https://github.com/ggml-org/whisper.cpp/blob/master/examples/cli/cli.cpp
