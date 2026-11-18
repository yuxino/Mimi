//! Shared synthetic session fixtures; behavior regressions follow each owning concern.

use super::diagnostics::source_capture_observation;
use super::events::{
    apply_provider_envelope, apply_provider_event, confirmed_history_tail_changed,
    generation_accepts_event, subtitle_content_is_current,
};
use super::fixtures::{finish_ui_test_archive, seed_ui_test_live_subtitles, ui_test_audio_samples};
use super::history::{append_recorded_audio, finalize_pending_history, LocalCaptureStats};
use super::lanes::{connect_source_group, release_retired_sources, EventPumpCleanup};
use super::lifecycle::{
    apply_establish_failure_state, clear_stale_configuration_error, session_connect_error,
    session_error_requires_user_action,
};
use super::ownership::{
    accepted_pause_transition_is_valid, begin_pause_request, clear_owned_value_if_generation,
    clear_task_slot_if_id, invalidate_audio_attempt_atoms, invalidate_generation_atoms,
    lifecycle_activity_is_active, lock_after_operations, pause_transition_is_valid,
    restore_failed_resume, resume_failure_is_still_owned, resume_transition_is_valid,
    retain_lifecycle_during_teardown, run_generation_bound_operation, start_request_can_proceed,
    try_begin_start, update_owned_value, SessionRequestGuard, SESSION_START_CANCELLED,
};
use super::publication::{
    publish_stop_boundary, status_should_show_overlay, visible_session_latencies,
    HealthCheckLatency, SessionStateEvent, StateBroadcasts, StatusPayload,
};
use super::recording_state::{AudioRecordingStatus, RecordingEvidence};
use super::recovery::{
    advance_recovery_request_atoms, claim_generation_recovery_atoms, commit_recovery_success_atoms,
    complete_recovery_health_round,
};
use super::selection::{
    audio_capture_selection_needs_restart, audio_input_switch_action,
    commit_audio_input_switch_boundary, commit_profile_selection,
    ensure_profile_switch_audio_format, pipeline_settings_mutation_is_allowed,
    profile_switch_action, release_reconfigured_audio_sources, source_switch_requires_reconnect,
    translation_mode_after_source_switch, validate_system_audio_target, AudioInputSwitchAction,
    ProfileSwitchAction,
};
use crate::audio::send_pipeline::AudioSendPipeline;
use crate::clients::provider_events::{provider_event_channel, ProviderEvent};
use crate::clients::translation_client::ConnectError;
use crate::core::audio_input::{AudioInput, AudioSource};
use crate::core::configuration::LiveTranslationConfiguration;
#[cfg(test)]
use crate::core::credentials::{ProviderCredentials, TextTranslationCredentials};
use crate::core::diagnostics::{TranslationLatency, TranslationLatencyKind, TranslationRecovery};
use crate::core::models::{SessionStatus, SourceLanguage, TranslationMode, UtteranceRole};
use crate::core::protocols::live_translate::LiveTranslateServerEvent;
#[cfg(test)]
use crate::core::protocols::qwen_mt::{QwenMTModel, REALTIME_MT_MODEL};
use crate::core::provider::ProviderKind;
use crate::core::session::{TranslationSessionController, TranslationSessionState};
use crate::core::system_audio_target::SystemAudioTarget;
use crate::session_history::SessionHistory;
use mimi_runtime::core::health::HEALTH_CHECK_INTERVAL;
use mimi_runtime::core::recovery::{
    advance_lifecycle_sequence_if_current, cancelled_recovery_attempt_is_retryable,
    clear_recovery_atoms, lifecycle_sequence_matches, provider_error_is_retryable,
    provider_recovery_minimum_delay, recovery_delay, recovery_exhaustion_is_still_owned,
    MTBudgetContinuity, MTBudgetRoute, MTBudgetScope, RecoveryContinuity, RecoverySchedule,
    NO_GENERATION, RECOVERY_ATTEMPTS,
};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex as TokioMutex, Notify};

mod budget;
mod events;
mod fixtures;
mod history;
mod lanes;
mod ownership;
mod publication;
mod recovery;
mod selection;

fn recording_stats(input: AudioInput) -> LocalCaptureStats {
    let mut evidence = RecordingEvidence::new(input);
    evidence.begin_attempt(7, 11);
    LocalCaptureStats {
        audio_evidence: evidence,
        ..Default::default()
    }
}

fn verify_failed_opt_out_cleanup_is_retried(retain_text: bool, retain_audio: bool) {
    let directory = tempfile::tempdir().unwrap();
    let history = SessionHistory::new(directory.path().to_path_buf(), false);
    let pair = |source| {
        let mut pair = crate::core::models::SubtitlePair::new(
            "Synthetic retained source".into(),
            "Synthetic retained translation".into(),
            100,
        );
        pair.audio_source = source;
        pair
    };
    let saved_id = uuid::Uuid::new_v4().to_string();
    let pending_id = uuid::Uuid::new_v4().to_string();
    for id in [&saved_id, &pending_id] {
        history.begin(id, 0).unwrap();
        for source in [AudioSource::System, AudioSource::Microphone] {
            history.append_pair(id, &pair(source)).unwrap();
            history
                .append_pcm_from(id, source, 16_000, &[1, 0, 2, 0])
                .unwrap();
        }
    }
    history.finalize(&saved_id, 200, false).unwrap();
    let saved_transcript = history.export_transcript(&saved_id).unwrap();
    let saved_audio = history
        .audio_from(&saved_id, AudioSource::Microphone)
        .unwrap();

    // A provider error ends listening but leaves the current journal for
    // the user's subsequent opt-out, restart or normal quit.
    let mut controller = TranslationSessionController::default();
    controller.did_connect();
    controller.did_fail("synthetic provider error");
    assert!(!controller.state.status.is_active());

    let blocked_path = directory.path().join(format!(
        "{pending_id}.{}",
        if retain_text { "pcm" } else { "jsonl" }
    ));
    let backup_path = directory.path().join("synthetic-cleanup-backup");
    std::fs::rename(&blocked_path, &backup_path).unwrap();
    std::fs::create_dir(&blocked_path).unwrap();
    let cleanup = if retain_text {
        history.clear_audio(&pending_id)
    } else {
        history.clear_text(&pending_id)
    };
    assert!(cleanup.is_err());
    assert!(
        finalize_pending_history(&history, &pending_id, 200, false, retain_text, retain_audio,)
            .is_err()
    );
    assert!(!directory.path().join(format!("{pending_id}.json")).exists());

    std::fs::remove_dir(&blocked_path).unwrap();
    std::fs::rename(&backup_path, &blocked_path).unwrap();
    assert!(
        finalize_pending_history(&history, &pending_id, 200, false, retain_text, retain_audio,)
            .unwrap()
    );
    let page = history.page(&pending_id, "", 0).unwrap();
    assert_eq!(page.total, if retain_text { 2 } else { 0 });
    if retain_text {
        assert_eq!(page.entries[0].audio_source, AudioSource::Microphone);
        assert_eq!(page.entries[1].audio_source, AudioSource::System);
    } else {
        assert!(history.export_transcript(&pending_id).is_err());
    }
    assert_eq!(
        history.audio_sources(&pending_id).unwrap(),
        if retain_audio {
            vec![AudioSource::System, AudioSource::Microphone]
        } else {
            Vec::new()
        }
    );
    for source in [AudioSource::System, AudioSource::Microphone] {
        assert_eq!(
            history.audio_from(&pending_id, source).is_ok(),
            retain_audio
        );
    }
    assert_eq!(
        history.export_transcript(&saved_id).unwrap(),
        saved_transcript
    );
    assert_eq!(
        history
            .audio_from(&saved_id, AudioSource::Microphone)
            .unwrap(),
        saved_audio
    );
}

fn capture_status_test_pipeline() -> Arc<AudioSendPipeline> {
    Arc::new(AudioSendPipeline::spawn(
        |_| async { Ok::<(), ()>(()) },
        |_| {},
    ))
}

fn synthetic_mt_scope(profile_id: &str, route: MTBudgetRoute) -> MTBudgetScope {
    MTBudgetScope {
        profile_id: profile_id.into(),
        provider: ProviderKind::AlibabaCloud,
        route,
    }
}
