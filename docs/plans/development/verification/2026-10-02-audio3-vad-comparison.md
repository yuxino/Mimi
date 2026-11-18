# Audio3 VAD segmentation comparison

## Rejected native trial

The current live-song session produced very few ASR finals while drafts kept
being revised. This observation does not establish that segmentation caused the
incorrect recognition, or that a VAD change will correct singing recognition.
The user has paused release and requested continued investigation. A signed
development build tested the VAD combination below on the authorized live-song
video, with explicit English selected. During the approximately 01:35–03:10
video interval, audible playback continued but the original subtitle still
changed between short English and Chinese fragments. Only one server final
was observed, with a content-free length of 8 characters. This did not meet the
trial's earlier-useful-boundaries and speech-retention acceptance criteria.

The seek start was not perfectly controlled against the baseline, and no
reference-aligned WER was measured. This is a qualitative rejection of the
observed experience, not a quantitative accuracy comparison or evidence that
these parameters caused the recognition errors.

The encoder and its protocol fixtures have been restored to the pre-trial
source: `semantic_punctuation_enabled=true`, with no explicit
`max_sentence_silence` or `multi_threshold_mode_enabled`. No VAD parameter
change remains in the application source. Recognition quality on this segment
remains unresolved; restoring the baseline is not claimed to fix it.

The Audio3 route has enabled `semantic_punctuation_enabled=true` since the Swift
implementation introduced in `b01932f` on 2026-08-03. `469b3ff` carried it into
Rust. The later draft-identity change did not change the recognition request.

## Official contract and scope

The [official client-event specification](https://help.aliyun.com/en/model-studio/qwen-audio-asr-streaming-client-events)
and [Python SDK parameter reference](https://help.aliyun.com/en/model-studio/qwen-audio-asr-streaming-python-sdk)
define `semantic_punctuation_enabled=true` as semantic segmentation with VAD
segmentation disabled. With this option enabled, `max_sentence_silence` is not
the criterion for returning `sentence_end`. The default `false` uses VAD
segmentation instead.

The rejected comparison sent exactly these three segmentation parameters:

```json
{
  "semantic_punctuation_enabled": false,
  "max_sentence_silence": 800,
  "multi_threshold_mode_enabled": true
}
```

`max_sentence_silence` is the VAD silence threshold in milliseconds, with a
documented range of 200–6000 and default of 1300. The official SDK gives 800 as
an example; it is a test setting, not a proven optimal value. Multi-threshold
mode is documented to prevent overlong VAD segments and takes effect only when
semantic segmentation is disabled. The documentation gives no exact maximum
segment duration for this combination. Turning off semantic segmentation does
not disable the service's separate automatic punctuation behavior.

The trial kept `qwen-audio-3.0-asr-flash-streaming`, the endpoint, heartbeat,
configured language hints, existing audiovisual context, and PCM16 mono 16 kHz
unchanged.
Automatic recognition still omits `language_hints`; explicit English still
sends `["en"]`. Do not add the 3.1-only `vad_model` or `keep_dialect` options,
adjust `speech_noise_threshold`, alter MT models/prompts, or introduce local
punctuation splitting. Preserve actual service sentence IDs, Clear content
revisions, request pacing/cooldown, final FIFO and bounded queues.

## Verification and limits

The trial's protocol fixtures verified the exact three parameters together for
explicit and automatic recognition, retaining the existing model,
language/context, PCM format and heartbeat contracts. The focused red fixture
failed against the old semantic setting; after the change, all 13 Audio3
protocol tests passed. Those request-serialization checks did not establish
native service behavior or lyric correctness. The trial-specific assertions
were reverted along with the encoder after native rejection.

The planned acceptance separated automatic and fixed-English trials and used
the same approved segment, content-free ASR/MT counts and timings, queue depth,
429/recovery observations and visual pair/history checks. Text lengths alone
were not recognition evidence. More ASR finals could also increase MT demand;
queue and quota pressure remain necessary observations in any future trial.

The required result was useful earlier durable boundaries without missed
speech, unnatural fragmentation, repeated old pairs or worse
rate-limit/recovery behavior. The observed native run did not meet that bar,
so this experiment was rejected instead of treating the green fixtures as a
product fix. No provider request bodies, lyrics or credentials are retained
in this record.
