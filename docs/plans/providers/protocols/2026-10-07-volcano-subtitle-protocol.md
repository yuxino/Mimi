# Volcano subtitle protocol and disconnect investigation

Issue [#184](https://github.com/yuxino/Mimi/issues/184) reports disconnection
with Japanese while English works. The official AST2.0 language table specifies
`ja`; keep that code and the existing source-dependent direction checks.

The [official API and downloadable protobuf schema](https://docs.volcengine.com/docs/DoubaoVoice/SimultaneousInterpretation20APIAccessDocumentation?lang=zh)
define `TranslateResponse.text` as proto3 `string text = 4`, without explicit
presence. [Proto3 defaults](https://protobuf.dev/programming-guides/proto3/#specifying-field-cardinality)
permit omission of an empty string. Both handwritten adapters incorrectly require
this field on draft/final events; desktop raises `MissingText` and terminates its
receive loop, while Android raises `invalid_server_event` and shuts down.

Decode omitted text as the same empty string as an explicitly encoded empty field.
Preserve event type and finality so existing bounded subtitle handling decides
whether there is content to show. Desktop's final committer already rejects empty
finals; do not fabricate a pair, reuse draft text or change shared subtitle policy.
Continue rejecting invalid UTF-8, wrong wire types, truncated data and repeated
explicit fields. Credentials, capture, transport and language mappings stay intact.

Two approved direct-cloud runs of the same 5.649-second public synthetic Japanese
sample each produced three source and three translation finals, successful status
codes and normal SessionFinished. Neither run reproduced the reported disconnect
or omitted text on subtitle draft/final events. The second run compared fragments
in memory: concatenating all 651/654 fragments exactly matched all six final texts.
Only counts, status, timing and comparison booleans were retained. Four further
public synthetic samples (English → Chinese, Chinese → English, Korean → Chinese,
French → Chinese) each produced three complete pairs and normal SessionFinished;
their setup times were 235–258 ms. All six fragment/final comparisons per run
matched. These are short direct-service probes, not native end-to-end or semantic
translation acceptance.

This establishes a separate adapter defect: draft events are incremental, while
Mimi's common subtitle reducer expects a complete replaceable preview. Accumulate
each side separately in the native provider adapter, resetting on its start/final,
local clear and session reset. Never infer a durable final from draft fragments.
Both adapters bound each accumulated preview to the shared 64 KiB UTF-8 subtitle
field limit; Android also retains its existing TurnText character bound. Overflow
fails safely before appending. This does not change the shared subtitle or
finalization policy.

Shared synthetic wire fixtures cover all four subtitle events with omitted,
explicit-empty and nonempty Unicode text, plus malformed negative cases. Both Rust
and Kotlin run those fixtures; a local desktop WebSocket regression checks that an
empty event does not prevent the next complete subtitle pair or normal shutdown.
Shared incremental sequences cover Unicode fragments, empty deltas and independent
sentence resets. Existing catalog fixtures exercise all 81 accepted setups across
20 ordinary languages, two input-only dialects and explicit bilingual reversal,
and reject unsupported directions on both platforms. Languages beyond the five
sampled ones are checked against the official language table and wire contracts,
not individually uploaded.

These are demonstrated protocol defects, not yet proof of the cause of the user's
original Japanese failure. Keep #184 open until a comparable cloud/native run or
safe diagnostic evidence identifies that failure. Cloud sample uploads require
the user's explicit approval; local fixture results do not establish live-provider
or real-device acceptance.
