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
stage an unavailable helper; other platforms keep their existing providers.
The small `hub-resources.patch` changes the pinned tokenizer fallback to search
`Contents/Resources` in a signed Mac app. SwiftPM's default accessor searches
the app root, which macOS signing rejects. Command line builds keep the normal
SwiftPM fallback. Dependency licenses are in `THIRD_PARTY_NOTICES.md`.

After explicitly downloading models in the development app, reproduce the
synthetic English/Chinese/Japanese checks without capture or network:

```sh
python3 scripts/check-local-speech.py \
  --models "$HOME/Library/Application Support/app.yuxino.mimi.dev/local-models" \
  --model qwen-small
# Repeat with --model qwen-standard.
```

This checks the signed app's bundled helper, repeated turns, draft/final IDs,
clear, ping with stdin kept open, and finish. It prints timing/count metadata
only and removes generated speech files. It does not establish live capture
latency or accuracy on game dialogue/music.
