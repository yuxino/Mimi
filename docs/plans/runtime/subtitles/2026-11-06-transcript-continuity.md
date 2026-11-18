# Continuous transcript pairing and source progress

## Evidence and scope

The user reported old subtitles after switching videos/languages, then missing
new originals while Chinese output continued. Production 1.6.16 diagnostics at
19:40:27 and 19:40:51 China time showed source updates fixed at 466, translation
updates 959 → 994 and confirmed pairs fixed at 24. Native observation showed
a short old original with a much longer new translation. There is no saved wire
trace establishing whether missing source output originated upstream or locally.
Continuous English/Japanese/Chinese switching and Chinese-to-Chinese behavior
are review cases. The user later authorized production saved credentials and,
after finishing computer use, native playback/control. Recording and transcript
retention stay off; credentials remain in their original private namespace.

The reported X case contains two music edits. Production-key replays showed the
second clip also returns almost no text on a fresh connection, while ordinary
speech immediately afterward returns both lanes. Ordinary speech also resumes
about 2.3 seconds after a 90-second silence. Missing text therefore cannot alone
justify reconnecting. The original general idle complaint remains unproven by
these bounded cases. See the integration run ledger for counts and limitations.

## Design boundaries

- Repair demonstrated local pairing errors in shared Rust. A frame timestamp is
  coarse alignment metadata, not an utterance identity; equal punctuation counts
  do not prove semantic correspondence. Never independently trim a joint coverage
  interval into two different sentence prefixes. Preserve replaceable drafts,
  full confirmed text, Unicode, repeated timestamps and bounded state.
- Retire a stale original when translated text continues without it: 20 seconds,
  eight translation updates and 64 characters, with no translation gap over five
  seconds. Silence and isolated late tails do not arm a failure timer. Keep the
  connection alive and the useful translation visible, bounded to 320 Unicode
  characters. Never confirm it against an old original. When source text returns,
  discard the orphan translation before starting a fresh block. Reset/Clear also
  retire this state. OpenAI and Azure share this rule; Gemini keeps its existing
  block behavior. The real music case rejected the initial hard-reconnect idea:
  optional ASR omission does not establish that the translation stream failed.
- Keep source metadata, local receipt time and provider frame time distinct.
  Do not turn a source omission into fabricated text or an unrelated final pair.
- Give OpenAI translation close acknowledgements a separate 15-second budget,
  with a 17-second outer provider bound. Production-key comparisons returned
  valid `session.closed` after 5.5 and 9.8 seconds; the previous two-second
  client deadline discarded those tails. Other providers retain their current
  bounds. Both native drivers use the client's bound, and Android's stop
  watchdog covers the longest finish plus drain/disconnect/publication. A
  seven-second delayed close fixture must retain its tail through actual JNI.
- Verify same-language passthrough before any independent MT request, including
  Chinese/English/Japanese changes, pending previews and late completions. Keep
  explicit language/script requirements; text containing Han characters alone
  does not identify Chinese. Integrated speech translation is generated remotely
  from audio; local deduplication does not disable its translation work. Do not
  silently change a user's selected provider, mode or credential requirements.
- Review Azure's shared committer, Gemini's whole-block checkpoints, xAI and
  DashScope identities, other integrated providers, and the independent ASR/MT
  routes. Apply a change only where the protocol and regression justify it.

## Verification

Start with deterministic synthetic regressions that fail before repair. Run direct
Rust, shared fixtures, actual JNI and affected adapter/socket tests; complete the
canonical checks after focused checks. Provider replays use verified public samples and explicitly selected saved
configuration; native playback is separately observed using the formal app. A provider result, automated pass and native display acceptance remain
separate claims. Retain content-free run evidence and preserve unrelated dirty
documentation. No public release is part of this task. A local formal-app replacement must pass
the complete designated-requirement comparison and retain its configuration.

Protocol reference:
[OpenAI translation events](https://developers.openai.com/api/reference/resources/realtime/translation-server-events).

Alternatives considered: matching sentence counts is insufficient when languages
segment differently; waiting for session end avoids local cuts but makes live
history unusable; joint coverage with conservative completion and bounded orphan previews keeps
useful output responsive without inventing sentence identities. In particular,
long music recognition gaps must not exhaust a healthy connection's retry budget.

Production-key Chinese/Japanese replays showed short unpunctuated output can
remain a replaceable preview through a clip. A ten-second timed checkpoint was
tried, then rejected by native mixed-language playback: aligned timestamps did
not prove the two lanes reached the same semantic boundary. Do not confirm short
unfinished text merely because ten seconds elapsed. Retain the conservative
existing punctuation/size behavior and continuous previews. A shared fixture
prevents reintroducing this time-only shortcut. Coarse provider timing and the
existing bounded-size checkpoints still do not establish exact sentence identity;
semantic alignment remains an explicit limitation, not a passed claim.

Further native Gemini playback reproduced a source-only Chinese block retained
through a long quiet interval and prepended to the next Japanese clip. Retire an
unpaired block when the next text arrives after at least 20 seconds without
either lane advancing. Preserve its visible preview during the wait and clear
both old previews at the next block; never invent a missing translation or infer
Chinese script from an undetected language. Delayed counterparts inside that
bound still pair normally. This conservative boundary does not identify rapid
video switches or repair upstream translation omissions.

Baidu production replays twice rejected valid zero PCM after about 25 seconds,
with code 20316 followed by END. Only after at least 20 seconds of successfully
sent all-zero PCM, route that rejection through the existing bounded transport
recovery policy. Keep qualifying silence evidence for five seconds after its last zero frame,
covering a delayed rejection just after speech resumes. Continuous silence
count resets on ordinary audio; disconnect clears all evidence. Expired evidence
cannot turn an ordinary rejection into recovery.
Setup rejection and invalid audio without this evidence remain fatal. Do not
ignore the error and keep a dead socket. Direct socket and actual JNI quiet /
reconnect / resumed PCM tests must cover this narrower classification.

The first formal-app Baidu replay still failed with bounded audio-queue
backpressure and exhausted recovery. Its sender scheduled each next 40-ms frame
from the previous write's completion, accumulating scheduler/socket overhead
against an independent real-time capture clock. Advance the existing scheduled
deadline instead; clamp to completion after a long stall to avoid storing burst
credit. Preserve frame sizes and queue bounds. A deterministic twenty-minute
cadence regression and the same native idle/resume replay are required; an
API probe that awaits each send before advancing input cannot prove this path.

The cadence repair removed native backpressure in the next comparable replay
(42 source/42 target updates, four confirmed captions). A second 20316 arrived
about 1.5 seconds after resumed sound, following another long quiet period on a
recovered connection. The five-second grace is local classification tolerance,
not a claimed upstream timing guarantee. Socket tests cover expired evidence;
actual JNI covers a rejection delayed until after resumed nonzero PCM.
