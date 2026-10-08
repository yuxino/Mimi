# Gemini Live translation setup and protocol evidence

## Evidence and request

Model comparisons and upstream observations below were recorded on 2026-10-04.
They are bounded evidence for that investigation, not a current provider catalog.

The reported Settings connection check rejected a valid Gemini key. Against
the same key, Google's model listing returned HTTP 200 and included
`gemini-3.5-live-translate-preview`. The existing setup closed with WebSocket
1007 and an unknown `inputAudioTranscription` field. Moving both transcription
options from `setup.generationConfig` to `setup` returned `setupComplete`.
The native Settings speech-service check then returned available in 971 ms.

During that investigation, the translation guide's raw WebSocket example nested transcription
options differently from the running API. Keep the generation and translation
configuration unchanged; use the actual accepted setup shape, also reflected
by the [Live API reference](https://ai.google.dev/api/live).

## Model capability investigation

The user's account returned the exact dedicated translation model through the
official model listing, version `3.5-live-translate-06-2026`, alongside dedicated
transcription and dialogue models. The desktop setup explicitly requests
`models/gemini-3.5-live-translate-preview`; no automatic model fallback exists in
Mimi. This is not a claim about Google's internal serving alias.

| Model | Purpose | Translation path |
| --- | --- | --- |
| `gemini-3.5-live-translate-preview` | Continuous speech-to-speech translation | Audio input directly returns translated speech; optional input/output audio transcription supplies subtitle text. No independent text translator is necessary. |
| `gemini-3.5-transcribe-live` | Dedicated streaming speech-to-text | Returns recognition text with interim/final fields; a separate text translator would be necessary for translated subtitles. It is a different integration. |
| `gemini-3.8-live` | General real-time voice agent/dialogue | Uses dialogue instructions and turn behavior; it is not a drop-in replacement for the dedicated translation configuration. |

Sources: [Live Translation](https://ai.google.dev/gemini-api/docs/live-api/live-translate),
[Live Transcription](https://ai.google.dev/gemini-api/docs/live-api/live-transcribe),
and [Gemini 3.8 Live](https://ai.google.dev/gemini-api/docs/models/gemini-3.8-live).
Mimi's Gemini route constructs `GeminiLiveClient` directly and displays
`inputTranscription`/`outputTranscription`; it does not send the recognized text
to another translation service. Output transcription represents translated audio.
Joint errors in both text streams do not prove an internal ASR-then-MT pipeline,
and text evidence alone does not independently establish spoken-output quality.

Two bounded direct WebSocket comparisons reused the same Chinese PCM. Requesting
`TEXT` from the dedicated translator still generated 816,000 bytes of audio,
no model text parts, and an English output transcript. Session acceptance alone
therefore does not prove a text-only translation mode. Substituting `gemini-3.8-live`
with the same translation configuration also accepted setup, but returned a
Chinese conversational response instead of translating to English. Keep the
documented dedicated model and `AUDIO` setup; do not change the model based only
on its version number or successful setup.

A subsequent configured comparison streamed the same three fixed FLEURS PCM
inputs separately through the dedicated translator and `gemini-3.8-live`.
The dialogue model used a translation-only `systemInstruction`, no
`translationConfig`, automatic activity detection, and `NO_INTERRUPTION` so
later source speech could not cut off generated speech. Both received identical
16 kHz mono audio at 100 ms cadence, two seconds of trailing silence and
`audioStreamEnd`. The observation window was bounded. This corrects the earlier
uninstructed dialogue probe; it is still only one run per input/configuration,
not a general model ranking or a native Mimi integration of the dialogue model.

The dedicated translator returned its first nonempty translated transcript
earlier in all three cases. The dialogue model corrected the Japanese time
condition but changed an English ranking; its Chinese output transcription
contained unexpected markup and omitted part of the clause despite generation
completion. Transcript evidence does not identify whether that last failure
originated in generated speech or the output transcription channel.

A third bounded prototype used `gemini-3.5-transcribe-live` with automatic
language detection, `VERBATIM`, and no reference vocabulary. Only authoritative
`inputTranscription` events went to `gemini-3.8-flash` via `generateContent` with
low thinking and a translation-only instruction. Interim hypotheses were
recorded privately but not translated. Japanese semantics improved in this
sample; English recognition errors and the Chinese place-name error remained
and were carried into text translation. All three text requests returned HTTP
200. This final-only prototype does not measure a production pipeline with
preview translation, cancellation, context, reconnects or overlay rendering.

Keep the current dedicated translator for the existing Gemini route. The
comparison supports its continuous, earlier-output behavior for this subtitle
use case, but not an accuracy endorsement. A transcription-plus-text route is a
separate potential integration, not a model-name substitution. Do not add
post-hoc word replacement rules or switch defaults based on these three clips.
Exact timings, semantic limits and private evidence boundaries are in the
integration run ledger.

At the time, the [pricing page](https://ai.google.dev/gemini-api/docs/pricing) listed this same
translation model for both free and paid tiers, and the
[billing guide](https://ai.google.dev/gemini-api/docs/billing) described tier,
quota and data-policy differences. Neither establishes a free-tier accuracy
downgrade. No paid-account control was performed, so account effects remain
unproven. Do not attribute the observed semantic errors to the free tier.

## Changes

- Desktop and Android emit transcription options beside `generationConfig`.
  Three complete setup fixtures in `shared/translation-contracts.json` are
  consumed by both platforms; nesting regressions must fail their tests.
- Real audio exposed a second boundary: legal language-only transcription
  objects omit the default empty `text`, and this continuous translation model
  did not emit `turnComplete`. Accept missing text as an empty update, while
  still rejecting a non-string value. Feed append-only deltas through the
  shared Rust transcript aligner on desktop and through actual JNI on Android.
  A repeated live sample exposed unequal punctuation: the translation's extra
  sentence was paired to a later source sentence. Gemini therefore uses a
  separate mode of that shared aligner, buffering append-only previews until
  both whole buffers end in sentence punctuation and no nonempty transcript
  delta has arrived for 2 seconds. Commit the whole block together, regardless
  of punctuation counts. The clock is a monotonic receipt clock, never an
  invented audio timestamp. This is a heuristic stable checkpoint, not a
  provider terminal guarantee; sustained speech may keep previews pending.
  The fixed 5,120-character safety ceiling rejects excessive unmatched growth.
  Preserve explicit turn boundaries and their 500 ms late-tail grace. At a
  bounded stop deadline, an empty pending buffer finishes cleanly; unmatched
  or unpunctuated content still times out and is discarded. Shared fixtures
  and actual JNI cover split translations, repetitions, empty updates,
  explicit boundaries and unmatched tails. Platform adapters only schedule
  polling and map events; the assembly and checkpoint policy remain shared.
Development uses ordinary editable profiles in its own private local file,
with no environment-backed presets. See the
[development setup guide](../development/local-dev-credentials.md).

## Verification

Use both shared-fixture consumers and the canonical repository check. The
ignored `settings_store::gemini_live_tests::manual_gemini_translation_audio` test
is compiled only on macOS with `development-debugger`. It requires a saved,
selected Gemini development profile and a bounded private PCM manifest. Run it
only with explicit authorization for the current provider/audio check. It streams
16 kHz mono PCM through `GeminiLiveClient` at real-time cadence, uses its existing
finalization path and writes bounded private final-pair evidence. Logs contain
only counts and timings. It does not start OS capture, open GUI windows or alter
the saved profile selection.

Record actual audio cases, results and remaining limits in the integration run
ledger after execution. Service acceptance, direct-client audio translation,
native capture and visible overlay behavior are separate claims.
