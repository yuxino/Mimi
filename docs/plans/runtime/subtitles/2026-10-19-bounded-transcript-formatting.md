# Bounded subtitle formatting and history serialization

## Scope

Baseline: clean main `1846507c`. Continue offline synthetic investigation while
the user uses the computer. Do not open native windows, capture audio, access
private settings/history or replace installed applications.

## Evidenced cost

The Gemini display formatter tests the entire accumulated output against its
abbreviation expression at each eligible ASCII period. A 65,535-character
synthetic `A. ` draft passes 715,816,960 characters through that check. Earlier
linear closing-punctuation scans do not bound this separate prefix scan.

Limit abbreviation matching to the suffix needed by the existing expression,
including the preceding word-boundary context. Do not change the abbreviation
set, punctuation, Unicode handling, line-length policy, raw transcript or saved
history. Avoid producing an entire new prefix merely to extract its suffix.

## Verification

Prove the existing implementation fails a deterministic linear-work budget;
then rerun the same case with the bounded implementation. Compare output with
the previous formatter for abbreviations, suffix lookalikes, Unicode case and
word boundaries, quotes, newlines and repeated punctuation. Keep existing
regressions. Independently review semantics and run the affected frontend
checks followed by complete frontend validation and CI.

Synthetic operation counts and local JavaScript timings describe only the
formatter. They do not establish native frame rate, real provider quality or
long-session process memory. Native visual acceptance is deferred to a time
when it can be performed without disturbing the user.

## Finalized history writes

Finalization serializes saved JSON directly into a temporary file. Serde emits
small JSON fragments separately, so a synthetic 10,000-entry / 2 MB text payload
produces 320,039 underlying writes for a 2,700,119-byte file. An 8 KiB writer
reduces that to 334 while preserving every output byte. The temporary-file,
sync and persist sequence is unchanged in the comparable measurement.

Buffer only finalized JSON serialization with a fixed 8 KiB allocation. Flush
explicitly and propagate errors before syncing or persisting the temporary
file. Leave incremental append rollback, WAV conversion, lock ownership,
permissions and retention unchanged. Cover successful round-trip data,
bounded underlying writes, and failures first surfaced by the final flush.
Run the history regressions and the no-argument canonical check because this
round now spans frontend and desktop storage.
