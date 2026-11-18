# Baidu realtime translation sentence boundaries

## Baseline and scope

The main-branch baseline `b04e394b` already implements Baidu's integrated
WebSocket speech translation. This change repairs that shared runtime rather
than adding another provider or an HTTP speech-translation path. It is independent
of Alibaba draft PR #266 and the Tencent work.

Protocol reference: [Baidu realtime speech translation](https://ai.baidu.com/ai-doc/MT/Sl9p2h5k9).
The existing `baiduTranslate` profile and its `appId` / `appKey` credentials stay
compatible. AppKey means the same machine-translation application's API Key;
an ASR application's key, Secret Key, or text-translation credential is not
silently substituted. No stored configuration or credentials are migrated.

The latest request to offer simultaneous translation before the previous route
needs a separate route to identify: this baseline has no Baidu standalone ASR
or independent text-translation route. The protocol repair introduces no
duplicate mode, fabricated legacy route, or main-branch default change.

## Behavior

- START uses the existing 16 kHz, mono, PCM16 little-endian stream and explicitly
  disables TTS. Audio begins only after STA. Complete 1,280-byte frames are sent
  at least 40 ms apart, including batched capture buffers; one partial tail is
  zero-padded before FINISH. Buffering and send/finish deadlines remain bounded.
- MID updates the current replaceable source and translation drafts. FIN's
  `sentence` / `sentence_trans` confirm the completed pair. Its `asr` /
  `asr_trans` belong to the following draft and are emitted afterwards. They
  cannot be merged into the completed sentence or recorded as confirmed text.
- Clear excludes the current sentence's remaining results without reconnecting
  or losing the next sentence carried by FIN. A decoded packet racing with
  clear cannot reintroduce either its draft or that draft's later final.
- The documented sentence-local ASR/translation/empty-result failures end the
  failed sentence so clear can admit the next one. TTS-only failure does not
  advance recognition. Invalid audio is not assumed to be recoverable.
- The documented 30-second idle disconnect enters the existing bounded
  transport recovery policy. WebSocket Ping/Pong remains the health probe;
  no undocumented application heartbeat is invented.
- Provider error prose and unknown event labels never enter diagnostics.
  Confirmed pairs use the existing reliable event lane; previews remain
  bounded and replaceable, with generation and clear-revision guards.

## Evidence boundary

Direct Rust protocol fixtures and loopback WebSocket tests cover START/STA,
MID/FIN handoff, clear, sentence errors, idle classification, PCM pacing,
padded FINISH/END, sanitization and bounded failures. An actual JNI loopback
test exercises the same shared client, controller and Android subtitle bus.

No real credentials are read, no provider API is called, and no audio is
uploaded. Account permissions, quota, live 30-second idle handling, real
translation quality/latency and native audio/overlay acceptance remain untested.
The running test or installed applications are not replaced or restarted.
