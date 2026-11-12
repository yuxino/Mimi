# Bounded history reading and export

Saved sessions currently load every subtitle before returning one page or
validating an audio track. File export also holds the complete formatted output.
Read the same private formats incrementally instead of keeping those copies.

- Parse the first-line header and its `entries` array with a serde seed. Visit
  one pair at a time; keep only metadata and the requested 30-entry page.
- Preserve finalized JSON and interrupted JSONL, field/identity validation,
  physical confirmation order, original indexes, case-insensitive queries and
  page clamping. Existing header entries remain readable beyond the append cap.
- A fixed 4 KiB line reader validates UTF-8 across buffer boundaries. Complete
  records fitting in that block use serde's slice reader; larger records keep
  the streaming reader. Malformed JSON
  journal lines are skipped; invalid UTF-8 stops subsequent recovery. No growing
  serialized-line buffer or complete subtitle vector is needed for these reads.
- Keep the newest page in a 30-entry ring during one scan. For later pages,
  count matches and then select the requested page under the existing I/O lock.
  A second pass must agree with the first pass's original header metadata and
  recovered entry count. A newly interrupted read returns an error instead of
  publishing a shorter export, returning an incomplete page, or replacing and
  consuming the source journal. Finalization's end-time/limit overrides are
  output fields and do not replace that comparison baseline.
  Export through a shared per-pair formatter; private file export flushes and
  syncs a temporary file before publishing it. Byte-returning APIs retain only
  their required output. Audio reads validate metadata without parsing a full
  owned transcript. Capture, source selection and recording policy stay intact.
- Preserve the bounded `AudioRecording` used by synthetic native UI/export
  fixtures; production session audio continues to use private files.

This bounds retained captions, not every allocation to a fixed byte count.
The requested 30 captions, the current pair and serde's largest string scratch
buffer still depend on text length. Byte-returning exports necessarily retain
their formatted output. Finalization keeps the existing compact JSON schema.

The trade-off is additional parsing work, especially the second scan for later
pages and exports. Desktop `session_export` dispatches these reads through
`spawn_blocking`; Android's `SavedSessionsViewModel` uses its single worker
executor. Neither path runs history parsing on the UI thread. This change is
for memory bounds; it does not claim faster history reads or native UI/RSS proof.

Compare the same private synthetic workload before and after: allocations,
peak live bytes and elapsed time for paging, export and audio reads, plus exact
output/recovery equivalence. Keep those measurements distinct from native RSS
or provider latency. Run focused Rust and actual JNI checks before publishing.
