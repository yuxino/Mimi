# Extreme repetition in the subtitle overlay

## Evidence and scope

A user screenshot from Alibaba realtime translation shows a short phrase
repeated across nearly the whole overlay. Current Audio3 recognition replaces
the cumulative source draft. Qwen-MT Lite returns incremental chunks; the HTTP
client accumulates one response and publishes whole replacement snapshots.
Retries create fresh decoders. This audit does not establish which provider
stage produced the screenshot's repeated text.

The follow-up language audit confirms that new Alibaba sessions use Audio3
plus Qwen-MT Lite. Selecting Korean sends `language_hints: ["ko"]` to Audio3
and `source_lang: "Korean"` to MT. Source text is decoded only from Audio3's
`sentence.text`; the independent MT result supplies the translation. The
Audio3 event language is derived from the selected language, rather than a
server detection result. The overlay language heading therefore does not
establish the recognized text's language. Confirmed history also survives
a language switch. No source/translation field swap, translated-text feedback
to ASR, or missing Korean parameter was found. A replay of the affected audio
is still required to distinguish an upstream recognition error from retained
history; display protection is not a recognition-quality fix.

An independent component fixture reproduces an application defect: atomic
bilingual previews display both lanes when their trimmed text is identical,
although confirmed pairs already show that text once. Apply the same exact
equality rule to previews, while retaining their original owner and keeping
Translation mode visible. Distinct utterance IDs remain separate history rows.

## Display protection

Compact overlay lanes currently accept a repetitive provider sentence within
the existing 64 KiB text bound. A separate component fixture reproduces this
screen-filling case. Fold only exact consecutive runs within a single lane:
at least twelve repetitions, at least 120 Unicode code points in the repeated
run, and a unit no longer than 32 code points. Show a representative unit and
an explicit `×N` count; preserve surrounding text. No similarity matching,
script filtering, provider prompt change, or cross-utterance deduplication.

The helper operates only on compact presentation. The accessible name, snapshot,
recognition input, translation input, confirmation identities and private file
exports retain their original text. Existing Home/upward reading opens the
complete sentence, including a live draft; End/return-to-live restores compact
presentation. Two genuine repeated sentences retain two independent rows.
Ordinary short repetition remains unchanged. Folding makes no claim that a
provider sentence is erroneous and does not improve recognition accuracy.

The implementation scans bounded candidate periods over Unicode code points,
without unbounded regex backtracking or quadratic searches. Memoize each
compact lane's projection by the input text; normal row-size observers continue
to own reflow when the folded display changes.

## Verification

- Reproduce identical atomic preview lanes and extreme repetition before fixing.
- Cover exact/whitespace-equal previews, Translation mode and distinct IDs.
- Cover multilingual and supplementary Unicode units, threshold boundaries,
  corrections, multiple separate runs, long/near-matching phrases and bounded
  64 KiB inputs.
- Exercise original/translation/bilingual, live/confirmed/history, full reading
  and returning to compact display with complete accessible text preserved.
- Run the canonical repository check and inspect the signed development UI.
  Synthetic display fixtures cannot establish the screenshot's upstream cause
  or a real-session recognition-quality improvement.

The two exact-equality model fixtures and five repetition component fixtures
failed before their respective fixes. The final canonical check passes with
800 Rust tests (one ignored), 750 frontend tests in 78 files, strict clippy,
formatting, typecheck, lint and production build. The existing SoftwareUpdate
Fast Refresh lint warning remains. Independent review prompted a further
delimited-unit boundary regression for a partial prefix and tail; that fix is
included in the final check.

The canonical signed development app was rebuilt and installed with its
unchanged designated requirement; strict signature verification passes. The
formal app was not replaced. With the user's authorization, the formal app
was quit, the UI-only development settings window was inspected, and the
development app was then restarted in normal mode. The normal native overlay
opened successfully. Native acceptance of the synthetic folding and full
reading cases remains pending; opening the app does not establish those cases
or reproduce the reported recognition error. The automatic checks above
establish display behavior with synthetic data, not provider quality.
