# Opt-in local session history

The export page is a reading surface for current and past sessions. Past
sessions are grouped by local calendar day (Today, Yesterday, then dated
groups); selecting one shows confirmed source and translation pairs, newest
first, with search and 30-entry pages. The neutral layout follows Mimi's
settings design rather than the reference app's data table. Timestamps are
final-confirmation times, not speech onset or WAV playback positions.

The Keep subtitles and Record session audio preferences remain off by
default. Enabling either before a new session saves that content to local app
data when the session stops, including a normal app quit. The two data types
can be enabled independently; no third history switch is added. This changes
the earlier memory-only export design. During a live session, accepted pairs
and PCM chunks go directly to private local journals, bounded to 10,000 pairs
or 2 MiB and 64 MiB of PCM. The session manager keeps only counts, byte sizes,
limits, and sample rate in memory. The existing bounded overlay display still
holds its visible recent lines; it is not the export or history source.
Explicit TXT and WAV exports read the local files. Input selection and source
identity follow the [independent audio input design](../../audio/2026-10-03-optional-microphone-input.md).

History uses one private JSON file per completed session and an optional WAV
per recorded source in the app data directory. Live sessions use append-only
JSONL and separate PCM files, which are readable after an interrupted process exit. UUID
filenames prevent path traversal. Final writes use private temporary files,
sync, and no-clobber commit; the JSON commit replaces the journal as the
authoritative session. Only the settings window has IPC
permission to list, search, play and delete saved sessions. Audio is loaded on
demand for playback, and the resulting object URL is revoked when selection
changes. Deletion removes both text and audio. Current-session journals clear
on opt-out or explicit clear; new sessions and normal exit finalize them.
Saved history has its own explicit delete action. A forced OS failure can lose
the last unsynced journal writes; normal stop or quit syncs the final files.

Core tests cover retained-pair paging and search. File tests cover session
save, list, read, delete and invalid IDs. Verify opt-in defaults, transitions,
history grouping, stale page responses, empty states, deletion, audio playback,
and light/dark layout in the signed development bundle. Real capture, audio
timing and distribution behavior require separate device testing.
