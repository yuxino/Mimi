# Batch delete saved sessions

Tracks GitHub issue [#416](https://github.com/yuxino/Mimi/issues/416).

## Goal

Let users delete multiple saved sessions at once—via multi-select, select all,
and clear all—on both Desktop and Android without risking accidental deletion of
an active/current session or hiding partial storage failures.

## Shared policy (`shared/mimi-runtime`)

- `SessionHistory::delete_many(ids, protected_ids) -> io::Result<HistoryDeleteReport>`
  is the single implementation for both Desktop and Android.
- It deduplicates requested IDs in order, never deletes any ID listed in
  `protected_ids`, delegates each deletion to `SessionHistory::delete` (removing
  transcript, optional `meta.json`, and any per-source audio tracks), and
  returns `HistoryDeleteReport { deleted, failed }`.
- When history storage is disabled, `delete_many` returns `NotFound` without
  touching files.

## Desktop and Android adapters

- **Desktop backend (`src-tauri/src/session_export.rs`)**:
  - `session_history_delete_many(ids)` passes `session.current_history_id()` as
    the protected set. `session_history_delete` also rejects the current session
    ID.
- **Desktop UI (`src/windows/settings/history/SessionExport.tsx`)**:
  - Normal library view stays uncluttered and adds compact **Select** and red
    **Clear all** actions in the saved-sessions header when saved sessions
    exist.
  - **Select** mode turns saved rows into checkable items (hiding the current
    session entry so it cannot be confused with saved sessions) and adds a
    selection bar with **Select all / Deselect all** (scoped to the currently
    filtered saved sessions), selected count, and **Delete (N)**.
  - **Clear all** snapshots the current saved session IDs at confirmation time
    so a session finalized while the modal is open is never silently deleted.
  - Both batch actions require confirmation via `SettingsConfirmation` with the
    exact session count. Partial failures remove succeeded items immediately,
    narrow the open dialog to retry only the failed IDs, and report sanitized
    counts through the shared settings toast.
- **Android JNI & UI (`shared/mimi-android-jni/src/history.rs`, `android/.../sessions/`)**:
  - JNI `history_delete_many` passes `active_ids(&directory)` as `protected_ids`
    and returns `{ deleted, failed }`.
  - `SavedSessionsActivity` and `SavedSessionsViewModel` expose **Select** and
    **Clear all** in the list view, **Select all / Deselect all**, **Done**,
    and **Delete (N)** in selection mode with native checkboxes, 48dp touch
    targets, Back-to-exit selection, exact count confirmation dialogs, and
    localized strings in all nine UI languages.
