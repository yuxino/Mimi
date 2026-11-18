# Tencent realtime subtitle stability

The main branch already uses Tencent's current
[realtime speech translation WebSocket API](https://cloud.tencent.com/document/api/1093/127565),
with `hunyuan-translation-lite`, PCM16 mono at 16 kHz, 6,400-byte frames,
and TTS disabled by the API's default. It does not use the retiring TSI API.
The existing AppID, SecretID and SecretKey fields, private credential storage,
service identity, source-dependent language catalog and translation preferences
are retained. No provider account or credential values are needed for this repair.

## Admission and presentation

Decode the documented `sentence_id`, `start_time`, `end_time` and
`sentence_end` together with each source/translation result. Require bounded,
nonempty sentence IDs and nonnegative integer audio-clock times with end at or
after start. Retain at most 64 sentence metadata records, without transcript
text. Older previews, regressing times and already-completed IDs are rejected;
the first final for a known earlier pending sentence remains durable. An old ID
outside the bounded window cannot be admitted as a new sentence.

Register each admitted sentence's identity once on the reliable event lane.
This metadata changes neither displayed text nor confirmation state and keeps
provider order when replaceable drafts coalesce. Publish source and translated
previews as one complete latest-value event; applying it publishes one session
snapshot after both lines are updated. Complete final pairs retain the same ID.
This also preserves two distinct sentences with identical final text.

The shared event additions are `UtteranceRegistered` and
`SubtitleIdentifiedPreviewPair`. Existing providers retain their event paths;
Tencent uses the additions through the shared desktop/Android runtime. A queued
newer complete preview survives an older sentence's final. Confirmed IDs and
content revisions still reject obsolete previews and cleared content. Registration
is accepted during the bounded stop-tail drain without reopening previews.

Clearing discards every already-observed sentence and advances only content
revision. A new sentence can resume without waiting for a cleared final. Socket
generation remains independent. Intermediate text from the current sentence can
still be revised by Tencent until `sentence_end=true`; this repair does not add
a local segmentation or translation-rewrite suppression algorithm.

## Connection and audio boundaries

Pace complete PCM frames no faster than one every 200 ms using a monotonic
clock, including a padded final tail. A delayed old send or end message cannot
write into a replacement connection. Preserve existing bounded audio queues,
send deadlines and fresh signed URLs/voice IDs for reconnects.

Ready state is published by the receive task, so a terminal immediately after
handshake cannot be revived by a late connect completion. Provider errors,
invalid responses, unexpected closes and final completion invalidate readiness,
wake finish waiters and attempt a bounded close. Finishing after a provider
rejection does not add a misleading timeout error. Provider messages and signed
URLs remain excluded from error feedback.

## Verification boundary

Use only synthetic credentials, documented JSON fixtures and loopback WebSocket
servers. Cover packet identity/timing validation, atomic preview replacement,
late finals, duplicate finals, repeated text under different IDs, clear without a
final, frame pacing, service rejection during finish and immediate terminal
handshakes. Run shared Rust/core/JNI-adapter checks and architecture guards;
desktop/Android CI verifies the platform seams. Local JNI-adapter Rust tests are
not a Java JNI invocation or an Android device test.

No Tencent requests, audio uploads, credential reads, service activation or
funding are part of these checks. Native capture/UI, Java JNI, cross-platform
applications, live translation quality, account permissions and quota remain
unverified locally. Do not replace or stop an existing test application.

Main already provides integrated Tencent recognition and translation. It has no
Tencent ASR plus independently selected MT route to preserve as a second mode.
Keep the existing single service path and defaults; this repair changes only
protocol admission, paired presentation and connection termination. Profile,
preference and credential schemas require no migration.
