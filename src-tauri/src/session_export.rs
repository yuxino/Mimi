//! Settings-only manual export: native save picker, then atomic file replacement.
use crate::commands::AppState;
use crate::core::audio::audio_input::AudioSource;
use crate::core::lifecycle::session_archive::{
    ArchiveState, TranscriptExportFormat, TranscriptExportOptions, TranscriptPage,
};
use crate::session_history::{
    classify_delete_error, HistoryDeleteFailureReason, HistoryDeleteReport, HistoryItem,
};
use serde::Deserialize;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

static EXPORT_BUSY: AtomicBool = AtomicBool::new(false);
#[derive(Debug)]
struct ExportGuard(&'static AtomicBool);

impl ExportGuard {
    fn acquire(busy: &'static AtomicBool) -> Result<Self, String> {
        busy.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| Self(busy))
            .map_err(|_| "An export is already open.".into())
    }
}

impl Drop for ExportGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

async fn run_export_worker<T: Send + 'static>(
    busy: ExportGuard,
    lifecycle: Option<tokio::sync::OwnedMutexGuard<()>>,
    work: impl FnOnce() -> T + Send + 'static,
) -> tauri::Result<(ExportGuard, T)> {
    tauri::async_runtime::spawn_blocking(move || {
        // A cancelled command cannot stop an in-flight disk operation. Keep
        // its locks with that operation, then return only export exclusivity
        // so the native picker never blocks session or preference changes.
        let _lifecycle = lifecycle;
        let result = work();
        (busy, result)
    })
    .await
}

fn export_picker_callback<T: Send + 'static>(
    busy: ExportGuard,
    reply: tokio::sync::oneshot::Sender<(ExportGuard, T)>,
) -> impl FnOnce(T) + Send + 'static {
    move |path| {
        // The native dialog outlives a cancelled command. Keep exclusivity
        // until its callback runs or is dropped, then hand it to the writer.
        let _ = reply.send((busy, path));
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExportKind {
    Transcript,
    Audio,
}

#[tauri::command]
pub fn session_archive_state(state: State<'_, AppState>) -> ArchiveState {
    state.session.archive_state()
}

#[tauri::command]
pub async fn session_transcript_page(
    state: State<'_, AppState>,
    query: String,
    page: usize,
) -> Result<TranscriptPage, String> {
    if query.chars().count() > 120 {
        return Err("Search is too long.".into());
    }
    let session = std::sync::Arc::clone(&state.session);
    tauri::async_runtime::spawn_blocking(move || session.transcript_page(&query, page))
        .await
        .map_err(|_| "Could not read current session.")?
        .map_err(|_| "Could not read current session.".into())
}

#[tauri::command]
pub async fn session_history_list(state: State<'_, AppState>) -> Result<Vec<HistoryItem>, String> {
    let history = state.session.history();
    list_saved_history(
        async move {
            tauri::async_runtime::spawn_blocking(move || history.list())
                .await
                .map_err(|_| "Could not read session history.")?
                .map_err(|_| "Could not read session history.".into())
        },
        || state.session.current_history_id(),
    )
    .await
}

async fn list_saved_history(
    read_history: impl std::future::Future<Output = Result<Vec<HistoryItem>, String>>,
    current_history_id: impl FnOnce() -> Option<String>,
) -> Result<Vec<HistoryItem>, String> {
    let mut items = read_history.await?;
    // Start may append a new current session while the file read is queued;
    // Stop may finalize the old one. Filter against the owner at completion.
    let current_id = current_history_id();
    items.retain(|item| Some(&item.id) != current_id.as_ref());
    Ok(items)
}

#[tauri::command]
pub async fn session_history_page(
    state: State<'_, AppState>,
    id: String,
    query: String,
    page: usize,
) -> Result<TranscriptPage, String> {
    if query.chars().count() > 120 {
        return Err("Search is too long.".into());
    }
    let history = state.session.history();
    tauri::async_runtime::spawn_blocking(move || history.page(&id, &query, page))
        .await
        .map_err(|_| "Could not read session history.")?
        .map_err(|_| "Could not read session history.".into())
}

#[tauri::command]
pub async fn session_history_audio(
    state: State<'_, AppState>,
    id: String,
    audio_source: Option<AudioSource>,
) -> Result<tauri::ipc::Response, String> {
    let history = state.session.history();
    let audio =
        tauri::async_runtime::spawn_blocking(move || history.audio_selected(&id, audio_source))
            .await
            .map_err(|_| "Could not read session audio.")?
            .map_err(|_| "Could not read session audio.")?;
    Ok(tauri::ipc::Response::new(audio))
}

#[tauri::command]
pub async fn session_archive_audio(
    state: State<'_, AppState>,
    audio_source: Option<AudioSource>,
) -> Result<tauri::ipc::Response, String> {
    let session = std::sync::Arc::clone(&state.session);
    let audio = read_stopped_audio(state.session.settings_mutation_guard(true), move || {
        session.export_audio_from(audio_source)
    })
    .await?;
    Ok(tauri::ipc::Response::new(audio))
}

async fn read_stopped_audio(
    lifecycle: impl std::future::Future<Output = Result<tokio::sync::OwnedMutexGuard<()>, String>>,
    read: impl FnOnce() -> std::io::Result<Option<Vec<u8>>> + Send + 'static,
) -> Result<Vec<u8>, String> {
    let lifecycle = lifecycle
        .await
        .map_err(|_| "Stop live subtitles before playing a recording.")?;
    tauri::async_runtime::spawn_blocking(move || {
        // Keep Start/clear/opt-out serialized with the actual disk read even if
        // the window closes and cancels its request while the worker is running.
        let _lifecycle = lifecycle;
        read()
    })
    .await
    .map_err(|_| "Could not read current session audio.")?
    .map_err(|_| "Could not read current session audio.")?
    .ok_or_else(|| "No current session audio is available.".into())
}

#[tauri::command]
pub async fn session_history_delete(state: State<'_, AppState>, id: String) -> Result<(), String> {
    if state.session.current_history_id().as_deref() == Some(id.as_str()) {
        tracing::warn!(
            reason = HistoryDeleteFailureReason::ActiveSession.code(),
            "session history delete blocked"
        );
        return Err(HistoryDeleteFailureReason::ActiveSession.code().into());
    }
    let history = state.session.history();
    let result = tauri::async_runtime::spawn_blocking(move || history.delete(&id))
        .await
        .map_err(|_| {
            tracing::warn!(
                reason = HistoryDeleteFailureReason::IoError.code(),
                "session history delete worker failed"
            );
            HistoryDeleteFailureReason::IoError.code().to_owned()
        })?;
    result.map_err(|error| {
        let reason = classify_delete_error(&error);
        tracing::warn!(
            reason = reason.code(),
            error_kind = ?error.kind(),
            "session history delete failed"
        );
        reason.code().into()
    })
}

#[tauri::command]
pub async fn session_history_delete_many(
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> Result<HistoryDeleteReport, String> {
    let history = state.session.history();
    let session = std::sync::Arc::clone(&state.session);
    delete_saved_history_many(history, ids, move || session.current_history_id()).await
}

async fn delete_saved_history_many(
    history: std::sync::Arc<crate::session_history::SessionHistory>,
    ids: Vec<String>,
    current_history_id: impl FnOnce() -> Option<String> + Send + 'static,
) -> Result<HistoryDeleteReport, String> {
    let result = tauri::async_runtime::spawn_blocking(move || {
        let protected: Vec<String> = current_history_id().into_iter().collect();
        history.delete_many(&ids, &protected)
    })
    .await
    .map_err(|_| {
        tracing::warn!(
            reason = HistoryDeleteFailureReason::IoError.code(),
            "session history batch delete worker failed"
        );
        HistoryDeleteFailureReason::IoError.code().to_owned()
    })?;
    match result {
        Ok(report) => {
            if !report.failed.is_empty() {
                tracing::warn!(
                    deleted = report.deleted.len(),
                    failed = report.failed.len(),
                    reason = report
                        .failure_reason
                        .map(HistoryDeleteFailureReason::code)
                        .unwrap_or("unknown"),
                    "session history batch delete incomplete"
                );
            }
            Ok(report)
        }
        Err(error) => {
            let reason = classify_delete_error(&error);
            tracing::warn!(
                reason = reason.code(),
                error_kind = ?error.kind(),
                "session history batch delete failed"
            );
            Err(reason.code().into())
        }
    }
}

#[tauri::command]
pub async fn session_history_rename(
    state: State<'_, AppState>,
    id: String,
    title: String,
) -> Result<(), String> {
    let history = state.session.history();
    tauri::async_runtime::spawn_blocking(move || history.rename(&id, &title))
        .await
        .map_err(|_| "Could not rename session history.")?
        .map_err(|_| "Could not rename session history.".into())
}

#[tauri::command]
pub async fn session_archive_clear(state: State<'_, AppState>) -> Result<(), String> {
    let _lifecycle = state.session.settings_mutation_guard(true).await?;
    state
        .session
        .clear_archive()
        .map_err(|_| "Could not clear session content.")?;
    Ok(())
}

#[tauri::command]
pub async fn session_export(
    app: AppHandle,
    state: State<'_, AppState>,
    kind: ExportKind,
    id: Option<String>,
    audio_source: Option<AudioSource>,
    options: Option<TranscriptExportOptions>,
) -> Result<bool, String> {
    let busy = ExportGuard::acquire(&EXPORT_BUSY)?;
    let options = options.unwrap_or_default();
    let (busy, revision, bytes) = if let Some(id) = id {
        let history = state.session.history();
        let (busy, bytes) = run_export_worker(busy, None, move || match kind {
            ExportKind::Transcript => history.export_transcript_with_options(&id, options),
            ExportKind::Audio => history.audio_selected(&id, audio_source),
        })
        .await
        .map_err(|_| "Could not read saved session.")?;
        let bytes = bytes.map_err(|_| "Could not read saved session.")?;
        (busy, None, bytes)
    } else {
        let lifecycle = state.session.settings_mutation_guard(true).await?;
        let session = std::sync::Arc::clone(&state.session);
        let (busy, snapshot) = run_export_worker(busy, Some(lifecycle), move || {
            let bytes = match kind {
                ExportKind::Transcript => session.export_transcript(options),
                ExportKind::Audio => session.export_audio_from(audio_source),
            }?;
            Ok::<_, std::io::Error>((session.archive_revision(), bytes))
        })
        .await
        .map_err(|_| "Could not read current session content.")?;
        let (revision, bytes) = snapshot.map_err(|_| "Could not read current session content.")?;
        let bytes = bytes.ok_or("No session content is available to export.")?;
        (busy, Some(revision), bytes)
    };
    let (extension, description) = export_file_type(kind, options);
    let (tx, rx) = tokio::sync::oneshot::channel();
    let file_name = match (kind, audio_source) {
        (ExportKind::Audio, Some(AudioSource::System)) => "mimi-session-system.wav".to_owned(),
        (ExportKind::Audio, Some(AudioSource::Microphone)) => {
            "mimi-session-microphone.wav".to_owned()
        }
        _ => format!("mimi-session.{extension}"),
    };
    app.dialog()
        .file()
        .add_filter(description, &[extension])
        .set_file_name(file_name)
        .save_file(export_picker_callback(busy, tx));
    let (busy, path) = rx
        .await
        .map_err(|_| "The export dialog closed unexpectedly.")?;
    let Some(path) = path else {
        return Ok(false);
    };
    let path = path
        .into_path()
        .map_err(|_| "The export destination is not a local file.")?;
    // Revalidate after the picker: toggling off, clearing or starting a session
    // must invalidate the old snapshot. Serialize only the final write, never
    // the user's time in the dialog, with lifecycle and preference changes.
    let lifecycle = if revision.is_some() {
        Some(state.session.settings_mutation_guard(true).await?)
    } else {
        None
    };
    if let Some(revision) = revision {
        if state.session.archive_revision() != revision {
            return Err("The session changed. Please export again.".into());
        }
    }
    let (_busy, written) = run_export_worker(busy, lifecycle, move || write_export(&path, &bytes))
        .await
        .map_err(|_| "The export could not be completed.")?;
    written.map_err(|_| "The export could not be saved.")?;
    Ok(true)
}

fn export_file_type(
    kind: ExportKind,
    options: TranscriptExportOptions,
) -> (&'static str, &'static str) {
    match (kind, options.format) {
        (ExportKind::Transcript, TranscriptExportFormat::Txt) => ("txt", "Text transcript"),
        (ExportKind::Transcript, TranscriptExportFormat::Markdown) => ("md", "Markdown transcript"),
        (ExportKind::Audio, _) => ("wav", "WAV audio"),
    }
}

fn write_export(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("missing parent"))?;
    // Private temporary file in the destination directory; failure removes the
    // temporary file and preserves any previous destination. No app cache copy.
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::SubtitlePair;
    use crate::session_history::SessionHistory;
    use std::sync::{Arc, Mutex};

    #[test]
    fn cancelled_export_keeps_exclusivity_until_the_native_picker_replies() {
        static BUSY: AtomicBool = AtomicBool::new(false);
        let (reply, receiver) = tokio::sync::oneshot::channel();
        let callback = export_picker_callback(ExportGuard::acquire(&BUSY).unwrap(), reply);
        // Cancelling the async command drops the receiver, while the native
        // picker still owns its callback and remains visible.
        drop(receiver);
        assert!(ExportGuard::acquire(&BUSY).is_err());
        callback(None::<std::path::PathBuf>);
        assert!(ExportGuard::acquire(&BUSY).is_ok());
    }

    #[tokio::test]
    async fn picker_reply_transfers_exclusivity_to_the_export_and_dropped_callbacks_release_it() {
        static BUSY: AtomicBool = AtomicBool::new(false);
        let (reply, receiver) = tokio::sync::oneshot::channel();
        let callback = export_picker_callback(ExportGuard::acquire(&BUSY).unwrap(), reply);
        callback(Some(std::path::PathBuf::from("synthetic-export.txt")));
        assert!(ExportGuard::acquire(&BUSY).is_err());
        let (busy, path) = receiver.await.unwrap();
        assert_eq!(path.unwrap(), Path::new("synthetic-export.txt"));
        assert!(ExportGuard::acquire(&BUSY).is_err());
        drop(busy);

        let (reply, receiver) = tokio::sync::oneshot::channel::<(ExportGuard, Option<String>)>();
        let callback = export_picker_callback(ExportGuard::acquire(&BUSY).unwrap(), reply);
        drop(callback);
        assert!(receiver.await.is_err());
        assert!(ExportGuard::acquire(&BUSY).is_ok());
    }

    #[tokio::test]
    async fn cancelled_export_worker_keeps_lifecycle_and_exclusivity_until_disk_work_finishes() {
        static BUSY: AtomicBool = AtomicBool::new(false);
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("export.txt");
        let worker_destination = destination.clone();
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        let (started, working) = tokio::sync::oneshot::channel();
        let (finished, completed) = tokio::sync::oneshot::channel();
        let (release, released) = std::sync::mpsc::channel();
        let task = tokio::spawn(run_export_worker(
            ExportGuard::acquire(&BUSY).unwrap(),
            Some(Arc::clone(&lock).lock_owned().await),
            move || {
                started.send(()).unwrap();
                released.recv().unwrap();
                write_export(&worker_destination, b"Synthetic export").unwrap();
                finished.send(()).unwrap();
            },
        ));
        working.await.unwrap();
        assert!(lock.try_lock().is_err());
        assert!(ExportGuard::acquire(&BUSY).is_err());
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        let lifecycle_retained = lock.try_lock().is_err();
        let busy_retained = ExportGuard::acquire(&BUSY).is_err();
        // Release before asserting so a regression cannot leave a blocked
        // worker behind when this test fails.
        release.send(()).unwrap();
        completed.await.unwrap();
        let _lifecycle = tokio::time::timeout(std::time::Duration::from_secs(2), lock.lock())
            .await
            .expect("The completed worker must release the lifecycle lock");
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let Ok(busy) = ExportGuard::acquire(&BUSY) {
                    break busy;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("The completed worker must release export exclusivity");
        assert!(lifecycle_retained);
        assert!(busy_retained);
        assert_eq!(std::fs::read(destination).unwrap(), b"Synthetic export");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn export_snapshot_releases_lifecycle_but_keeps_exclusivity_through_the_picker() {
        static BUSY: AtomicBool = AtomicBool::new(false);
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        let (busy, snapshot) = run_export_worker(
            ExportGuard::acquire(&BUSY).unwrap(),
            Some(Arc::clone(&lock).lock_owned().await),
            || b"Synthetic snapshot".to_vec(),
        )
        .await
        .unwrap();
        assert!(lock.try_lock().is_ok());
        assert!(ExportGuard::acquire(&BUSY).is_err());
        // A native picker owns only export exclusivity; the final worker
        // obtains the lifecycle guard again after revision validation.
        let (busy, written) = run_export_worker(
            busy,
            Some(Arc::clone(&lock).lock_owned().await),
            move || snapshot,
        )
        .await
        .unwrap();
        assert_eq!(written, b"Synthetic snapshot");
        assert!(lock.try_lock().is_ok());
        assert!(ExportGuard::acquire(&BUSY).is_err());
        drop(busy);
        assert!(ExportGuard::acquire(&BUSY).is_ok());
    }

    #[tokio::test]
    async fn failed_export_worker_releases_its_lifecycle_and_busy_guards() {
        static BUSY: AtomicBool = AtomicBool::new(false);
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        let result: Result<(), String> = async {
            let (_busy, written) = run_export_worker(
                ExportGuard::acquire(&BUSY).unwrap(),
                Some(Arc::clone(&lock).lock_owned().await),
                || Err::<(), _>(std::io::Error::other("Synthetic private failure")),
            )
            .await
            .map_err(|_| "The export could not be completed.")?;
            written.map_err(|_| "The export could not be saved.")?;
            Ok(())
        }
        .await;
        assert_eq!(result.unwrap_err(), "The export could not be saved.");
        assert!(lock.try_lock().is_ok());
        assert!(ExportGuard::acquire(&BUSY).is_ok());
    }

    #[tokio::test]
    async fn current_audio_requires_stopped_state_and_sanitizes_read_failures() {
        let denied = read_stopped_audio(
            async { Err("Synthetic internal lifecycle detail".into()) },
            || panic!("An active session must not read recording data"),
        )
        .await;
        assert_eq!(
            denied.unwrap_err(),
            "Stop live subtitles before playing a recording."
        );

        for failure in [true, false] {
            let lock = Arc::new(tokio::sync::Mutex::new(()));
            let result =
                read_stopped_audio(async move { Ok(lock.lock_owned().await) }, move || {
                    if failure {
                        Err(std::io::Error::other("Synthetic private file detail"))
                    } else {
                        Ok(None)
                    }
                })
                .await;
            assert_eq!(
                result.unwrap_err(),
                if failure {
                    "Could not read current session audio."
                } else {
                    "No current session audio is available."
                },
            );
        }
    }

    #[tokio::test]
    async fn current_audio_keeps_lifecycle_locked_until_a_cancelled_read_finishes() {
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        let reader_lock = Arc::clone(&lock);
        let (started, reading) = tokio::sync::oneshot::channel();
        let (release, released) = std::sync::mpsc::channel();
        let task = tokio::spawn(read_stopped_audio(
            async move { Ok(reader_lock.lock_owned().await) },
            move || {
                started.send(()).unwrap();
                released.recv().unwrap();
                Ok(Some(vec![1, 2, 3]))
            },
        ));
        reading.await.unwrap();
        assert!(lock.try_lock().is_err());
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(lock.try_lock().is_err());
        release.send(()).unwrap();
        let _guard = tokio::time::timeout(std::time::Duration::from_secs(2), lock.lock())
            .await
            .expect("The completed read must release the lifecycle lock");
    }

    #[tokio::test]
    async fn current_audio_returns_the_existing_wav_bytes_without_reformatting() {
        let mut recording = crate::core::lifecycle::session_archive::AudioRecording::default();
        recording.begin(true);
        recording.append(16_000, &[1, 0, 255, 127]);
        let expected = recording.export().unwrap();
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        let audio = read_stopped_audio(async move { Ok(lock.lock_owned().await) }, move || {
            Ok(recording.export())
        })
        .await
        .unwrap();
        assert_eq!(audio, expected);
    }

    #[test]
    fn picker_extension_matches_transcript_format_and_audio_ignores_options() {
        for (format, extension) in [
            (TranscriptExportFormat::Txt, "txt"),
            (TranscriptExportFormat::Markdown, "md"),
        ] {
            let options = TranscriptExportOptions {
                format,
                ..Default::default()
            };
            assert_eq!(
                export_file_type(ExportKind::Transcript, options).0,
                extension
            );
            assert_eq!(
                export_file_type(ExportKind::Audio, options),
                ("wav", "WAV audio")
            );
        }
    }

    #[tokio::test]
    async fn history_listing_uses_the_session_owner_after_the_file_read() {
        for start_new_session in [true, false] {
            let directory = tempfile::tempdir().unwrap();
            let history = Arc::new(SessionHistory::new(directory.path().join("history"), false));
            let saved_id = uuid::Uuid::new_v4().to_string();
            history.begin(&saved_id, 100).unwrap();
            history
                .append_pair(
                    &saved_id,
                    &SubtitlePair::new("Synthetic saved".into(), "Synthetic result".into(), 150),
                )
                .unwrap();
            history.finalize(&saved_id, 200, false).unwrap();
            let changing_id = uuid::Uuid::new_v4().to_string();
            let current = Arc::new(Mutex::new(None));
            if !start_new_session {
                history.begin(&changing_id, 300).unwrap();
                *current.lock().unwrap() = Some(changing_id.clone());
                history
                    .append_pair(
                        &changing_id,
                        &SubtitlePair::new(
                            "Synthetic current".into(),
                            "Synthetic result".into(),
                            350,
                        ),
                    )
                    .unwrap();
            }
            let (reading_tx, reading_rx) = tokio::sync::oneshot::channel();
            let (continue_tx, continue_rx) = tokio::sync::oneshot::channel();
            let reader = Arc::clone(&history);
            let owner = Arc::clone(&current);
            let listing = tokio::spawn(list_saved_history(
                async move {
                    reading_tx.send(()).unwrap();
                    continue_rx.await.unwrap();
                    reader.list().map_err(|error| error.to_string())
                },
                move || owner.lock().unwrap().clone(),
            ));
            reading_rx.await.unwrap();

            if start_new_session {
                // Production Start writes the empty header, publishes its owner,
                // then allows providers/capture to append content.
                history.begin(&changing_id, 300).unwrap();
                *current.lock().unwrap() = Some(changing_id.clone());
                history
                    .append_pair(
                        &changing_id,
                        &SubtitlePair::new(
                            "Synthetic current".into(),
                            "Synthetic result".into(),
                            350,
                        ),
                    )
                    .unwrap();
            } else {
                // Successful finalization clears the current owner, so this
                // newly saved session must be visible in the completed list.
                history.finalize(&changing_id, 400, false).unwrap();
                *current.lock().unwrap() = None;
            }
            continue_tx.send(()).unwrap();
            let items = listing.await.unwrap().unwrap();
            assert!(items.iter().any(|item| item.id == saved_id));
            assert_eq!(
                items.iter().any(|item| item.id == changing_id),
                !start_new_session,
                "list must use the owner after reading; start_new_session={start_new_session}",
            );
            assert_eq!(items.len(), if start_new_session { 1 } else { 2 });
        }
    }

    #[test]
    fn exports_replace_only_the_selected_file_without_leaving_temporary_copies() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("export.txt");
        std::fs::write(&path, b"previous").unwrap();
        write_export(&path, b"synthetic test content").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"synthetic test content");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn failed_destination_leaves_existing_content_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("existing-directory");
        std::fs::create_dir(&destination).unwrap();
        assert!(write_export(&destination, b"synthetic test content").is_err());
        assert!(destination.is_dir());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn batch_delete_command_helper_protects_current_history_id() {
        let directory = tempfile::tempdir().unwrap();
        let history = Arc::new(SessionHistory::new(directory.path().join("history"), false));
        let saved_id = uuid::Uuid::new_v4().to_string();
        history.begin(&saved_id, 100).unwrap();
        history
            .append_pair(
                &saved_id,
                &SubtitlePair::new("Synthetic saved".into(), "Synthetic result".into(), 150),
            )
            .unwrap();
        history.finalize(&saved_id, 200, false).unwrap();

        let current_id = uuid::Uuid::new_v4().to_string();
        history.begin(&current_id, 300).unwrap();
        history
            .append_pair(
                &current_id,
                &SubtitlePair::new("Synthetic active".into(), "Synthetic result".into(), 350),
            )
            .unwrap();

        let protected = current_id.clone();
        let report = delete_saved_history_many(
            Arc::clone(&history),
            vec![saved_id.clone(), current_id.clone()],
            move || Some(protected),
        )
        .await
        .unwrap();

        assert_eq!(report.deleted, vec![saved_id]);
        assert_eq!(report.failed, vec![current_id.clone()]);
        assert_eq!(
            report.failure_reason,
            Some(HistoryDeleteFailureReason::ActiveSession)
        );
        let remaining: Vec<String> = history
            .list()
            .unwrap()
            .into_iter()
            .map(|item| item.id)
            .collect();
        assert_eq!(remaining, vec![current_id]);
    }

    #[tokio::test]
    async fn batch_delete_command_helper_reports_sanitized_reason_when_history_disabled() {
        let directory = tempfile::tempdir().unwrap();
        let history = Arc::new(SessionHistory::new(directory.path().join("history"), true));
        let id = uuid::Uuid::new_v4().to_string();
        let error = delete_saved_history_many(history, vec![id], || None)
            .await
            .unwrap_err();
        assert_eq!(error, HistoryDeleteFailureReason::HistoryDisabled.code());
    }
}
