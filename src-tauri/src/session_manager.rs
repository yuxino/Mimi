//! Desktop session owner and public entry point.
//! Lifecycle generations, selected input lanes and the shared subtitle controller have one owner; each sibling module implements one part of that lifecycle.

mod cleanup_owner;
mod diagnostics;
mod events;
mod fixtures;
mod history;
mod lanes;
mod lifecycle;
#[cfg(test)]
mod lifecycle_tests;
mod ownership;
mod publication;
mod recording_state;
mod recovery;
mod recovery_cleanup;
mod selection;

pub use publication::SessionStateEvent;
// Preserve the established event-type API even when only serde consumes its variants.
#[allow(unused_imports)]
pub use publication::StatusPayload;

use crate::core::audio_input::{AudioInput, AudioSource};
use crate::core::configuration::LiveTranslationConfiguration;
use crate::core::models::SessionStatus;
use crate::core::session::TranslationSessionController;
use crate::core::support_diagnostics::{
    CaptureObservation, DiagnosticJournal, RecoveryAction, SafeFailure,
};
use crate::session_history::SessionHistory;
use crate::settings_store::SettingsStore;
use cleanup_owner::CleanupOwners;
use history::LocalCaptureStats;
use lanes::{EchoPipelineSlot, RuntimeLane};
use mimi_runtime::core::recovery::{RecoveryContinuity, NO_GENERATION};
use ownership::lifecycle_activity_is_active;
use publication::{status_should_show_overlay, HealthCheckLatency, StateBroadcasts};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::{AppHandle, Manager};
use tokio::sync::{Mutex as TokioMutex, Notify, OwnedMutexGuard};
use tokio::task::JoinHandle;

#[derive(Clone)]
pub struct SessionManager {
    app: AppHandle,
    lanes: [RuntimeLane; 2],
    /// One DSP stage per connection attempt; a same-generation settings
    /// rebuild also replaces the reference history and adaptive filter.
    echo_pipeline: EchoPipelineSlot,
    audio_attempt: Arc<Mutex<Option<(u64, u64)>>>,
    diagnostic_epoch: Instant,
    diagnostic_error: Arc<Mutex<Option<(SafeFailure, Instant)>>>,
    diagnostic_recovery: Arc<Mutex<Option<(RecoveryAction, Instant)>>>,
    diagnostic_capture: Arc<Mutex<Option<(CaptureObservation, Instant)>>>,
    diagnostic_journal: Arc<Mutex<DiagnosticJournal>>,
    health_latency: Arc<Mutex<Option<HealthCheckLatency>>>,
    settings: Arc<SettingsStore>,
    controller: Arc<Mutex<TranslationSessionController>>,
    /// Captured at manual start; pause, reconnect and recovery preserve it.
    active_audio_input: Arc<Mutex<AudioInput>>,
    recording: Arc<Mutex<[crate::core::session_archive::AudioRecording; 2]>>,
    archive_revision: Arc<AtomicU64>,
    history: Arc<SessionHistory>,
    history_pending_id: Arc<Mutex<Option<String>>>,
    history_pending_text: Arc<AtomicBool>,
    history_pending_audio: Arc<AtomicBool>,
    history_stats: Arc<Mutex<LocalCaptureStats>>,
    history_save_error: Arc<AtomicBool>,
    /// Clear and event application (including local-history append) are atomic.
    subtitle_content_lock: Arc<TokioMutex<()>>,
    active_settings: Arc<Mutex<Option<LiveTranslationConfiguration>>>,
    active_settings_generation: Arc<AtomicU64>,
    is_paused: Arc<AtomicBool>,
    is_recovering: Arc<AtomicBool>,
    is_overlay_collapsed: Arc<AtomicBool>,
    state_broadcasts: Arc<StateBroadcasts>,
    health_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    health_task_id: Arc<AtomicU64>,
    recovery_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    recovery_task_id: Arc<AtomicU64>,
    recovery_continuity: Arc<Mutex<RecoveryContinuity>>,
    /// Marks a provider transport failure that cancelled the current recovery
    /// attempt. The existing recovery owner consumes it and performs the next
    /// retry instead of trying to enqueue a second recovery task.
    recovery_retry_generation: Arc<AtomicU64>,
    background_task_sequence: Arc<AtomicU64>,
    /// Explicit lifecycle operations and settings mutations share this lock,
    /// eliminating check-then-mutate races around credential/profile reads.
    lifecycle_lock: Arc<TokioMutex<()>>,
    lifecycle_operations: Arc<AtomicUsize>,
    lifecycle_sequence: Arc<AtomicU64>,
    lifecycle_notify: Arc<Notify>,
    active_generation: Arc<AtomicU64>,
    stopping_tail_generation: Arc<AtomicU64>,
    /// Start and Resume share one connection owner; Pause/Stop may supersede it.
    start_in_progress: Arc<AtomicBool>,
    /// Owns a toggle's direction and completion across independent WebViews.
    pause_in_progress: Arc<AtomicBool>,
    generation_transition: Arc<Mutex<()>>,
    teardown_operations: Arc<AtomicUsize>,
    teardown_notify: Arc<Notify>,
    /// Registered synchronously before a teardown can expose a newer start.
    /// Its independent owner survives cancellation of any individual waiter.
    cleanup_owners: CleanupOwners,
}

impl SessionManager {
    fn lane(&self, source: AudioSource) -> &RuntimeLane {
        &self.lanes[match source {
            AudioSource::System => 0,
            AudioSource::Microphone => 1,
        }]
    }

    fn sources(&self) -> &'static [AudioSource] {
        self.active_audio_input.lock().unwrap().sources()
    }

    pub fn new(app: AppHandle, settings: Arc<SettingsStore>) -> Arc<Self> {
        let history_directory = app.path().app_data_dir().ok();
        let history = SessionHistory::new(
            history_directory
                .clone()
                .unwrap_or_default()
                .join("session-history"),
            settings.is_ui_test() || history_directory.is_none(),
        );
        Arc::new(Self {
            lanes: [RuntimeLane::new(&app), RuntimeLane::new(&app)],
            echo_pipeline: Default::default(),
            audio_attempt: Default::default(),
            app,
            diagnostic_epoch: Instant::now(),
            diagnostic_error: Default::default(),
            diagnostic_recovery: Default::default(),
            diagnostic_capture: Default::default(),
            diagnostic_journal: Default::default(),
            health_latency: Default::default(),
            settings,
            controller: Arc::new(Mutex::new(TranslationSessionController::default())),
            active_audio_input: Default::default(),
            recording: Default::default(),
            archive_revision: Default::default(),
            history: Arc::new(history),
            history_pending_id: Arc::new(Mutex::new(None)),
            history_pending_text: Arc::new(AtomicBool::new(false)),
            history_pending_audio: Arc::new(AtomicBool::new(false)),
            history_stats: Arc::new(Mutex::new(LocalCaptureStats::default())),
            history_save_error: Arc::new(AtomicBool::new(false)),
            subtitle_content_lock: Default::default(),
            active_settings: Arc::new(Mutex::new(None)),
            active_settings_generation: Arc::new(AtomicU64::new(NO_GENERATION)),
            is_paused: Arc::new(AtomicBool::new(false)),
            is_recovering: Arc::new(AtomicBool::new(false)),
            is_overlay_collapsed: Arc::new(AtomicBool::new(false)),
            state_broadcasts: Arc::default(),
            health_task: Arc::new(Mutex::new(None)),
            health_task_id: Arc::new(AtomicU64::new(NO_GENERATION)),
            recovery_task: Arc::new(Mutex::new(None)),
            recovery_task_id: Arc::new(AtomicU64::new(NO_GENERATION)),
            recovery_continuity: Default::default(),
            recovery_retry_generation: Arc::new(AtomicU64::new(NO_GENERATION)),
            background_task_sequence: Arc::new(AtomicU64::new(0)),
            lifecycle_lock: Arc::new(TokioMutex::new(())),
            lifecycle_operations: Arc::new(AtomicUsize::new(0)),
            lifecycle_sequence: Arc::new(AtomicU64::new(0)),
            lifecycle_notify: Arc::new(Notify::new()),
            active_generation: Arc::new(AtomicU64::new(NO_GENERATION)),
            stopping_tail_generation: Arc::new(AtomicU64::new(NO_GENERATION)),
            start_in_progress: Arc::new(AtomicBool::new(false)),
            pause_in_progress: Arc::new(AtomicBool::new(false)),
            generation_transition: Arc::new(Mutex::new(())),
            teardown_operations: Arc::new(AtomicUsize::new(0)),
            teardown_notify: Arc::new(Notify::new()),
            cleanup_owners: CleanupOwners::default(),
        })
    }

    pub fn is_active(&self) -> bool {
        lifecycle_activity_is_active(
            self.has_active_session(),
            self.lifecycle_operations.load(Ordering::SeqCst),
        )
    }

    /// Errors remain visible in the overlay until the user retries or stops.
    /// This is intentionally separate from `is_active`: tray and shortcut
    /// actions must still treat an error as a stopped session that can retry.
    pub fn should_show_overlay(&self) -> bool {
        self.is_active()
            || status_should_show_overlay(&self.controller.lock().unwrap().state.status)
    }

    /// True only for an established/in-flight session, excluding a settings
    /// mutation that merely owns the shared lifecycle lock.
    pub fn has_active_session(&self) -> bool {
        self.controller.lock().unwrap().state.status.is_active()
            || self.active_generation.load(Ordering::SeqCst) != NO_GENERATION
    }

    /// Serializes a profile or listening-settings mutation with start/stop.
    /// The caller must retain the returned guard until its synchronous store
    /// mutation and persistence have both completed.
    pub async fn settings_mutation_guard(
        self: &Arc<Self>,
        require_inactive: bool,
    ) -> Result<OwnedMutexGuard<()>, String> {
        let guard = self.lock_after_teardown().await;
        if require_inactive && self.is_active() {
            Err("Listening settings cannot be changed while a session is active.".into())
        } else {
            Ok(guard)
        }
    }

    pub fn is_paused(&self) -> bool {
        self.is_paused.load(Ordering::SeqCst)
    }

    pub fn is_overlay_collapsed(&self) -> bool {
        self.is_overlay_collapsed.load(Ordering::SeqCst)
    }

    /// Machine-readable status kind for the global shortcut gate.
    pub fn status_kind(&self) -> String {
        match self.controller.lock().unwrap().state.status {
            SessionStatus::Idle => "idle".into(),
            SessionStatus::Connecting => "connecting".into(),
            SessionStatus::Listening => "listening".into(),
            SessionStatus::Stopping => "stopping".into(),
            SessionStatus::Error(_) => "error".into(),
        }
    }

    pub fn set_overlay_collapsed(&self, collapsed: bool) {
        self.is_overlay_collapsed.store(collapsed, Ordering::SeqCst);
    }

    fn is_ui_test(&self) -> bool {
        self.settings.is_ui_test()
    }
}
