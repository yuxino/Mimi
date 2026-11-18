# Real-audio comparison: release remains held

The user requested more real material after the Queen concert reproduction,
including investigation of Chinese appearing in the original-language lane.
No release, tag, merge or public package is authorized by this comparison.

## Fixed public corpus

The speech corpus uses original human recordings, not synthesized speech.
Temporary PCM, audition WAV and public references remain outside the repository.
Each audition WAV contains exactly the PCM sent by the direct-ASR benchmark;
metadata records source revision, sample ID, licensing and SHA-256 hashes.

| Sample | Selection | Duration | Reference |
| --- | --- | ---: | --- |
| FLEURS English | en_us validation row 48, ID 1527 | 23.44 s | Official raw transcript |
| FLEURS Japanese | ja_jp validation row 51, ID 1527 | 20.94 s | Official raw transcript |
| Tears of Steel dialogue | Original soundtrack, 00:23–01:04 | 41.00 s | Official English subtitle cues 1–11 |
| FLEURS Mandarin | cmn_hans_cn validation row 0, ID 1579 | 18.38 s | Official raw transcript |
| Second FLEURS English | en_us validation row 1, ID 1620 | 16.38 s | Official raw transcript |

The two additional FLEURS samples were selected by scanning from row zero and
taking the first recording lasting 15–30 seconds, before observing ASR results.
FLEURS is [CC BY 4.0](https://huggingface.co/datasets/google/fleurs), collected as
[native-speaker recordings](https://arxiv.org/html/2205.12446). Tears of Steel is
[CC BY 3.0](https://mango.blender.org/about/), obtained from the
[Xiph test-media mirror](https://media.xiph.org/tearsofsteel/).

An independently prepared 45-second excerpt of the previously user-authorized
local song uses identical PCM for English-hint and automatic-language runs.
Its original licensing source has not been independently reverified here; no
public redistribution or transcript is included, and no WER is claimed for it.
The user-provided Queen concert remains the original native reproduction; this
different song does not substitute for Queen acceptance.

## Native baseline

The signed canonical development app ran the English, film and Japanese samples
through actual system-output capture, Audio3 ASR, Qwen-MT and the overlay.
The concert browser was paused before playing the fixture WAVs. The runs used
explicit source-language hints, Chinese target and bilingual display. Capture
and text/audio recording preferences were not expanded. Only the existing
content-free diagnostic path was enabled.

| Sample | ASR server finals | Completed MT preview/final | Preview request median | Final request median | Maximum pending finals |
| --- | ---: | ---: | ---: | ---: | ---: |
| English | 2 | 14 / 2 | 261 ms | 335 ms | 1 |
| Film dialogue | 4 | 19 / 4 | 275 ms | 259.5 ms | 1 |
| Japanese | 1 | 9 / 1 | 341 ms | 493 ms | 1 |

These windows had zero observed 429 responses, audio-queue overflow or recovery.
Inspected final screens retained original and translated text in their proper
lanes without the old confirmed-preview duplicate. The film includes recognition
and translation inaccuracies; these runs are not a general accuracy pass.
Request times exclude recognition and initial pacing/queue waits. They are not
audio-to-visible latency measurements. Following-window clipping of older rows
is distinct from deleting their stored text.

## Source-language investigation

A source audit found no MT-to-source assignment in the current Audio3 path:
the decoder reads `payload.output.sentence.text` into source events; HQ keeps
that text as the source while storing the MT result separately; reducer and
three-mode projection preserve these fields. Same-language bypass copies source
to translation, and automatic mode reports no detected language, so that bypass
does not activate solely because an automatic-language draft contains Chinese.

The direct benchmark independently observed Han characters in intermediate ASR
responses for an English recording even with the English hint. This path has no
MT or frontend. Thus at least this class of transient wrong-language original
originates in recognition. The completed English result in those runs contained
no Han characters. A language hint is not an observed language-detection result.
Draft character totals count every replaceable response and can count the same
character repeatedly; they are neither unique error counts nor a language ratio.
Japanese naturally contains Han characters and must not be classified as Chinese
on that basis. Hiding all Han characters would damage valid Japanese/mixed speech.

## Comparison method and limits

The [test-only benchmark](../plans/development/verification/2026-10-02-audio3-repeatable-benchmark.md) sends
each fixed PCM in real time, with the same two-second tail, endpoint, account and
proxy. The three arms isolate current 3.0/context, 3.0/no context and 3.1/no
context. All calls are serial; the native provider session is paused during them.
The credential loader is reused without exposing the key or adding a production
credential path. No recognized transcript is saved; online edit-distance keeps
the public reference and numeric rows, with one bounded replaceable draft.

WER/CER here are reference edit rates after lowercasing/removing punctuation,
not semantic accuracy scores. Proper names, number forms and Japanese spelling
remain differences. Film subtitles are not exact word timing or exhaustive
transcripts. Service sentence-end delay is receipt time minus the service's
audio-clock end, not UI latency. No parameter or model change is accepted merely
because one short example improves.

## First controlled round

All fifteen clip/arm runs received genuine task completion, with zero failed
runs. Maximum observed sending lateness across the complete round was 21 ms.
Every arm received identical fixture bytes and the same two-second tail.

| Reference edit count / units | 3.0 current context | 3.0 no context | 3.1 no context |
| --- | ---: | ---: | ---: |
| English, ID 1527 (words) | 5 / 28 | 5 / 28 | 6 / 28 |
| Japanese, ID 1527 (characters) | 21 / 89 | 8 / 89 | 8 / 89 |
| Film dialogue (words) | 14 / 79 | 13 / 79 | 17 / 79 |
| Mandarin, ID 1579 (characters) | 0 / 44 | 0 / 44 | 0 / 44 |
| English, ID 1620 (words) | 4 / 33 | 4 / 33 | 4 / 33 |

The first English sample produced three cumulative Han-character observations
in replaceable drafts in each arm, and zero Han characters in completed results.
The film also produced transient Han observations (three, three, one); completed
results contained no Han characters. Removing the existing context therefore
does not, by itself, eliminate transient wrong-language drafts. It improved this
Japanese sample's reference match; broader/reversed-order comparison is required
before attributing a general recognition benefit. The 3.1 arm did not show a
consistent advantage and is not promoted to the production model.

The six song runs (same 45-second PCM, English/automatic × three arms) also
completed. Each produced eight cumulative Han-character observations in drafts
and zero in completed results. First nonempty responses occurred around 25.4 s
in this excerpt, which includes a long instrumental introduction; this number
must not be described as recognition latency. Final counts/text lengths differed
between arms, but there is no exact reference and thus no defensible song WER or
accuracy ranking. Specifying English did not eliminate these transient drafts.

## Alternate realtime protocol

An explicit fourth arm tested `qwen3-asr-flash-realtime-2026-02-10` through the
separate realtime API, with server VAD (threshold 0.2, silence 800 ms). This is
an alternate protocol/configuration comparison, not an isolated model swap.
One smoke run and seven corpus/song runs all reached genuine session completion.
Together with the earlier rounds, 29 direct-ASR sessions completed.

| Reference edit count / units | Realtime alternative |
| --- | ---: |
| English, ID 1527 (words) | 9 / 28 |
| Japanese, ID 1527 (characters) | 21 / 89 |
| Film dialogue (words) | 10 / 79 |
| Mandarin, ID 1579 (characters) | 0 / 44 |
| English, ID 1620 (words) | 5 / 33 |

The English and film runs produced no Han characters in either drafts or finals.
Their stable prefixes were not observed retracting, but their first nonempty
stable prefixes arrived 4.2–6.5 seconds after PCM playback began. Replaceable
drafts arrived earlier; clip-relative offsets include silence and are not
normalized speech-onset latency. The Japanese and Mandarin offsets are not
directly comparable to English. Stable-prefix retraction counters retain only
the latest item, so interleaved older-item changes could be missed.

Both 45-second song runs (English and automatic) completed without returning
any draft or final text, while Audio3 returned text for the same bytes. Zero
wrong-script observations in an empty response are not an accuracy improvement.
The film reference match improved, but both English references and the Japanese
reference did not improve over 3.0/no-context. No alternate configuration is
promoted to the production default. In particular, hiding replaceable drafts
until a stable prefix arrives would introduce a substantial responsiveness
trade-off and would not address the empty song results.

Protocol semantics are documented in Alibaba's
[realtime server events](https://help.aliyun.com/en/model-studio/qwen-asr-realtime-server-events)
and [client events](https://help.aliyun.com/en/model-studio/qwen-asr-realtime-client-events).
The benchmark keeps source text private and reports only character classes,
counts, edit distances and timing; it does not save or reproduce song lyrics.

## Confirmed application defect addressed

A separate local-WebSocket fixture proved that already-captured real PCM could
still be waiting in the send pipeline when Audio3's independent idle timer sent
an extra 100 ms of synthetic silence. The native pipeline now binds a shared
pending-PCM counter to Audio3. A guard travels with each accepted buffer until
send completion, failure or cancellation, so this known waiting window no
longer looks idle. Real bytes, queue capacity and normal order are preserved;
genuinely drained input retains keepalive behavior. See the
[bounded fix and its limits](../plans/providers/alibaba/2026-10-02-audio3-pending-pcm-heartbeat.md).
This test establishes an application bug, but not that it caused the Queen
recognition failure. Capture callbacks that have not arrived are outside the
counter's guarantee.

## New signed development app acceptance

The complete repository check passed with 801 Rust tests (one ignored) and
698 frontend tests in 76 files, including strict clippy, formatting, typecheck,
lint and production build. The optional benchmark's 18 local tests and its
feature-enabled strict clippy also passed; paid provider runs are explicit.
The only installed development bundle was rebuilt at `/Applications/mimi-dev.app`
with the same designated requirement. The formal application was not replaced.

The new bundle played the same English and film WAVs through system capture,
then paused and resumed before a different human English recording. Original and
translation remained separate in the inspected bilingual screens, and resume
produced new actual recognition/translation, not just a changed button.

| Native run after fix | ASR finals | MT preview/final completions | Preview median / max | Final median / max |
| --- | ---: | ---: | ---: | ---: |
| English ID 1527 | 2 | 13 / 2 | 281 / 342 ms | 320 / 322 ms |
| Film dialogue | 5 | 19 / 5 | 261 / 403 ms | 285 / 363 ms |
| English ID 1620 after resume | 1 | 9 / 1 | 265 / 363 ms | 485 / 485 ms |

All three windows had zero observed 429 responses, MT failures, audio-queue
overflow, send failures or recovery attempts; maximum waiting final depth was
one. These request times exclude ASR and initial scheduling. The native runs
did not deliberately stall the sender, so the deterministic local-WebSocket
regression, not these timings, proves the pending-PCM ordering guarantee.
Recognition/translation mistakes remain visible in natural material; neither
these checks nor the audio fix resolve the concert's overall quality problem.

Testing stopped the session and restored automatic recognition, Chinese target,
translation-only display, divider off, 17 px white centered text, both motion
preferences on, immersion/position lock off and the existing system proxy.
The diagnostic app exited, then the canonical development app was reopened
normally. Public video playback remains paused and unmuted for later testing.

## Remaining release blockers

- Wrong-language and revising ASR drafts, especially songs/mixed backgrounds,
  are reproduced but not eliminated. No hard script filter or unproven model
  switch is installed.
- Queen recognition quality has not passed acceptance. This round adds real
  corpus evidence and a proven transport fix; it does not certify all prior
  UI/device/signing scenarios or every supported language.
- The release hold remains in effect. The benchmark does not ship in the app;
  no new release, tag or merge is part of this work.
