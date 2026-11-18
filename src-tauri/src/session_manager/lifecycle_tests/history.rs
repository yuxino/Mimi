//! History behavior regressions.

use super::*;

#[test]
fn recording_status_changes_only_after_all_selected_files_receive_audio() {
    let directory = tempfile::tempdir().unwrap();
    let history = SessionHistory::new(directory.path().to_path_buf(), false);
    let id = uuid::Uuid::new_v4().to_string();
    history.begin(&id, 100).unwrap();
    let mut stats = recording_stats(AudioInput::Both);
    for (source, changed) in [
        (AudioSource::System, false),
        (AudioSource::Microphone, true),
        (AudioSource::System, false),
        (AudioSource::Microphone, false),
    ] {
        let result = append_recorded_audio(&mut stats, (7, 11), source, 16_000, &[1, 0], |data| {
            history.append_pcm_from(&id, source, 16_000, data)
        });
        assert_eq!(result.status_changed, changed);
        assert!(!result.write_failed);
    }
    assert_eq!(stats.audio_bytes, 8);
    assert_eq!(
        stats.audio_evidence.write_status(false),
        AudioRecordingStatus::Recording
    );
    for source in [AudioSource::System, AudioSource::Microphone] {
        assert_eq!(
            &history.audio_from(&id, source).unwrap()[44..],
            &[1, 0, 1, 0]
        );
    }
}

#[test]
fn recording_write_failure_does_not_count_samples_or_claim_a_complete_dual_recording() {
    let mut stats = recording_stats(AudioInput::Both);
    let failed = append_recorded_audio(
        &mut stats,
        (7, 11),
        AudioSource::System,
        16_000,
        &[1, 0],
        |_| Err(std::io::Error::other("synthetic write failure")),
    );
    assert!(failed.status_changed && failed.write_failed);
    assert_eq!(stats.audio_bytes, 0);
    for source in [AudioSource::Microphone, AudioSource::System] {
        let recovered =
            append_recorded_audio(&mut stats, (7, 11), source, 16_000, &[1, 0], |_| Ok(()));
        assert!(!recovered.status_changed);
    }
    assert_eq!(
        stats.audio_evidence.write_status(false),
        AudioRecordingStatus::Error
    );
}

#[test]
fn recording_limit_stops_writes_and_publishes_the_transition_once() {
    let mut stats = recording_stats(AudioInput::System);
    stats.audio_bytes = crate::core::session_archive::AUDIO_BYTE_LIMIT - 2;
    let outcome = append_recorded_audio(
        &mut stats,
        (7, 11),
        AudioSource::System,
        16_000,
        &[1, 0, 2, 0],
        |accepted| {
            assert_eq!(accepted, &[1, 0]);
            Ok(())
        },
    );
    assert!(outcome.status_changed);
    assert!(!outcome.write_failed);
    assert_eq!(
        stats.audio_evidence.write_status(stats.audio_limited),
        AudioRecordingStatus::Limited
    );
    let repeated = append_recorded_audio(
        &mut stats,
        (7, 11),
        AudioSource::System,
        16_000,
        &[1, 0],
        |_| panic!("the recording limit must stop later writes"),
    );
    assert!(!repeated.status_changed);
}

#[test]
fn recording_rejects_stale_attempts_and_empty_or_invalid_pcm_without_success_evidence() {
    let mut stats = recording_stats(AudioInput::System);
    for (owner, data) in [
        ((6, 11), &[1, 0][..]),
        ((7, 10), &[1, 0][..]),
        ((7, 11), &[][..]),
    ] {
        let outcome =
            append_recorded_audio(&mut stats, owner, AudioSource::System, 16_000, data, |_| {
                panic!("stale or empty audio must not be appended")
            });
        assert!(!outcome.status_changed);
    }
    let invalid = append_recorded_audio(
        &mut stats,
        (7, 11),
        AudioSource::System,
        16_000,
        &[1],
        |_| panic!("unaligned audio must not be appended"),
    );
    assert!(invalid.status_changed);
    assert_eq!(stats.audio_bytes, 0);
    assert_eq!(
        stats.audio_evidence.write_status(stats.audio_limited),
        AudioRecordingStatus::Error
    );
}

#[test]
fn pending_text_opt_out_retries_failed_cleanup_before_finalization() {
    verify_failed_opt_out_cleanup_is_retried(false, true);
}

#[test]
fn pending_audio_opt_out_retries_failed_cleanup_before_finalization() {
    verify_failed_opt_out_cleanup_is_retried(true, false);
}

#[test]
fn repeated_confirmations_trigger_private_file_append_even_when_text_is_equal() {
    let first = crate::core::models::SubtitlePair::new(
        "Synthetic source".into(),
        "Synthetic translation".into(),
        1,
    );
    let second =
        crate::core::models::SubtitlePair::new(first.source.clone(), first.translation.clone(), 2);
    assert_eq!(
        first, second,
        "legacy text equality intentionally ignores timestamps"
    );
    assert!(confirmed_history_tail_changed(Some(&first), Some(&second)));
    assert!(!confirmed_history_tail_changed(
        Some(&second),
        Some(&second)
    ));
    assert!(confirmed_history_tail_changed(None, Some(&first)));
    assert!(!confirmed_history_tail_changed(Some(&first), None));
}

#[test]
fn profile_selection_preserves_recordings_by_rejecting_sample_rate_changes() {
    for current_rate in [16_000, 24_000] {
        for next_rate in [16_000, 24_000] {
            assert!(ensure_profile_switch_audio_format(false, current_rate, next_rate).is_ok());
            assert_eq!(
                ensure_profile_switch_audio_format(true, current_rate, next_rate),
                if current_rate == next_rate {
                    Ok(())
                } else {
                    Err("profile_switch_recording_requires_stop".into())
                }
            );
        }
    }
}
