# Custom speech recognition

## Product scope

Custom recognition and text translation are independent stages. Users select
either the DashScope ASR WebSocket protocol or OpenAI Realtime standalone
transcription, then supply a full endpoint, model and API key. HTTP file
transcription and Chat Completions compatibility alone do not meet the live
audio contract. Custom recognition defaults to Original, without a text request.
DeepL, DeepLX and OpenAI-compatible Chat Completions remain independent text
destinations. Audio sources are configured independently; see the
[audio input design](2026-10-03-optional-microphone-input.md).

The settings detail shows stage headings, protocol identity, fields and actions
at normal size. Requirements, model-dependent languages and storage explanations
use help icons with hover/focus tooltips. Validation errors remain beside the
field and receive focus. Actions share compact icons and right alignment.
Recognition and translation have separate save actions. Changing an endpoint
requires a replacement key rather than sending an existing key to another host.
They also have independent connection checks. The recognition check measures
actual session setup without PCM; the text check requests a fixed public short
phrase and requires a nonempty response. Each check reads only its stage's saved
credentials and reports request duration in milliseconds, not subtitle latency.
Text checks remain available before recognition is configured. Missing settings
and UI preview mode have explicit results and no invented timing measurement.

## Protocol and streaming

- DashScope: 16 kHz mono PCM16 binary frames, `run-task`, `task-started`,
  `result-generated` sentence events and `finish-task`.
- OpenAI Realtime standalone transcription: 24 kHz mono PCM16, a transcription
  `session.update`, `input_audio_buffer.append` and explicit commits, then input
  transcription delta/completed events. Client segmentation bounds silence,
  preroll and utterance length; item ordering preserves confirmed sentence order.
  Reconnect, Clear, cancellation, pending items and drafts remain bounded.

Recognition uses the same bounded original/translation pipeline as Alibaba
Audio3. It never constructs Qwen-MT with a custom speech key. Original makes no
translation request and reads no text credential. Selected text destinations
reuse their existing bounded final/preview workers and quota recovery scope.

## Configuration and privacy

Provider kinds `customDashScopeASR` and `customOpenAIASR` identify the exact wire
protocol. The private local credential file holds a validated speech
endpoint/model/key record and a separate profile-scoped text credential slot.
Preference JSON and IPC snapshots contain only non-secret metadata and stage
availability. Stage availability comes from each required local-file slot;
aggregate readiness is not a substitute for the individual states. Save/delete
rollback preserves independent records.

Full WSS URLs are required, except WS on loopback. URL credentials, query
parameters, fragments and control characters are rejected. OpenAI's intent
parameter is added internally only when connecting. Models are bounded to 256
bytes. Production and development use separate editable private local files;
native stores are only one-time legacy import sources. See the
[storage design](2026-10-04-local-credential-storage.md). Diagnostics contain
sanitized labels, timing, counts and language codes only.

Resolve custom source choices from the protocol's encodable catalog, any saved
model-language declaration, and the selected text route's intersection. A generic
protocol code does not promise model support or automatic detection. Original
remains available; independent targets follow their own route catalog. See the
[current language rules](2026-10-06-provider-language-catalogs.md).

## Verification

Focused tests cover wire setup, validation, source/destination isolation,
write-only snapshots, rollback, Original without MT, ordered final translations,
connection probes without PCM, streaming bounds and stale events. Frontend tests
cover independent forms, tooltip copy in all seven interface languages, field
error focus, retained drafts, address/key guards, deletion and active-session
locks. Before release, run `scripts/check.sh`, signed native UI checks, and the
platform CI/release asset checks. Record real-provider limits separately from
synthetic protocol fixtures.
