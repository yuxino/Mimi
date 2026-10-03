# Reproduce and compare a development case

1. In the signed development app, open Settings → Diagnostics → Development
   debugger. Start **audio + subtitle evidence** before starting capture.
   This explicitly saves private speech and subtitles locally. Metadata-only
   tracing has no audio or subtitle payloads.
2. Use non-sensitive speech. Confirm the selected input and provider. Reproduce
   the symptom, stop the subtitle session first so final provider frames drain,
   then stop evidence collection.
3. Load the sent-audio player for each selected input. Confirm by listening.
   The player contains PCM decoded from audio messages whose local socket send
   completed; a server receipt or model consumption is not proven by that.
   Check failed/cancelled sends, missing evidence and limits before drawing a
   conclusion. Capture PCM, queued client PCM and sent wire PCM are different.
4. Step through recorded backend snapshots in the replay inspector. Compare
   provider event character counts and admission, reducer changes, numbered
   snapshot publication, per-window store receipt/application, selected/stable
   text and DOM commit/clipping. Same-length replacement is still a change.
5. Export the case folder. `trace.json` and `trace-events.jsonl` are content-free.
   The latter contains the complete persisted metadata stream for a private
   case; its rows have the same `{id, elapsedMs, event}` shape as the live trace.
   Concurrent callbacks can reach disk out of order, so order the full stream
   by exact event ID when analyzing it. `snapshots.jsonl`,
   `events.jsonl` and sent WAV files contain explicitly saved private content.
   `events.jsonl` contains allowlisted recognition/provider text and prepared
   independent text-translation HTTP bodies, including the actual model,
   prompt and translation options. It excludes credentials, headers, endpoints
   and arbitrary error bodies. HTTP preparation does not prove server receipt.
   Saved local cases can be reopened in the dev inspector after restarting.
   Keep them local
   unless deliberately choosing to share them.
6. An AI analysis must cite trace event IDs and snapshot IDs, check all loss
   counters, identify the earliest divergent stage, and distinguish provider
   omission, backend rejection, preview replacement, frontend projection and
   clipping. Do not infer model reasoning or semantic accuracy from counts.
7. Replay snapshots offline to reproduce frontend projection. For provider
   reproduction, play exactly the saved sent WAV in a separate test session;
   this performs a new provider request and its output may vary. A deterministic
   provider result requires replaying recorded output instead.
8. Save a focused regression around the demonstrated boundary. Compare the
   same case before/after, then run the repository check and signed dev UI.

The live journal retains 2,048 events and the inspector shows the newest 240.
Private cases additionally persist metadata continuously and expose it in pages
of at most 64 rows. `trace.evicted` describes the live ring, not disk loss.
Check `tracePersistenceEnabled`, `persistedTraceEntries`, `traceDropped`,
`traceLimited` and `traceFailed` before using the complete metadata file. Old
cases without this file, and metadata-only traces, still have only their bounded
live journal; their eviction represents missing evidence. `contentDropped`
includes `traceDropped`, so do not add that subset twice. Content and metadata
stop at a shared 16 MiB per case; audio has a separate bounded recorder. Old
cases are not silently deleted. A DOM commit is not proof of native window
visibility or that every character was readable.

`visibleCharacters` counts submitted DOM text in lanes intersecting the
viewport; it is an upper bound, not a count of readable glyphs. A partly
scrolled history block or a bounded long-text lane can increase
`overflowedBlocks` without losing provider output or confirmed history.
Compare raw, selected and stable counts, saved snapshots, scroll position and
native appearance before attributing a clipping observation to swallowed text.

Snapshots, private events and persisted metadata share a 16 MiB cap and one
32-item nonblocking writer queue; each line is capped at 512 KiB. Stop records
the terminal metadata entry, disables collection, waits for already assigned
case-bound callbacks, then seals and drains the writer. A callback-drain timeout
leaves the case unsealed and reports an error so Stop can be retried.
Audio/index has a 20 MiB cap,
and a complete case reserves a further 4 MiB for its report. The local store
has a 128 MiB reservation limit and at most 64 cases. The same monotonic epoch
is used for trace, snapshots, private events and audio send timestamps.
Frontend journal time is backend batch receipt time (normally up to 50 ms
after observation), not an exact browser rendering timestamp. WAV playback
concatenates successful sends; index offsets and timestamps locate pauses,
failures and missing evidence. Snapshot stepping is projection replay, not a
simulation of original timers, provider behavior or native window geometry.
