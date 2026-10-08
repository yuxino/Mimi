# Bundled local speech helper

The Apple silicon app bundles a pinned MLX Audio Swift executable and Metal
library. End users need no Python, command line tools, server, or account.
Recognition requires macOS 14+. Downloads are owned by the Rust model manager;
the worker only opens the directory passed by the parent and has no downloader.

Building on an Apple silicon Mac requires Swift 6.3+ (Xcode 26.4+), CMake,
and the Xcode Metal Toolchain:

```sh
xcodebuild -downloadComponent MetalToolchain
```

Cargo's build script invokes `build.sh` and pins dependencies through
`Package.resolved`. Generated binaries/resources stay ignored. Intel builds
stage an unavailable helper. Users on desktop platforms can instead choose a
user-owned whisper.cpp CLI or a Mimi-compatible worker; see
[own models and speech services](../../docs/local-programs.md).
The small `hub-resources.patch` changes the pinned tokenizer fallback to search
`Contents/Resources` in a signed Mac app. SwiftPM's default accessor searches
the app root, which macOS signing rejects. Command line builds keep the normal
SwiftPM fallback. Dependency licenses are in `THIRD_PARTY_NOTICES.md`.

For native acceptance, use `./scripts/dev-app.sh`. Explicitly download a model,
click **Use model**, choose original-only subtitles, and verify a system-audio
session with saving/recording and microphone disabled. Verify stop, deletion
protection during recognition, confirmed deletion, progress and cancellation.
Do not use private audio for shared evidence. One-off speech fixtures and
benchmark runners belong outside the repository.

The download manager also has an explicit, ignored acceptance test. Run it only
with a disposable directory and deliberate network/download authorization:

```sh
MIMI_MODEL_ACCEPTANCE_DIR="$TMPDIR/mimi-model-acceptance" \
  cargo test --manifest-path src-tauri/Cargo.toml \
  accept_real_model_download -- --ignored
```

It exercises verified downloads, cancellation/retry and lease protection.
Remove the disposable model directory after acceptance. Ordinary repository
checks never download weights.
