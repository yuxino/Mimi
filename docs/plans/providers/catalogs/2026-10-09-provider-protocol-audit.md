# Provider protocol and region audit

Checked against production shared Rust paths and official documentation on
2026-10-09. This accompanies the measured-only timing change in PR #235.
No provider prompt, audio source, credential destination or dependency changes.

## Confirmed fixes

xAI Voice has distinct input-item and output-response lifecycles. A transcript
`done` event does not establish that a response succeeded. Keep the current
response and the next input separate; publish a durable pair only after its
matching successful `response.done` and complete source/transcript. Grok also
sends `input_audio_transcription.completed` with `status=in_progress` for partial
source text; event names alone cannot establish the source final boundary. Retire old
IDs so late cancellation, completion or text cannot overwrite the next turn.
Normal VAD cancellation discards that response without failing the whole session.
Preview lanes and durable finals retain the actual source item ID through the
existing shared reducer. Clearing content or dropping an oversized turn also
retires queued source IDs and retains the pending response boundary; late events
cannot restore discarded subtitles or pair that response with a newer source. Pairing remains bounded and
never infers identity from similar text.
If multiple following items make response ownership ambiguous, clear unconfirmed
content and trigger one existing transport recovery; a reconnect restores pairing.

Sending PCM does not establish a speech turn: silence is streamed as well.
After sending the final padded frame, keep receiving for the existing two-second
finish window, allowing server VAD and a later response to arrive. At the
window end, an observed unfinished speech/committed turn is a timeout; an idle
silent stream finishes normally. This can add two seconds to silent Stop; it
neither expands the outer finish budget nor treats a Pong as a VAD acknowledgement.

Azure's dedicated session transcript is append-only with no required per-turn
done boundary. Forward deltas without retaining a full-session transcript.
For the alternative `response.text` stream, retain at most 4,096 bytes to compare
a normal done transcript with the already forwarded prefix. If that bound is
exceeded, release the prefix and continue forwarding every delta. At done, do
not replay the response or guess a suffix: text present only in that done event
is omitted for the oversized response. The next response resumes exact matching.
Selecting one event stream per session prevents duplicate translated text.

## Dedicated translation contracts

OpenAI recoverable errors keep the socket open and now emit one fixed diagnostic
label instead of silently disappearing. Neither provider error fields nor subtitle
text are logged. Related setup rejection and fatal error behavior stay intact.

OpenAI's `gpt-realtime-translate` uses `/v1/realtime/translations`, continuous
24 kHz mono PCM16, translation `session.*` events and `session.close` followed by
`session.closed`. It is not ordinary Realtime chat and must not receive chat VAD,
`response.create` or rewritten translation instructions.
[Translation guide](https://developers.openai.com/api/docs/guides/realtime-translation),
[client events](https://developers.openai.com/api/reference/resources/realtime/translation-client-events).

Microsoft explicitly documents `/openai/v1/realtime/translations?model=<deployment>`,
`api-key`, `audio.output.language`, `session.input_audio_buffer.append`,
`response.text.delta` (`text`) and `response.text.done`. The implementation matches
that dedicated example; no preview `api-version` is added. Microsoft's Realtime
reference explicitly adopts the upstream protocol and requires an existing
**deployment name** for input transcription. Its Translate/Whisper model guides
also describe parallel source transcription. Combined with the dedicated
upstream translation fields and close events, the current deployment alias and
`session.close` handling conform to the documented contract. This is a
cross-document conclusion; the short Microsoft translation example omits these
optional/configuration and close details.
[Azure protocol deviation](https://learn.microsoft.com/en-us/azure/foundry/openai/realtime-audio-reference),
[Translate model](https://learn.microsoft.com/en-us/azure/foundry/openai/concepts/gpt-realtime-translate),
[Whisper model](https://learn.microsoft.com/en-us/azure/foundry/openai/concepts/gpt-realtime-whisper).
The current `openai.azure.com/.cn/.us` whitelist is an implementation constraint,
not proof that these clouds offer this model; generic v1 host guidance does not
justify accepting a new host for this endpoint.
[Microsoft dedicated example](https://learn.microsoft.com/en-us/azure/foundry/openai/how-to/realtime-audio-websockets#translate-audio-in-real-time).

xAI `grok-voice-latest` is a voice agent constrained by the existing translation
instructions, not OpenAI's dedicated translation service. Its VAD and
`response.created/done` lifecycles must be handled independently.
[xAI voice guide](https://docs.x.ai/developers/model-capabilities/audio/speech-to-speech),
[lifecycle reference](https://docs.x.ai/developers/rest-api-reference/inference/voice).
Use the documented `turn_detection: {type: server_vad}` default configuration,
removing the undocumented `silence_duration_ms` override. The local finish tail
uses the official interpreter example's 1.5-second silence policy, rounded up to
whole frames; it is not a claim about the server's VAD threshold.
[Official interpreter configuration](https://github.com/xai-org/xai-cookbook/blob/main/examples/live-interpreter/typescript/src/interpreter.ts),
[partial transcription and silence handling](https://github.com/xai-org/xai-cookbook/blob/main/examples/live-interpreter/typescript/src/index.ts).

## Regions: service support versus Mimi settings

| Service path | Current Mimi configuration | Official region constraint |
| --- | --- | --- |
| Alibaba Audio3; legacy LiveTranslate path | Built-in Beijing endpoint; no region selector. Custom DashScope can use a compatible full WSS/model. | Speech models document Beijing and Singapore; key, endpoint and model availability must match. Custom Audio3 uses `run-task`, not the different `/realtime` protocol. [Audio3](https://help.aliyun.com/en/model-studio/qwen-audio-asr-streaming-python-sdk), [LiveTranslate](https://help.aliyun.com/en/model-studio/qwen-livetranslate-python-sdk). |
| Qwen-MT Lite/Flash/Plus | Built-in Beijing HTTP endpoint, model selectable. | Beijing, Singapore, Frankfurt and Virginia are documented for these models, with deployment-scope differences. A generic chat-compatible request does not replace Qwen's `translation_options`. [Lite](https://www.alibabacloud.com/help/en/model-studio/qwen-mt-lite), [Flash](https://www.alibabacloud.com/help/en/model-studio/qwen-mt-flash), [Plus](https://www.alibabacloud.com/help/en/model-studio/qwen-mt-plus). |
| OpenAI Realtime Translation | Fixed `api.openai.com`; no region setting. | Dedicated translation data residency covers US/Europe, subject to project eligibility; the wider ordinary API region list cannot be applied automatically. [Data controls](https://developers.openai.com/api/docs/guides/your-data). |
| Azure OpenAI | User's resource root and actual deployments. | Availability depends on this model and deployment type. Global Standard resource region does not guarantee inference occurs there. [Current availability](https://learn.microsoft.com/en-us/azure/foundry/foundry-models/concepts/models-sold-directly-by-azure-region-availability?pivots=standard). |
| xAI Voice | Fixed global `api.x.ai`. | The US regional endpoint explicitly excludes Voice; changing to `us.api.x.ai` is not supported. [Regional endpoints](https://docs.x.ai/developers/advanced-api-usage/regions). |
| Gemini Live | Fixed Google Developer API WebSocket. | Mimi does not implement Vertex AI regional endpoint/authentication. [Live translation](https://ai.google.dev/gemini-api/docs/live-api/live-translate). |
| Volcano AST | Fixed `openspeech.bytedance.com`. | Overseas BytePlus Live Interpretation has a separate Singapore endpoint, resource and credentials; it is not a transparent region switch. [BytePlus API](https://docs.byteplus.com/es/docs/byteplusvoice/liveinterpretationapi). |
| Tencent speech translation | Fixed signed `asr.cloud.tencent.com` WebSocket. | This exact protocol has no Mimi region setting; ordinary Tencent HTTP ASR region rules do not establish alternative WS hosts. [Speech translation](https://cloud.tencent.com/document/product/1093/127565). |
| Baidu realtime speech translation | Fixed `aip.baidubce.com` WebSocket. | The exact API documents no region parameter. [Realtime protocol](https://ai.baidu.com/ai-doc/MT/Sl9p2h5k9). |
| DeepL | Free/Pro endpoints selected from key; no region field. | US/JP regional endpoints require a signed regional deployment addendum and activation, otherwise 403; EU is the default. [Regional endpoints](https://developers.deepl.com/docs/getting-started/regional-endpoints). |
| DeepLX, ChatMock, OpenAI-compatible text; custom ASR | Endpoint/model editable. | Region depends on the actual compatible service. A nearby proxy or local ChatMock does not establish that upstream inference is local. [DLX](https://github.com/OwO-Network/DLX), [ChatMock](https://github.com/RayBytes/ChatMock). |
| Apple Speech/Translation | Local native services and language assets. | No cloud region selector; connected-state health is not a network RTT sample. |

A nearby ingress can reduce network RTT, but model processing, sentence/VAD
waiting, queueing, rate-limit recovery and proxy routing also affect completed
subtitles. Alibaba explicitly separates ingress/storage region from inference
scope. Compare the same audio/model/route using stage diagnostics; a WebSocket
Ping measures neither the independent text service nor audio-to-screen delay.
[Alibaba regions](https://www.alibabacloud.com/help/en/model-studio/regions).

No region selector is introduced by this audit. Live provider acceptance and
regional account eligibility remain separate from official-document matching
and synthetic WebSocket regression tests. This audit does not establish live Azure service acceptance.
Delivery remains a child PR into PR #235's branch only; no main merge or release.
