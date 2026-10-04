# Gemini Live setup and local development preset

## Evidence and request

The reported Settings connection check rejected a valid Gemini key. Against
the same key, Google's model listing returned HTTP 200 and included
`gemini-3.5-live-translate-preview`. The existing setup closed with WebSocket
1007 and an unknown `inputAudioTranscription` field. Moving both transcription
options from `setup.generationConfig` to `setup` returned `setupComplete`.
The native Settings speech-service check then returned available in 971 ms.

The translation guide's raw WebSocket example currently nests transcription
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

The [pricing page](https://ai.google.dev/gemini-api/docs/pricing) lists this same
translation model for both free and paid tiers, and the
[billing guide](https://ai.google.dev/gemini-api/docs/billing) describes tier,
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
- At the user's explicit request, extend the existing default-off macOS local
  credential exception with an independent `Google Gemini · dev` preset.
  `GEMINI_API_KEY` is read only for that preset's Gemini account. The Alibaba
  preset remains compatible with existing single-key files. A Gemini assignment,
  including an empty assignment, enables its preset; removing it removes that
  preset at the next startup. Existing explicit selections are retained.
- Apply the same file ownership, regular-file, no-symlink, `0600`, size and
  literal-value validation to both keys. Reject duplicate provider assignments
  and unsupported variable names. An invalid file reports unavailable presets
  without an OS fallback. No process environment credential input is added.
- Both presets prohibit save, delete, reveal, copying to ordinary profiles and
  independent translation overrides. Ordinary development configurations keep
  their own editable OS credentials. The two presets do not consume user slots.
  The Cargo feature, macOS dev identifier and non-UI-only gates still precede
  all file access; production behavior remains OS-backed.

## Verification

Use synthetic loader/catalog/UI regressions, both shared-fixture consumers,
and the canonical repository check. An ignored headless test invokes the same
native service check as Settings. A separate explicit ignored audio test streams
bounded private 16 kHz mono PCM through `GeminiLiveClient` at real-time cadence,
uses its existing finalization path and writes bounded private final-pair
evidence. Logs contain only counts and timings. It does not start OS capture,
open GUI windows or alter the saved profile selection.

Record actual audio cases, results and remaining limits in the integration run
ledger after execution. Service acceptance, direct-client audio translation,
native capture and visible overlay behavior are separate claims.
