# Repeatable, content-free Alibaba ASR comparison

## Accepted scope

The Queen native reproduction was qualitative: seeks and system capture did not
guarantee identical uploaded samples. This opt-in benchmark sends the same public
fixture PCM directly to ASR, independently of capture, HQ, MT and the overlay.
It measures recognizer behavior; it does not prove capture-to-screen latency,
translation correctness or installed-app acceptance. Publication remains paused.

`audio3_benchmark.rs` is registered only under `cfg(test)`. The paid ignored test
exists only on macOS with the `local-dev-credentials` feature. Ordinary app builds
have no benchmark entry, model switch, configuration or credential behavior change.

## Controlled arms

- `baseline`: the production Audio3 3.0 encoder and existing audiovisual context.
- `no-context`: the same 3.0 request with `input={}`.
- `model31`: the same no-context request, with a test-only allowlisted model name
  `qwen-audio-3.1-asr-flash-streaming`.
- `realtime-asr`: **explicit selection only**, standalone
  `qwen3-asr-flash-realtime-2026-02-10`, no context or MT. This is a different
  recognizer/protocol comparison, not an isolated Audio3 parameter comparison.
  Omitting the arm selector still runs only the original three arms.

A/B isolates context; B/C isolates model selection. Fixed language hints, semantic
segmentation, wire heartbeat, endpoint and resolved proxy remain the same. The
rejected VAD trial is not reintroduced. No translation model or prompt changes.

[The official model page](https://help.aliyun.com/en/model-studio/qwen-audio-3-1-asr-flash-streaming)
lists 3.1 streaming; [the shared client protocol](https://help.aliyun.com/en/model-studio/qwen-audio-asr-streaming-client-events)
provides its `run-task` sample. This proves a documented API, not availability or
quality for the user's account. A rejected arm reports failure without falling
back to 3.0. That protocol defines context as previous recognition/domain terms;
the existing generic audiovisual description is a hypothesis to test, not a
proven cause of the Queen recognition errors.

The [connection guide](https://help.aliyun.com/en/model-studio/qwen-audio-asr-streaming-websocket-api)
recommends workspace-specific domains while identifying the legacy DashScope
domains as migration sources. This comparison preserves the app's current Beijing
endpoint in every arm; it does not silently infer a workspace or mix endpoint
migration into the model comparison.

The [official realtime model/region list](https://help.aliyun.com/en/model-studio/real-time-speech-recognition-user-guide)
lists the fixed February 2026 Qwen3 snapshot in Beijing and Singapore; the stable
alias currently refers to the October 2025 snapshot. The new arm keeps the current
Beijing legacy domain, which the [Qwen-ASR connection guide](https://help.aliyun.com/en/model-studio/qwen-asr-realtime-interaction-process)
explicitly says remains functional. The URL is `/api-ws/v1/realtime?model=...`,
not Audio3's `/inference` `run-task` protocol. This route exists only inside the
benchmark; no production protocol, model, segmentation policy or prompt changes.

## Inputs and bounded lifecycle

Use a manifest object with a `clips` array, at most ten entries:

```json
{
  "clips": [
    {
      "label": "speech-en",
      "pcm_path": "/path/to/public-corpus/speech-en.pcm",
      "reference_path": "/path/to/public-corpus/speech-en.txt",
      "language": "English",
      "unit": "word"
    }
  ]
}
```

Languages are `English`, `Japanese`, `Chinese`, or `auto`; units are `word` or
`char`. A null reference omits WER/CER rather than inventing ground truth. Labels
are short ASCII fixture identifiers. All paths must be absolute regular files
inside the manifest directory; links, outside paths, secret-like extensions,
unknown fields, duplicate labels, odd PCM lengths and oversized input are rejected.
The manifest and references are capped at 64 KiB. PCM is raw signed little-endian
PCM16, mono 16 kHz, at most 60 seconds per clip. References have at most 4096
normalized evaluation units; recognized finals have at most 8192 evaluated units.

The tool reuses the production encoder, decoder and immutable `ProviderNetwork`
route. It uses the WebSocket directly because the production client injects idle
zero PCM and the event watch coalesces drafts. Waiting for genuine `task-started`
precedes any PCM. Exactly 20 ms / 640-byte frames are paced on a monotonic clock,
then exactly two seconds of explicitly counted zero PCM. There is no automatic
silence insertion, capture, microphone, audio recording or MT request. The final
partial frame keeps its actual sample duration. Sender and receiver run together;
`finish-task` is sent once and genuine `task-finished` is required, with a ten-second
finish deadline and an overall clip + tail + 30-second bound. Error/timeout drops
both futures and socket ownership. Arms and clips run serially, never concurrently.

The fourth arm instead follows the [Qwen-ASR realtime client events](https://help.aliyun.com/en/model-studio/qwen-asr-realtime-client-events):
`session.update` selects PCM/16 kHz, the requested language if explicit, and the
balanced VAD configuration (threshold 0.2, silence 800 ms). It sends no corpus,
translation configuration or instructions. A successful WS handshake or
`session.created` is insufficient: no PCM is uploaded until a matching
`session.updated` configuration arrives. The server reference uses `pcm16` in
acknowledgement examples; both `pcm` and `pcm16` echoes are accepted, while the
normative client request always sends `pcm`. Missing required or conflicting
language/VAD/audio acknowledgements fail visibly. The same 20 ms PCM frames and
two-second tail are sent through the existing Base64 append encoder. Finish uses
the existing `session.finish` encoder and requires genuine `session.finished`;
ready/send/finish/overall limits match the original arms. PCM limits are checked
before any connection attempt.

Safe readiness failures expose only a fixed `realtime_stage` name and numeric
`ready_ack_mismatch_mask`: bits 1/2/4/8/16/32/64/128/256 mean missing session,
PCM format, language, sample rate, modalities, model, VAD type, threshold and
silence respectively. No raw acknowledgement, provider text or arbitrary error
value enters the report. The initial source of a failed request is therefore
distinguishable without publishing response content.

## Credentials and output

The only environment inputs are the non-secret manifest path, optional arm name,
and optional development config-directory path. Keys are not CLI/environment
inputs. The ignored entry reuses `SettingsStore::load` with the exact development
identifier and checks `credential_storage == localDevFile` **before** a credential
read. The existing profile configuration API supplies the Alibaba key and proxy;
there is no `credentials_snapshot()` API. An absent/invalid local dev file cannot
cause a benchmark Keychain lookup. Credentials stay native, authorization headers
are marked sensitive, and underlying errors are replaced with fixed categories.
Do not run the paid benchmark while the native app has an active provider session.

Output lines prefixed `ASR_BENCH_JSON` contain fixed arm/language/unit/error names,
public fixture labels and numerical metrics only. No paths, key, context,
recognition, reference, provider error body or raw response is serialized. The
decoder sees one bounded event at a time. Revision comparison retains only the
last replaceable draft; WER/CER keeps the public reference plus two numerical
Levenshtein rows, not an accumulated hypothesis or recognized transcript. Only
authoritative final units advance the distance. Same final-ID replays are counted
and excluded from scoring; genuinely repeated words in different IDs remain.
Case is folded and punctuation/whitespace omitted according to word/character
units; there is no Unicode normalization or provider-specific correction.

For the explicitly authorized development debugger comparison, the optional
`MIMI_ASR_BENCH_PRIVATE_OUTPUT_DIR` enables private disk evidence. It is restricted
to `/private/tmp/mimi-debug-benchmark` and its descendants, and to the `baseline`
arm with the existing production encoder. Each clip gets a new 0700 directory
with 0600 `request.json`, `events.jsonl`, and `metrics.json`. Events preserve
decoded draft/final text, real sentence IDs, sentence/word audio timestamps and
local receipt time. They omit headers, credentials, endpoints and arbitrary
provider errors. Output is capped at 512 KiB per event and 8 MiB per clip; an
exceeded cap or write failure marks the run incomplete. This option is off by
default and exists only in the test executable. Standard output stays numerical.
These decoded events are the independent provider baseline; comparison with
native capture/UI requires the exact same fixture, language and route settings.

With native provider activity stopped, run the private baseline with:

```sh
./scripts/asr-baseline.sh /absolute/public-fixtures/manifest.json
```

The script fixes `baseline` and bypass preprocessing, reads credentials through
the existing validated private development-file mode, and never starts Mimi.
Use `language: "auto"` in the manifest when comparing with an automatic-language
app session. Explicit-language runs must remain separately labelled.

Metrics include uploaded frames/bytes, maximum sending lateness, readiness,
first nonempty/final response, draft changes/retractions, lengths, character-class
counts separately for draft and final, identity presence/replays, result gaps and
after-finish finals. Character classes count Unicode letter blocks, not detected
language. At most 512
per-final numeric samples contain actual sentence ID, begin/end audio-clock times,
wall elapsed receipt, length and finish/replay flags. The
[server protocol](https://help.aliyun.com/en/model-studio/qwen-audio-asr-streaming-server-events)
documents sentence clocks and cumulative usage. Receipt minus server `end_time`
is a recognizer-end delay, excludes neither network nor sending lateness, and is
not UI latency. Missing/invalid clocks stay null. Failed runs omit WER/CER so an
incomplete response cannot be presented as a completed comparison. `language` is
the configured hint, not a measured language-detection result.

For the fourth arm, the [server event contract](https://help.aliyun.com/en/model-studio/qwen-asr-realtime-server-events)
separates confirmed `text` prefixes from revisable `stash` suffixes. Their Unicode
Latin/Han/kana counts are **sums across received partial events**: repeated prefixes
are counted again, so these are not unique character counts or independent error
counts. `first_stable_prefix_ms` records the first nonempty `text`. Prefix change,
retraction and retracted-Han counts compare consecutive retained partials only
within the same real `item_id`; they never assume stability across sentences.
An extension is a change but not a retraction. A final contradicting the last
retained prefix has a separate counter. Interleaved older items whose drafts are
no longer retained are not exhaustively compared: these violation counters are
lower bounds, not validation of the provider's complete stability guarantee.
Stash retraction is normal,
including text being promoted to the confirmed prefix, and is not a quality error.
Only one item's text/stash and the common observer's single bounded draft remain
in memory, with their combined text capped at 64 KiB.

Opaque realtime IDs have separate bounded bookkeeping (at most 512 IDs, each at
most 256 bytes). Numeric `sentence_id` values in this arm are private local
ordinals, **not numeric server IDs or ordered watermarks**. Final replays are
excluded using the exact actual ID; different IDs with repeated lyrics remain,
and an older-created item completing after a newer item is not discarded. Final
WER/CER consumes unique finals in receipt order. The returned language metadata
matches the configured language when specified; an `en` label is not evidence
that the text itself contains no Han characters.

Actual realtime speech-start/end clocks are preserved per item when available.
Their documented zero point is not proven identical to the ready-to-upload local
clock, so this arm leaves the Audio3 `sentence_end_delay_*` fields null instead of
subtracting incompatible clocks. `speech_stop_to_final_ms` and its p50/p95/max
measure the local interval between receiving a real same-item `speech_stopped`
with an audio end clock and receiving its final; missing stop/end data remain
null. This measures result delivery after the VAD boundary notification, not
acoustic-end, translation, capture-to-screen or UI latency.

A task rejection disables that arm for remaining clips; an authentication
rejection disables all subsequent requests using the same key. These entries
have `metrics.skipped=1`, the fixed failure category and no evaluation. A rejected
3.1 request therefore cannot cause five identical failed calls or a silent model
fallback. Other network failures remain visible and may be tested on the next
clip. Upload counts include only completed local frame writes, not a server
acknowledgement that those samples were recognized.

[LibriSpeech](https://www.openslr.org/12/) is a suitable public 16 kHz, CC BY 4.0
read-speech reference source. A song/film with no exact approved transcript should
use null reference and receive no WER/CER claim. Synthetic or fixture passes do
not establish real-service quality; actual results and native acceptance are
recorded separately after authorized execution.

## Running

Pure/local fixture validation (does not read credentials or call a provider):

```sh
cargo test --manifest-path src-tauri/Cargo.toml --lib audio3_benchmark -- --skip manual_same_pcm_asr_comparison
```

Explicit paid invocation, after stopping native provider activity:

Replace `/path/to/public-corpus` with the absolute directory containing the
prepared public fixtures and manifest.

```sh
MIMI_ASR_BENCH_MANIFEST=/path/to/public-corpus/manifest.json \
cargo test --manifest-path src-tauri/Cargo.toml --features local-dev-credentials \
  --lib audio3_benchmark::manual_same_pcm_asr_comparison -- --ignored --exact --nocapture
```

Set `MIMI_ASR_BENCH_ARM=baseline`, `no-context`, `model31`, or `realtime-asr` to run
only one arm. The fourth arm requires explicit selection:

```sh
MIMI_ASR_BENCH_ARM=realtime-asr \
MIMI_ASR_BENCH_MANIFEST=/path/to/public-corpus/manifest.json \
cargo test --manifest-path src-tauri/Cargo.toml --features local-dev-credentials \
  --lib audio3_benchmark::manual_same_pcm_asr_comparison -- --ignored --exact --nocapture
```

Omitting it runs all three in the above order. Keep the same prepared PCM files,
account/region, proxy and language hint, then repeat the winner/baseline in reverse
order before attributing a difference to configuration. No provider run has been
performed by the original implementation agent. Later measured comparisons
are recorded separately in development notes.

## Recognition-first denoising candidate

`MIMI_ASR_BENCH_DENOISE` accepts only `bypass` (the default) or `speex-mild`.
Every per-clip ASR report includes `input_processing`. Selecting `speex-mild`
filters fixed input PCM with the already linked SpeexDSP preprocessor, using
20 ms frames, mild -12 dB noise suppression and no speech gate. Its one-frame
overlap is flushed and startup delay compensated before paced upload; the
exact sample count, reference, language hint, model and protocol stay the same.
This is test-only and does not enable denoising in the application.

An explicit offline CPU evaluation is available without credentials or network:

```sh
MIMI_ASR_BENCH_MANIFEST=/path/to/public-corpus/manifest.json \
cargo test --manifest-path src-tauri/Cargo.toml --lib \
  audio3_benchmark::denoising::manual_public_corpus_cpu_comparison \
  -- --ignored --exact --nocapture
```

`DENOISE_BENCH_JSON` reports public fixture labels, sample counts and numeric
timings only. Frame CPU time, 20 ms overlap delay and up to 20 ms frame assembly
are different boundaries. Offline preprocessing is completed before the ASR
clock starts; its CPU result is not capture-to-visible latency. Recognition
acceptance still requires an authorized, completed bypass/candidate ASR pair.

An optional clip `preparation` field accepts only `raw` (default),
`resample-roundtrip`, or `deepfilter12`. Every report includes
`fixture_preparation`, independently of `input_processing`. Prepared fixtures
must use in-benchmark `bypass`; combining them with Speex fails before credentials
are read or a provider connection starts. Preparation labels identify a supplied
fixture, not proof that its external processor was implemented correctly.

Reference evaluation reports substitutions, insertions and deletions in addition
to total edit distance. It retains only the bounded public reference and two
numeric rows, without saving recognized text. Ties between minimum alignments
prefer fewer deletions, then insertions; reference edits are not semantic proof
of capture loss. Actual candidate regressions and preparation/timing limits are
recorded in [the denoising results](../development/2026-10-03-asr-denoising-results.md).
