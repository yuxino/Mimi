# UI-only native export fixture

At integration `197f165`, cloud Linux QA verified synthetic subtitles,
pause/resume and DeepLX advanced settings, but could not reach the native export
picker: stopping clears the UI-only archive, while export requires an inactive
session. This is a fixture gap, not evidence of production transcript loss.

Set both `MIMI_UI_TEST=1` and `MIMI_UI_TEST_EXPORT=1` to retain the already opted-in,
bounded synthetic transcript/audio buffers in memory after stop. No new IPC,
network, capture, history file or credential path is introduced. Production
history finalization is unchanged; the extra flag has no effect outside UI-only.
The existing start, clear and preference opt-out paths still reset the buffers;
normal UI-test finalization without the extra flag still clears them. Exiting
releases the in-memory fixture. This does not enable persisted saved-session
fixtures or seven-phase/streaming sequence injection.

## Acceptance on the exact CI package

The coordinator must first authorize the isolated Linux native QA window.
Use fresh temporary XDG_CONFIG_HOME/DATA_HOME/CACHE_HOME/RUNTIME_DIR and the
UI-only flags above; never reuse a user's profile or credential store. Keep
MIMI_AUTO_START=0. This note does not authorize Mac signing/install/launch.

1. In the recording/export settings enable subtitle retention, start the
   UI-only session, observe the explicitly synthetic bilingual pair, then stop.
2. Current-session count/text must remain visible through several refreshes.
   Open TXT export: the real native save picker should appear. Cancel and
   verify text/search/selection/page stay unchanged and no success is reported.
3. Save only to a test-owned temporary destination. Verify the TXT contains the
   marked synthetic pair. Exercise a failing destination inside the same test
   directory (for example a directory where a file is expected), then verify
   error feedback, retained text and successful retry. Never touch user files.
4. Repeat for opted-in synthetic WAV if needed; it is a generated test tone,
   not captured system audio. Clear, disable retention or start a new session
   and verify old content is not available. Without the extra export flag,
   stopping should still clear the UI-only buffers.
5. Record exact head, OS, artifact and executable SHA256, screenshots and result
   in #88. Do not mark native cancel/failure/save as passed before observation.

Two backend regressions cover retained transcript/page/WAV after stopped-state
and repeated fixture finalization, new-session/opt-out reset, plus clearing
without opt-in. Existing export file tests cover atomic replacement and failure
without destructive overwrite. Native dialog behavior remains a separate QA
requirement. No seven-state motion or real audio/provider claim follows.
