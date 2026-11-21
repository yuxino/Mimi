//! Session payload projection and ordered/coalesced publication to native windows.

use super::recording_state::AudioRecordingStatus;
use super::SessionManager;
use crate::core::diagnostics::latency::{
    TranslationLatency, TranslationLatencyKind, TranslationRecovery,
};
use crate::core::lifecycle::session::TranslationSessionState;
use crate::core::models::SessionStatus;
use crate::windows::OverlayWindowManager;
use mimi_runtime::core::lifecycle::recovery::NO_GENERATION;
use serde::Serialize;
use std::future::Future;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use tauri::Emitter;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StatusPayload {
    Idle,
    Connecting,
    Listening,
    Stopping,
    Error { message: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStateEvent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug_snapshot_id: Option<u64>,
    pub status: StatusPayload,
    #[serde(rename = "isActive")]
    pub is_active: bool,
    #[serde(rename = "isPaused")]
    pub is_paused: bool,
    #[serde(rename = "audioRecording", skip_serializing_if = "Option::is_none")]
    pub audio_recording: Option<AudioRecordingStatus>,
    #[serde(rename = "isOverlayCollapsed")]
    pub is_overlay_collapsed: bool,
    pub subtitles: crate::core::models::SubtitleSnapshot,
    pub atomic_preview: Option<bool>,
    #[serde(rename = "detectedLanguage")]
    pub detected_language: Option<String>,
    #[serde(rename = "isTranslationPending")]
    pub is_translation_pending: bool,
    pub is_translation_preview_pending: bool,
    #[serde(rename = "isTranslationTimedOut")]
    pub is_translation_timed_out: bool,
    /// Existing WebSocket health-check round trip, including local send wait.
    /// HQ/DeepLX probe their ASR socket, not the text-translation HTTP service.
    pub api_latency_ms: Option<u64>,
    /// Latest current successful preview/final; the kind identifies boundaries.
    pub translation_latency_ms: Option<u64>,
    pub translation_latency_kind: Option<TranslationLatencyKind>,
    pub translation_recovery: Option<TranslationRecovery>,
}

#[derive(Clone, Copy)]
pub(super) struct HealthCheckLatency {
    pub(super) generation: u64,
    pub(super) task_id: u64,
    pub(super) milliseconds: u64,
}

pub(super) fn visible_session_latencies(
    status: &SessionStatus,
    paused: bool,
    recovering: bool,
    generation: u64,
    health_task_id: u64,
    health_latency: Option<HealthCheckLatency>,
    translation_latency: Option<TranslationLatency>,
) -> (Option<u64>, Option<u64>, Option<TranslationLatencyKind>) {
    if !matches!(status, SessionStatus::Listening)
        || paused
        || recovering
        || generation == NO_GENERATION
    {
        return (None, None, None);
    }
    let api_latency = health_latency
        .filter(|sample| {
            sample.generation == generation
                && sample.task_id == health_task_id
                && health_task_id != NO_GENERATION
        })
        .map(|sample| sample.milliseconds);
    (
        api_latency,
        translation_latency.map(|sample| sample.milliseconds),
        translation_latency.map(|sample| sample.kind),
    )
}

impl From<&TranslationSessionState> for SessionStateEvent {
    fn from(state: &TranslationSessionState) -> Self {
        let status = match &state.status {
            SessionStatus::Idle => StatusPayload::Idle,
            SessionStatus::Connecting => StatusPayload::Connecting,
            SessionStatus::Listening => StatusPayload::Listening,
            SessionStatus::Stopping => StatusPayload::Stopping,
            SessionStatus::Error(message) => StatusPayload::Error {
                message: message.clone(),
            },
        };
        Self {
            debug_snapshot_id: None,
            is_active: state.status.is_active(),
            status,
            is_paused: false,
            audio_recording: None,
            is_overlay_collapsed: false,
            subtitles: state.subtitles.clone(),
            atomic_preview: state.atomic_preview,
            detected_language: state
                .detected_language
                .as_ref()
                .map(|language| language.code.clone()),
            is_translation_pending: state.is_translation_pending,
            is_translation_preview_pending: state.is_translation_preview_pending,
            is_translation_timed_out: state.is_translation_timed_out,
            api_latency_ms: None,
            translation_latency_ms: None,
            translation_latency_kind: None,
            translation_recovery: state.translation_recovery,
        }
    }
}

pub(super) fn status_should_show_overlay(status: &SessionStatus) -> bool {
    status.is_active() || matches!(status, SessionStatus::Error(_))
}

/// Stop is a completion boundary: retain normal diagnostic scheduling and
/// publish the fully drained state before the caller can seal evidence or quit.
pub(super) async fn publish_stop_boundary(
    request_publish: impl FnOnce(),
    publish_now: impl Future<Output = ()>,
) {
    request_publish();
    publish_now.await;
}

#[derive(Default)]
pub(super) struct StateBroadcasts(Mutex<StateBroadcastStatus>);

#[derive(Default)]
pub(super) enum StateBroadcastStatus {
    #[default]
    Idle,
    Publishing,
    Pending,
}

impl StateBroadcasts {
    /// An idle caller owns the publisher until its cooldown sees no changes.
    pub(super) fn request(&self) -> bool {
        let mut state = self.0.lock().unwrap();
        if matches!(*state, StateBroadcastStatus::Idle) {
            *state = StateBroadcastStatus::Publishing;
            return true;
        }
        *state = StateBroadcastStatus::Pending;
        false
    }

    pub(super) async fn drain<Publish, Published>(&self, publish: Publish)
    where
        Publish: Fn() -> Published,
        Published: Future<Output = ()>,
    {
        // Requests queued before this task was polled are part of its first
        // snapshot. Changes while it publishes stay pending for the next one.
        *self.0.lock().unwrap() = StateBroadcastStatus::Publishing;
        loop {
            publish().await;
            tokio::time::sleep(mimi_runtime::core::lifecycle::health::SNAPSHOT_PUBLISH_INTERVAL)
                .await;
            let mut state = self.0.lock().unwrap();
            if !matches!(*state, StateBroadcastStatus::Pending) {
                *state = StateBroadcastStatus::Idle;
                return;
            }
            *state = StateBroadcastStatus::Publishing;
        }
    }
}

impl SessionManager {
    // MARK: state publishing

    /// Builds the current session state snapshot without emitting it (used by
    /// windows that boot after the last broadcast, e.g. the overlay control).
    pub fn current_state_event(&self) -> SessionStateEvent {
        let (mut event, status, recording_owner, stopping_tail_generation) = {
            let _transition = self.generation_transition.lock().unwrap();
            let controller = self.controller.lock().unwrap();
            let mut event = SessionStateEvent::from(&controller.state);
            event.is_active |= self.is_recovering.load(Ordering::SeqCst);
            event.is_paused = self.is_paused();
            let active_generation = self.active_generation.load(Ordering::SeqCst);
            let active_owner = self
                .audio_attempt
                .lock()
                .unwrap()
                .filter(|(generation, _)| {
                    *generation == active_generation && active_generation != NO_GENERATION
                });
            (
                event,
                controller.state.status.clone(),
                active_owner,
                self.stopping_tail_generation.load(Ordering::SeqCst),
            )
        };
        // Appending audio holds these counters during file I/O. Never wait for
        // that work with the subtitle controller or lifecycle gate held.
        {
            let stats = self.history_stats.lock().unwrap();
            let owner = recording_owner.or_else(|| {
                stats
                    .audio_evidence
                    .owner_for_generation(stopping_tail_generation)
            });
            event.audio_recording = Some(stats.audio_evidence.status(
                self.history_pending_audio.load(Ordering::SeqCst),
                &status,
                event.is_paused,
                owner,
                stats.audio_limited,
            ));
        }
        event.is_overlay_collapsed = self.is_overlay_collapsed();
        let generation = self.active_generation.load(Ordering::SeqCst);
        let translation_latency = self
            .sources()
            .iter()
            .filter_map(|&source| {
                self.client_for_generation(source, generation)
                    .and_then(|client| client.translation_latency())
            })
            .max_by_key(|latency| latency.milliseconds);
        let (api_latency_ms, translation_latency_ms, translation_latency_kind) =
            visible_session_latencies(
                &status,
                event.is_paused,
                self.is_recovering.load(Ordering::SeqCst),
                generation,
                self.health_task_id.load(Ordering::SeqCst),
                *self.health_latency.lock().unwrap(),
                translation_latency,
            );
        if self.is_generation_current(generation) {
            event.api_latency_ms = api_latency_ms;
            event.translation_latency_ms = translation_latency_ms;
            event.translation_latency_kind = translation_latency_kind;
        }
        #[cfg(feature = "development-debugger")]
        if self.is_ui_test() {
            Self::apply_ui_test_status_samples(&mut event);
        }
        event.debug_snapshot_id = crate::core::development::development_debug::record_snapshot(
            generation,
            &event.subtitles,
        );
        event
    }

    pub(super) fn schedule_lifecycle_state(self: &Arc<Self>) {
        self.observe_diagnostic_status();
        let manager = Arc::clone(self);
        tauri::async_runtime::spawn(async move { manager.publish_state_now().await });
    }

    /// Lifecycle actions should reach every window without waiting for a
    /// subtitle batch. Snapshot ordering still uses the same content lock.
    pub(super) async fn publish_lifecycle_state(self: &Arc<Self>) {
        self.observe_diagnostic_status();
        self.publish_state_now().await;
    }

    /// Publish the first state after an idle interval immediately, then batch
    /// streaming updates so all windows can share one bounded snapshot.
    pub fn publish_state(self: &Arc<Self>) {
        self.observe_diagnostic_status();
        if !self.state_broadcasts.request() {
            return;
        }
        let this = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            this.state_broadcasts
                .drain(|| this.publish_state_now())
                .await;
        });
    }

    pub(super) async fn publish_state_now(self: &Arc<Self>) {
        // Snapshot and delivery share the clear/apply gate, so a snapshot
        // captured before Clear cannot be emitted after the cleared one.
        let _content = self.subtitle_content_lock.lock().await;
        let event = self.current_state_event();
        #[cfg(any(test, feature = "development-debugger"))]
        crate::development_debugger::record_snapshot(&event, &self.settings);
        self.write_ui_test_session_state(&event);
        let should_show_overlay =
            event.is_active || matches!(&event.status, StatusPayload::Error { .. });
        let is_collapsed = event.is_overlay_collapsed;
        let preferences = self.settings.preferences();
        let click_through =
            preferences.overlay_locked || preferences.subtitle_blends_with_background;
        #[cfg(any(test, feature = "development-debugger"))]
        let debug_snapshot_id = event.debug_snapshot_id;
        let _delivered = self.app.emit("session-state", &event).is_ok();
        #[cfg(any(test, feature = "development-debugger"))]
        if let Some(snapshot_id) = debug_snapshot_id {
            crate::core::development::development_debug::record(
                crate::core::development::development_debug::DebugEvent::Published {
                    snapshot_id,
                    delivered: _delivered,
                    overlay_requested: should_show_overlay,
                    collapsed: is_collapsed,
                },
            );
        }
        OverlayWindowManager::sync_presentation(
            &self.app,
            should_show_overlay,
            is_collapsed,
            click_through,
            preferences.subtitle_blends_with_background,
        );
    }

    /// Broadcasts the current settings snapshot to every window (used after
    /// preference writes so no window keeps a stale selection, and when a
    /// window re-shows in case its webview missed events while hidden).
    pub fn publish_settings(&self) {
        crate::commands::sync_overlay_minimum(&self.app);
        if let Ok(payload) =
            crate::commands::SettingsSnapshotPayload::try_from_store(&self.settings)
        {
            let _ = self.app.emit("settings-changed", payload);
        }
    }
}
