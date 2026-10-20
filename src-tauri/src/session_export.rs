//! Settings-only manual export: native save picker, then atomic file replacement.
use crate::commands::AppState;
use crate::core::audio_input::AudioSource;
use crate::core::session_archive::{ArchiveState, TranscriptPage};
use crate::session_history::HistoryItem;
use serde::Deserialize;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

static EXPORT_BUSY: AtomicBool = AtomicBool::new(false);
struct ExportGuard;
impl Drop for ExportGuard {
    fn drop(&mut self) {
        EXPORT_BUSY.store(false, Ordering::SeqCst);
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
pub async fn session_history_delete(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let history = state.session.history();
    tauri::async_runtime::spawn_blocking(move || history.delete(&id))
        .await
        .map_err(|_| "Could not delete session history.")?
        .map_err(|_| "Could not delete session history.".into())
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
) -> Result<bool, String> {
    if EXPORT_BUSY
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("An export is already open.".into());
    }
    let _busy = ExportGuard;
    let (revision, bytes) = if let Some(id) = id {
        let history = state.session.history();
        let bytes = tauri::async_runtime::spawn_blocking(move || match kind {
            ExportKind::Transcript => history.export_transcript(&id),
            ExportKind::Audio => history.audio_selected(&id, audio_source),
        })
        .await
        .map_err(|_| "Could not read saved session.")?
        .map_err(|_| "Could not read saved session.")?;
        (None, bytes)
    } else {
        let _lifecycle = state.session.settings_mutation_guard(true).await?;
        let bytes = match kind {
            ExportKind::Transcript => state.session.export_transcript(),
            ExportKind::Audio => state.session.export_audio_from(audio_source),
        }
        .map_err(|_| "Could not read current session content.")?
        .ok_or("No session content is available to export.")?;
        (Some(state.session.archive_revision()), bytes)
    };
    let (extension, description) = match kind {
        ExportKind::Transcript => ("txt", "Text transcript"),
        ExportKind::Audio => ("wav", "WAV audio"),
    };
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
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    let Some(path) = rx
        .await
        .map_err(|_| "The export dialog closed unexpectedly.")?
    else {
        return Ok(false);
    };
    let path = path
        .into_path()
        .map_err(|_| "The export destination is not a local file.")?;
    // Revalidate after the picker: toggling off, clearing or starting a session
    // must invalidate the old snapshot. Serialize only the final write, never
    // the user's time in the dialog, with lifecycle and preference changes.
    let _lifecycle = if revision.is_some() {
        Some(state.session.settings_mutation_guard(true).await?)
    } else {
        None
    };
    if let Some(revision) = revision {
        if state.session.archive_revision() != revision {
            return Err("The session changed. Please export again.".into());
        }
    }
    tauri::async_runtime::spawn_blocking(move || write_export(&path, &bytes))
        .await
        .map_err(|_| "The export could not be completed.")?
        .map_err(|_| "The export could not be saved.")?;
    Ok(true)
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
}
