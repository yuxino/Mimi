# Real sentence ownership for Audio3 drafts and completed pairs

## Accepted scope

Audio3's [official server-event specification](https://help.aliyun.com/zh/model-studio/qwen-audio-asr-streaming-server-events)
assigns positive increasing `sentence_id` values to both intermediate and final
recognition. A new sentence may start with an empty `sentence_begin` result.
Heartbeats use zero and are excluded. Preserve these existing wire identities
internally; do not invent identities from text, divide lyrics by punctuation, or
deduplicate different IDs because their words match.

The previous draft conversion discarded the ID. A late draft from a confirmed
sentence could recreate its preview, and a previous sentence's final could reset
a newer candidate. The reducer also could replace an already displayed newer
pair with the older final. Three synthetic fixtures reproduce the draft/begin
failures before the change. The live screenshots alone cannot prove which
upstream event order occurred.

## Bounded ownership

Identified source drafts use the existing latest-value lane. HQ keeps one current
source ID and one confirmed-source watermark. Drafts at/below that watermark or
older than the current source are ignored. A different real ID cancels only
replaceable work, invalidates its callbacks, and clears only the obsolete preview.
Same-ID revisions retain the completed pair and may finish their in-flight HTTP
request. Empty real begins carry the boundary without translating empty text.

A previous final remains on the ordered durable lane. If a newer identified
candidate already exists, keep it for after the final lane drains. The transport
does not let an older sentence final supersede that newer identified draft.
Confirmation IDs use a separate local monotonic counter; they are not borrowed
from the newest draft revision.

Preview and confirmed events retain an optional actual source ID internally.
The reducer uses bounded private source/preview watermarks: an older final appends
history but does not overwrite a newer raw source or completed preview. The
two owners are checked independently. If a complete preview belongs to A7,
raw B8 arrives (even with empty text), and A7 later confirms, retain raw B8 but
clear the already-confirmed A7 preview. A newer raw source must not keep an
older preview visible beside its own durable history. A completed B8 preview
still survives an older A7 confirmation. This decision uses actual IDs, not
text-prefix similarity, so repeated lyrics with different IDs remain distinct.
The completed preview has an optional opaque `utteranceId`, also used by identified
raw sources in the existing source field. Its real server ID and a private layout
epoch keep a newer preview's reading key stable when an older final appends to
history. Clear, reducer creation and connection reset refresh that epoch; no text
is used as identity. Missing IDs omit the preview field and keep old UI behavior.
Reconnect resets task-local source
watermarks; Clear retains confirmation floors and the existing content revision
guards. Lifecycle/errors still cross Clear. Old providers and missing-ID events
keep their existing compatibility behavior.

Request starts, 429 suppression, shared cooldown, final FIFO and queue limits
remain owned by the existing pipeline. A sentence change forgets only its preview
candidate fingerprint, never consumed request budget. No connection is restarted
and no extra request is introduced to discover identity.

## Verification and limits

Focused fixtures cover late same/older-ID drafts, empty new begins, same-ID
revisions, newer drafts delivered before older finals through the bounded
transport, preservation of a newer completed pair during an older confirmation,
independent cleanup of a confirmed preview while the next raw sentence has
already begun (empty and nonempty B8 cases), distinct-ID repeated lyrics,
Clear/reconnect ownership and late callbacks.

The live-song duplicate was observable after pausing, but the content-free log
showed the server final completing before the pause request. The pause path
disconnects instead of generating a fallback final. The pure event regression
reproduces the owner condition; existing logs do not expose sentence IDs and
cannot establish the exact private-audio event sequence. This correction is
not described as a pause-fallback repair or a recognition-quality improvement.

This preserves actual service sentence boundaries. It does not repair ASR text
that overlaps across genuinely different IDs, guess missing words, align two
languages sentence by sentence, or prove live song recognition quality. Official
word timestamps exist, but are not currently mapped to the UI clear clock.
Automated fixtures and native provider acceptance are separate evidence.
