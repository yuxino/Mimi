# Queen live-song reproduction: release held

The user supplied the [Queen Live Aid concert](https://www.bilibili.com/video/BV1YdwgzkE9Z/)
after stopping the v1.5.6 release. PR #99 remains a draft. Earlier controlled
speech checks do not establish acceptance for this concert. No v1.5.6 merge,
tag, release or public assets were created. Publishing requires renewed user
authorization.

## Native baseline and reproducible defects

The signed development app ran from its single canonical macOS installation.
Playback was initially muted by the website; that portion is not recognition
evidence. Subsequent observations verified unmuted playback and nonzero system
audio ingress. The test used bilingual display to distinguish recognized
source from translated text, with automatic language and then explicit English.
Only timing, counts and sanitized states were logged; no lyrics or audio were
saved to the repository.

Two separate application defects were reproduced:

1. **Previous phrase collapsed to one line.** A completed English phrase was
   still complete in accessibility, but the next short phrase made the visible
   original shrink to its final word. `Timeline` forced every non-last row to
   one line per language even when the body had space. Rows now retain their
   viewport-bounded lane budget. Outer scrolling handles older rows; explicit
   reading still opens full text. It does not make all history unbounded.
2. **Confirmed preview remained alongside its final.** The screen showed a
   completed source/translation pair and its shorter previous preview together.
   The safe event trace places the final completion at 07:10:40.083 UTC and
   pause at 07:10:40.763 UTC. Pausing made the state stable to inspect; it did not
   create a fallback final. The reducer incorrectly used a newer raw-source ID
   to preserve an older preview. Raw source and complete preview now receive
   independent owner checks: final A clears preview A while preserving raw B;
   a real preview B survives. No text-prefix or similarity deduplication is used.

## Recognition remains unresolved

Automatic recognition switched between short Chinese and English output during
English singing. Explicit English improved parts of the baseline but did not
make the concert reliably accurate. The inspected request uses the configured
English hint, the existing Audio3 model and the expected PCM16 mono 16 kHz
format. This establishes neither a model-quality guarantee nor an application
cause for every recognition error.

A controlled VAD experiment used semantic segmentation off, 800 ms silence and
multi-threshold segmentation. It did not demonstrate acceptable earlier
confirmation or better recognition in the concert and was **reverted**. The
shipping encoder remains unchanged. See the [rejected comparison](../plans/development/verification/2026-10-02-audio3-vad-comparison.md)
for official parameter contracts and limitations. Seek/start timing and
nondeterministic recognition mean these observations are qualitative, not a WER
benchmark or proof that the parameters caused the mistakes.

The semantic baseline automatic session completed 13 MT requests, median 231 ms,
maximum 345 ms, with one server final before stopping. The explicit-English
baseline completed 31 requests, median 253 ms, maximum 2578 ms, with six server
finals. The longer rejected VAD session completed 32 requests, median 228 ms,
maximum 443 ms, with three server finals and maximum pending final depth one.
No observed 429 or audio overflow occurred in these samples. These are different
session windows, not normalized throughput comparisons. Request duration excludes
initial recognition and scheduling delay; it is not audio-to-screen latency.

## Focused verification and limits

- A three-mode component regression first reproduced the previous phrase's
  three-lines-to-one collapse, then passed while retaining the same row DOM.
  Small-window bounds, reading, End and return-to-live behavior remain covered.
- A pure event regression first reproduced preview A + raw B (empty and
  nonempty) + final A leaving A visible. All 34 reducer tests pass after the
  owner fix, including preservation of a real preview B on late final A.
- During the experimental native build, the corrected layout retained two
  original lines and two translated lines when the next short phrase arrived.
  This verifies the layout behavior, not the withdrawn VAD parameters or later
  reducer fix.
- The final canonical check passes: 775 Rust tests, one ignored, and 698 frontend
  tests in 76 files, plus formatting, strict clippy, lint, types, production build,
  installer safety and diff checks. Independent cross-review found no blocker in
  the two narrow changes.
- The final signed development build restores the original recognition
  parameters and includes both fixes. A subsequent concert run from about
  03:06 to 05:12 retained two original and two translated lines when a new
  phrase arrived. Three server finals completed; after pause, no confirmed
  pair plus its old preview remained in the inspected screen. Three-mode
  switching through Settings retained the selected-language text. The exact
  A/B event interleaving is proved by the regression, not by content-free native
  logs, which do not expose the sentence owners.
- That final run completed 38 MT requests, median 245.5 ms, maximum 327 ms,
  maximum pending-final depth one, with zero observed 429 or audio overflow.
  Recognition still produced incorrect or heavily revised song fragments, so
  this is not a recognition-quality pass.
- Playback was paused, the session stopped, and testing preferences restored to
  automatic source, Simplified Chinese and translation-only display. A normal
  restart without diagnostic flags confirmed session off and preserved display
  preferences. One canonical development process remains; the formal app was
  not replaced. The concert's recognition quality remains a release blocker;
  automated green checks alone do not clear it.
