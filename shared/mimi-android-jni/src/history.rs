//! JNI ownership only; journal limits, file formats and exporters are shared.
use mimi_runtime::{
    core::{
        models::SubtitlePair,
        session_archive::{TranscriptExportFormat, TranscriptExportOptions},
        subtitle_reducer::now_epoch_ms,
    },
    session_history::{HistoryItem, SessionHistory},
    session_journal::{JournalState, SessionJournal},
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex, OnceLock,
    },
};

const ERROR: &str = "session_history_unavailable";
const MAX_JOURNALS: usize = 2;
const MAX_RECENT_HANDLES: usize = 8;

struct Entry {
    directory: PathBuf,
    journal: Mutex<SessionJournal>,
    owners: AtomicUsize,
    finish_requested: AtomicBool,
    finish_settled: AtomicBool,
}

impl Entry {
    fn active_id(&self) -> Option<String> {
        if self.finish_settled.load(Ordering::SeqCst) && self.owners.load(Ordering::SeqCst) == 0 {
            return None;
        }
        self.journal.lock().ok()?.id().map(str::to_owned)
    }

    fn finish_if_released(&self) -> Result<(), &'static str> {
        if self.finish_requested.load(Ordering::SeqCst) && self.owners.load(Ordering::SeqCst) == 0 {
            self.finish_settled.store(false, Ordering::SeqCst);
            let result = self
                .journal
                .lock()
                .map_err(|_| ERROR)?
                .finish(now_epoch_ms())
                .map_err(|_| ERROR);
            self.finish_settled.store(true, Ordering::SeqCst);
            result?;
        }
        Ok(())
    }

    fn status(&self) -> Result<Value, &'static str> {
        let journal = self.journal.lock().map_err(|_| ERROR)?;
        Ok(json!({
            "pending": self.finish_requested.load(Ordering::SeqCst) && !self.finish_settled.load(Ordering::SeqCst),
            "finalized": journal.id().is_none(),
            "state": journal.state(),
        }))
    }
}

fn journals() -> &'static Mutex<HashMap<u64, Arc<Entry>>> {
    static JOURNALS: OnceLock<Mutex<HashMap<u64, Arc<Entry>>>> = OnceLock::new();
    JOURNALS.get_or_init(Default::default)
}

pub struct Lease(Arc<Entry>);
impl Lease {
    pub fn append(&self, pair: &SubtitlePair) {
        // Save failures stay on the journal state; never put caption content
        // or a private filesystem path into provider diagnostics.
        let _ = self.0.journal.lock().unwrap().append(pair);
    }
    pub fn state(&self) -> JournalState {
        self.0.journal.lock().unwrap().state()
    }

    pub fn release(self) -> JournalState {
        let entry = Arc::clone(&self.0);
        drop(self);
        let state = entry.journal.lock().unwrap().state();
        state
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        if self.0.owners.fetch_sub(1, Ordering::SeqCst) == 1 {
            let _ = self.0.finish_if_released();
        }
    }
}

pub fn attach(handle: u64) -> Result<Lease, &'static str> {
    let registry = journals().lock().map_err(|_| ERROR)?;
    let entry = registry.get(&handle).ok_or(ERROR)?;
    if entry.finish_requested.load(Ordering::SeqCst)
        || entry.journal.lock().map_err(|_| ERROR)?.id().is_none()
    {
        return Err(ERROR);
    }
    entry.owners.fetch_add(1, Ordering::SeqCst);
    Ok(Lease(Arc::clone(entry)))
}

fn directory(value: &Value, field: &str) -> Result<PathBuf, &'static str> {
    let text = value[field]
        .as_str()
        .filter(|v| !v.is_empty() && v.len() <= 4096)
        .ok_or(ERROR)?;
    let path = PathBuf::from(text);
    if !path.is_absolute() {
        return Err(ERROR);
    }
    Ok(path)
}

fn active_ids(directory: &PathBuf) -> Result<Vec<String>, &'static str> {
    let registry = journals().lock().map_err(|_| ERROR)?;
    Ok(registry
        .values()
        .filter(|entry| entry.directory == *directory)
        .filter_map(|entry| entry.active_id())
        .collect())
}

fn list_saved_history(
    directory: &PathBuf,
    read_history: impl FnOnce() -> std::io::Result<Vec<HistoryItem>>,
) -> Result<Vec<HistoryItem>, &'static str> {
    let mut items = read_history().map_err(|_| ERROR)?;
    // Start or finish can change ownership while the file read is in progress.
    let active = active_ids(directory)?;
    items.retain(|item| !active.contains(&item.id));
    Ok(items)
}

pub fn exchange(value: &Value) -> Result<Value, &'static str> {
    match value["operation"].as_str().ok_or(ERROR)? {
        "history_begin" => {
            let directory = directory(value, "directory")?;
            let mut registry = journals().lock().map_err(|_| ERROR)?;
            let active = registry
                .values()
                .filter(|entry| entry.active_id().is_some())
                .count();
            if active >= MAX_JOURNALS {
                return Err(ERROR);
            }
            // Keep recent completed handles for a service to observe deferred
            // finalization failures after both runtime handles have stopped.
            while registry.len() >= MAX_RECENT_HANDLES {
                let oldest = registry
                    .iter()
                    .filter(|(_, entry)| entry.active_id().is_none())
                    .map(|(handle, _)| *handle)
                    .min()
                    .ok_or(ERROR)?;
                registry.remove(&oldest);
            }
            let history = SessionHistory::new(directory.clone(), false);
            let journal = SessionJournal::begin(history, now_epoch_ms()).map_err(|_| ERROR)?;
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let handle = NEXT.fetch_add(1, Ordering::SeqCst);
            registry.insert(
                handle,
                Arc::new(Entry {
                    directory,
                    journal: Mutex::new(journal),
                    owners: AtomicUsize::new(0),
                    finish_requested: AtomicBool::new(false),
                    finish_settled: AtomicBool::new(false),
                }),
            );
            Ok(json!({"historyHandle":handle}))
        }
        "history_finish" | "history_disable" | "history_state" => {
            let handle = value["historyHandle"].as_u64().ok_or(ERROR)?;
            let registry = journals().lock().map_err(|_| ERROR)?;
            let entry = registry.get(&handle).ok_or(ERROR)?;
            if value["operation"] == "history_state" {
                return entry.status();
            }
            entry.finish_requested.store(true, Ordering::SeqCst);
            if value["operation"] == "history_disable" {
                let result = entry
                    .journal
                    .lock()
                    .map_err(|_| ERROR)?
                    .disable()
                    .map_err(|_| ERROR);
                entry.finish_settled.store(true, Ordering::SeqCst);
                result?;
            } else {
                entry.finish_if_released()?;
            }
            entry.status()
        }
        "history_list" | "history_page" | "history_export" | "history_delete" => {
            let directory = directory(value, "directory")?;
            let history = SessionHistory::new(directory.clone(), false);
            if value["operation"] == "history_list" {
                let items = list_saved_history(&directory, || history.list())?;
                return Ok(json!({"items":items}));
            }
            let active = active_ids(&directory)?;
            let id = value["id"].as_str().ok_or(ERROR)?;
            if active.iter().any(|active| active == id) {
                return Err("session_history_active");
            }
            match value["operation"].as_str().unwrap() {
                "history_page" => {
                    let query = value.get("query").and_then(Value::as_str).unwrap_or("");
                    if query.chars().count() > 120 {
                        return Err(ERROR);
                    }
                    let page = value.get("page").and_then(Value::as_u64).unwrap_or(0);
                    serde_json::to_value(
                        history
                            .page(id, query, page.min(usize::MAX as u64) as usize)
                            .map_err(|_| ERROR)?,
                    )
                    .map_err(|_| ERROR)
                }
                "history_delete" => {
                    history.delete(id).map_err(|_| ERROR)?;
                    Ok(json!({}))
                }
                "history_export" => {
                    let options: TranscriptExportOptions = value
                        .get("options")
                        .map(|options| serde_json::from_value(options.clone()))
                        .transpose()
                        .map_err(|_| ERROR)?
                        .unwrap_or_default();
                    let export_directory = directory_from_export(value)?;
                    let path = history
                        .export_transcript_to_directory(id, options, &export_directory)
                        .map_err(|_| ERROR)?;
                    let mime_type = match options.format {
                        TranscriptExportFormat::Txt => "text/plain",
                        TranscriptExportFormat::Markdown => "text/markdown",
                    };
                    let extension = match options.format {
                        TranscriptExportFormat::Txt => "txt",
                        TranscriptExportFormat::Markdown => "md",
                    };
                    Ok(
                        json!({"path":path,"fileName":format!("mimi-{id}.{extension}"),"mimeType":mime_type}),
                    )
                }
                _ => unreachable!(),
            }
        }
        _ => Err("native_runtime_invalid_request"),
    }
}

fn directory_from_export(value: &Value) -> Result<PathBuf, &'static str> {
    directory(value, "exportDirectory")
}

#[cfg(test)]
mod tests {
    use super::*;
    use mimi_runtime::core::audio_input::AudioSource;

    fn registry_test_guard() -> std::sync::MutexGuard<'static, ()> {
        static TEST_LOCK: Mutex<()> = Mutex::new(());
        // Mutation checks deliberately fail assertions; keep the next case
        // runnable after its predecessor has cleaned up its own journal handles.
        TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    struct HistoryFixture {
        directory: PathBuf,
        handles: Vec<u64>,
    }

    impl HistoryFixture {
        fn new() -> Self {
            static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);
            Self {
                directory: std::env::temp_dir().join(format!(
                    "mimi-history-list-jni-{}-{}-{}",
                    std::process::id(),
                    now_epoch_ms(),
                    NEXT_DIRECTORY.fetch_add(1, Ordering::SeqCst),
                )),
                handles: Vec::new(),
            }
        }

        fn begin(&mut self) -> (u64, Lease, String) {
            let result = exchange(&json!({
                "operation":"history_begin",
                "directory":self.directory,
            }))
            .unwrap();
            let handle = result["historyHandle"].as_u64().unwrap();
            self.handles.push(handle);
            let owner = attach(handle).unwrap();
            let id = journals().lock().unwrap()[&handle]
                .journal
                .lock()
                .unwrap()
                .id()
                .unwrap()
                .to_owned();
            owner.append(&SubtitlePair::new(
                "Synthetic source".into(),
                "Synthetic target".into(),
                now_epoch_ms(),
            ));
            assert_eq!(owner.state().count, 1);
            (handle, owner, id)
        }
    }

    impl Drop for HistoryFixture {
        fn drop(&mut self) {
            {
                let mut registry = journals()
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                for handle in &self.handles {
                    registry.remove(handle);
                }
            }
            if self.directory.is_dir() {
                let _ = std::fs::remove_dir_all(&self.directory);
            } else {
                let _ = std::fs::remove_file(&self.directory);
            }
        }
    }

    fn finish_owned_journal(handle: u64, owner: Lease) {
        let finish = exchange(&json!({"operation":"history_finish","historyHandle":handle}));
        assert_eq!(finish.unwrap()["pending"], true);
        assert!(!owner.release().save_error);
        let state = exchange(&json!({"operation":"history_state","historyHandle":handle}));
        assert_eq!(state.unwrap()["finalized"], true);
    }

    #[test]
    fn history_listing_hides_a_journal_started_during_the_file_read() {
        let _serial = registry_test_guard();
        let mut fixture = HistoryFixture::new();
        let (saved_handle, saved_owner, saved_id) = fixture.begin();
        finish_owned_journal(saved_handle, saved_owner);
        let directory = fixture.directory.clone();
        let history = SessionHistory::new(directory.clone(), false);
        let mut started = None;
        let items = list_saved_history(&directory, || {
            // This is the scheduling window after the old active-ID snapshot.
            started = Some(fixture.begin());
            history.list()
        })
        .unwrap();
        let started_id = &started.as_ref().unwrap().2;
        assert!(items.iter().all(|item| &item.id != started_id));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, saved_id);
    }

    #[test]
    fn history_listing_includes_a_journal_finished_during_the_file_read() {
        let _serial = registry_test_guard();
        let mut fixture = HistoryFixture::new();
        let (saved_handle, saved_owner, saved_id) = fixture.begin();
        finish_owned_journal(saved_handle, saved_owner);
        let (finishing_handle, finishing_owner, finishing_id) = fixture.begin();
        let directory = fixture.directory.clone();
        let history = SessionHistory::new(directory.clone(), false);
        let items = list_saved_history(&directory, || {
            finish_owned_journal(finishing_handle, finishing_owner);
            history.list()
        })
        .unwrap();
        assert!(items.iter().any(|item| item.id == finishing_id));
        assert!(items.iter().any(|item| item.id == saved_id));
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn history_listing_keeps_file_read_errors_sanitized() {
        let _serial = registry_test_guard();
        let fixture = HistoryFixture::new();
        std::fs::write(&fixture.directory, b"not a history directory").unwrap();
        let result = exchange(&json!({
            "operation":"history_list",
            "directory":fixture.directory,
        }));
        assert_eq!(result.unwrap_err(), ERROR);
    }

    #[test]
    fn shared_journal_waits_for_both_owners_and_retains_deferred_failure_status() {
        let _serial = registry_test_guard();
        let directory = std::env::temp_dir().join(format!(
            "mimi-journal-jni-{}-{}",
            std::process::id(),
            now_epoch_ms()
        ));
        let result = exchange(&json!({"operation":"history_begin","directory":directory})).unwrap();
        let handle = result["historyHandle"].as_u64().unwrap();
        let system = attach(handle).unwrap();
        let microphone = attach(handle).unwrap();
        let mut pair = SubtitlePair::new(
            "synthetic source".into(),
            "synthetic target".into(),
            now_epoch_ms(),
        );
        system.append(&pair);
        pair.audio_source = AudioSource::Microphone;
        microphone.append(&pair);
        let finish = json!({"operation":"history_finish","historyHandle":handle});
        assert_eq!(exchange(&finish).unwrap()["pending"], true);
        assert_eq!(
            exchange(&json!({"operation":"history_list","directory":directory})).unwrap()["items"],
            json!([])
        );
        system.release();
        let status = json!({"operation":"history_state","historyHandle":handle});
        assert_eq!(exchange(&status).unwrap()["pending"], true);
        let id = journals().lock().unwrap()[&handle]
            .journal
            .lock()
            .unwrap()
            .id()
            .unwrap()
            .to_owned();
        let blocked_final = directory.join(format!("{id}.json"));
        std::fs::create_dir(&blocked_final).unwrap();
        let released = microphone.release();
        assert!(released.save_error);
        let failed = exchange(&status).unwrap();
        assert_eq!(failed["pending"], false);
        assert_eq!(failed["finalized"], false);
        assert_eq!(failed["state"]["saveError"], true);
        let recovered =
            exchange(&json!({"operation":"history_list","directory":directory})).unwrap();
        assert_eq!(
            recovered["items"][0]["count"], 2,
            "a stopped failed finalization must leave its journal recoverable"
        );
        std::fs::remove_dir(&blocked_final).unwrap();
        assert_eq!(exchange(&finish).unwrap()["finalized"], true);
        let items = exchange(&json!({"operation":"history_list","directory":directory})).unwrap();
        assert_eq!(items["items"][0]["count"], 2);
        assert!(attach(handle).is_err());
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
