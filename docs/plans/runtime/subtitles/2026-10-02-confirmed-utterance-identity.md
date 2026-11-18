# Confirmed utterance identity through bounded draft transport

## Problem

The provider transport keeps recognition and translation drafts in latest-value
slots, while final events use an ordered reliable queue. A final boundary can
therefore supersede every draft from a new sentence. Using an intervening draft
or text equality to distinguish confirmations can drop a genuinely repeated
lyric twice: before HQ receives its recognition result, and before the subtitle
reducer receives its translated pair.

## Accepted identity

Audio3's official [streaming server-event specification](https://help.aliyun.com/zh/model-studio/qwen-audio-asr-streaming-server-events)
provides a positive integer `payload.output.sentence.sentence_id` for recognized
sentences. IDs increase within a recognizer task; heartbeats use zero. Preserve
the actual positive ID in a local `SourceUtteranceFinal` event on the existing
reliable lane. Missing, zero, negative, noninteger, and string IDs keep the legacy
`SourceFinal` fallback. Do not synthesize identity from timestamps or captions.
No outbound request, protocol prompt, or model configuration changes.

HQ retains one source-ID watermark per recognizer task. Replayed or older IDs
are rejected before canceling preview work or resetting its next draft. A new
ID remains a new sentence even when its text exactly matches the previous one.
Connect/reset drops this task-local watermark.

The accepted HQ utterance revision travels in `SubtitleConfirmedPair` for both
original-text and serial translated finals. It becomes `SubtitleEvent::ConfirmedPair`
in the core controller. The reducer retains only the last accepted revision,
using wrapping serial ordering, and commits each new identity even if its text
matches the preceding confirmed pair. Old or repeated identities cannot rewind
the display, clear pending work, or enter history/archive again. Generation reset
clears this watermark; explicit display Clear retains it so a replay cannot
restore text the user cleared. Legacy final-pair behavior remains available to
other providers.

Distinct confirmations receive distinct monotonic confirmation timestamps.
Private session-file append checks compare that timestamp as well as the pair's
text, preserving repeated sentences when recording/history was explicitly
enabled. Numeric transport identities remain internal; no new caption, key,
endpoint, path, or identity field enters diagnostics or IPC snapshots.

## Verification

Focused fixtures decode real-shaped synthetic Audio3 frames and pass them through
both bounded transports with no delivered draft between identical confirmed
sentences. Different source IDs must commit twice; replaying the same ID must
commit once. Other fixtures cover rejection before canceling a new draft,
recognizer-task reset, wrapped local revisions, Clear, bounded history, archive
counts, stopping-tail acceptance, and private-file append eligibility.

Native provider-session acceptance remains separate from these deterministic
tests. Responses without a valid sentence ID intentionally retain the legacy
fallback's ambiguity.

## Caption byte bounds

Every retained raw caption or pair field shares a 64KiB UTF-8 byte limit. The
provider event transport rejects an oversized field before retaining it in the
latest-value or reliable lanes, publishing only a fixed content-free error.
Audio3 decoding, the cumulative draft candidate, HQ preview/final admission, and
core session/reducer admission also enforce the bound. Reject the entire field;
do not truncate a sentence or pair it with an unrelated previous translation.
Rejected confirmations do not consume identity, clear pending/recovery state, or
overwrite the last complete subtitle pair.

Audio3 recognition-only WebSockets explicitly cap incoming messages and frames
at 1MiB, and its decoder rejects larger JSON before parsing. Other WebSocket
services retain their existing receive limits because some carry output audio;
this change does not claim a universal 1MiB network limit. Completed captions
from every service must still pass the shared transport and core text guards.
