# Qwen-MT model-specific streaming text

Alibaba's Qwen-MT API documents cumulative content for Plus and incremental
content for Lite/Flash. The shared streaming client currently appends every
content event, which repeats text when its public streaming method receives
Plus snapshots. Production subtitles and connection probes already select
non-streaming Plus; preserve that policy and its existing UI description.

Use one bounded model-aware accumulator in `shared/mimi-runtime`, called by the
existing SSE decoder. Plus replaces its previous text, including a shorter
correction; Lite/Flash append. Empty content preserves the current text, and
duplicate Plus snapshots do not emit duplicate partial callbacks. Enforce the
existing 64 KiB text limit before mutation. Keep response/line bounds, terminal
validation, cancellation and HTTP tail draining unchanged.

Shared fixtures exercise cumulative snapshots, corrections, duplicate/empty
events and incremental Unicode text. Direct Rust and actual Android JVM/JNI
tests run the same accumulator; local HTTP/SSE tests retain transport coverage.
No endpoint, authentication, prompt, default model or scheduling changes.

Protocol source: [Qwen-MT API](https://help.aliyun.com/zh/model-studio/qwen-mt-api).
List screenshots belong in PR comment attachments, outside the source tree.
