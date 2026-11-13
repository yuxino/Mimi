# Provider JSON transport validation

Issue #369 exposed a Gemini ASR transport boundary: valid UTF-8 JSON in a
WebSocket Binary message was rejected before the existing decoder ran. The
follow-up audit found other existing Binary JSON paths replacing invalid UTF-8
with U+FFFD, allowing corrupted transcripts to become subtitle content.

## Contract

- Text and supported Binary JSON use the same protocol decoder. Binary bytes
  must be valid UTF-8; malformed input follows the existing protocol-error path.
- Gemini ASR accepts documented standalone `voiceActivity` metadata without
  treating it as a transcript or final. Transcript events in the same message
  retain priority. Unknown or malformed transcription messages still fail.
- Gemini Live (including replacement and rotation), Audio3/DashScope, Aliyun
  LiveTranslate, OpenAI Realtime and Azure Realtime retain their existing Binary
  JSON support but no longer repair corrupt bytes into subtitle text.
- Independent Gemini, xAI STT and custom ASR setup errors preserve the exact
  internal protocol/rejection labels across the translation adapter. They stop
  as provider failures; authentication and retryable transport categories stay
  unchanged. Connection probes distinguish incompatible responses from explicit
  provider rejection using existing localized copy.
- Tencent/Baidu/xAI voice binary audio and Volcano protobuf remain separate
  protocols. This change does not interpret their binary payloads as JSON.

## Evidence and validation

Shared `translation-contracts.json` contains synthetic Unicode transcripts,
malformed bytes inside otherwise valid JSON, and Gemini activity notifications.
Direct Rust transport tests and actual JNI loopback tests replay these contracts;
pure classification tests cover setup retry categories and probe serialization.
No private audio, credentials or provider response payloads are logged.

Official references: [Gemini WebSocket schema](https://aistudio.google.com/api-reference/live),
[Gemini ASR](https://aistudio.google.com/docs/live-api/live-transcribe), and
[JSON UTF-8 requirement](https://www.rfc-editor.org/rfc/rfc8259#section-8.1).

## Limits

Loopback regressions establish transport behavior, not production service
availability or recognition quality. Azure, Tencent and Volcano have older
setup-error variants shared by explicit rejection and transport termination;
changing their retry policy requires a separate typed-error split and regression
coverage. This patch does not change that ambiguous boundary or provider defaults.
