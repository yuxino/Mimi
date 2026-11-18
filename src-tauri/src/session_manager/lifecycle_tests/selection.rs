//! Selection behavior regressions.

use super::*;

#[test]
fn application_target_validation_is_bounded_and_ui_fixtures_never_probe_os_support() {
    let target = SystemAudioTarget::Application {
        id: "test.player".into(),
        name: "Test Player".into(),
    };
    assert_eq!(
        validate_system_audio_target(&target, false, || true),
        Ok(())
    );
    assert_eq!(
        validate_system_audio_target(&target, false, || false),
        Err("application_audio_unsupported".into())
    );
    assert_eq!(
        validate_system_audio_target(&target, true, || panic!("UI fixture probed OS")),
        Ok(())
    );
    assert_eq!(
        validate_system_audio_target(&SystemAudioTarget::System, false, || panic!(
            "System target probed app support"
        )),
        Ok(())
    );
    for ui_test in [false, true] {
        assert_eq!(
            validate_system_audio_target(
                &SystemAudioTarget::Application {
                    id: "".into(),
                    name: "Test Player".into(),
                },
                ui_test,
                || panic!("Invalid target probed OS")
            ),
            Err("application_audio_invalid_target".into())
        );
    }
}

#[test]
fn target_switch_restarts_selected_system_capture_but_not_microphone_only() {
    assert!(!audio_capture_selection_needs_restart(
        AudioInput::Microphone,
        AudioInput::Microphone
    ));
    for previous in [AudioInput::System, AudioInput::Microphone, AudioInput::Both] {
        for selected in [AudioInput::System, AudioInput::Microphone, AudioInput::Both] {
            if previous != selected || selected != AudioInput::Microphone {
                assert!(audio_capture_selection_needs_restart(previous, selected));
            }
        }
    }
}

#[tokio::test]
async fn application_target_change_waits_for_old_system_capture_without_stopping_selected_microphone(
) {
    let mut released = Vec::new();
    release_reconfigured_audio_sources(AudioInput::Both, AudioInput::Both, true, |source| {
        released.push(source);
        async { Ok(()) }
    })
    .await
    .unwrap();
    assert_eq!(released, [AudioSource::System]);
    released.clear();
    release_reconfigured_audio_sources(
        AudioInput::Microphone,
        AudioInput::Microphone,
        true,
        |source| {
            released.push(source);
            async { Ok(()) }
        },
    )
    .await
    .unwrap();
    assert!(released.is_empty());
    assert_eq!(
        release_reconfigured_audio_sources(
            AudioInput::System,
            AudioInput::System,
            true,
            |_| async { Err("audio_input_switch_stop_failed".into()) }
        )
        .await,
        Err("audio_input_switch_stop_failed".into())
    );
}

#[tokio::test]
async fn audio_input_switch_seals_old_events_before_resetting_confirmation_ids() {
    let controller = Arc::new(Mutex::new(TranslationSessionController::default()));
    let confirmed = LiveTranslateServerEvent::SubtitleConfirmedPair {
        source_utterance_id: Some(1),
        utterance_id: 1,
        source: "Synthetic confirmed system".into(),
        translation: "Synthetic confirmation".into(),
        language: Some("en".into()),
    };
    controller.lock().unwrap().did_connect();
    controller
        .lock()
        .unwrap()
        .handle_from(AudioSource::System, confirmed.clone());
    let expected_history = controller.lock().unwrap().state.subtitles.history.clone();
    let active = Arc::new(AtomicU64::new(7));
    let sequence = AtomicU64::new(7);
    let transition = Mutex::new(());
    let content = Arc::new(TokioMutex::new(()));
    let held_content = content.lock().await;
    let (popped_tx, popped_rx) = tokio::sync::oneshot::channel();
    let pump = {
        let controller = Arc::clone(&controller);
        let active = Arc::clone(&active);
        let content = Arc::clone(&content);
        tokio::spawn(async move {
            // These events have left the provider queue before the switch,
            // but cannot apply until its shared content gate is released.
            popped_tx.send(()).unwrap();
            let _content = content.lock().await;
            for event in [
                confirmed,
                LiveTranslateServerEvent::SourceDraft {
                    text: "Retired synthetic draft".into(),
                    language: Some("ja".into()),
                },
            ] {
                if generation_accepts_event(active.load(Ordering::SeqCst), NO_GENERATION, 7, &event)
                {
                    controller
                        .lock()
                        .unwrap()
                        .handle_from(AudioSource::System, event);
                }
            }
        })
    };
    popped_rx.await.unwrap();
    let boundary = {
        let _transition = transition.lock().unwrap();
        commit_audio_input_switch_boundary(
            &active,
            &sequence,
            7,
            &mut controller.lock().unwrap(),
            AudioInput::Both,
            || Ok(()),
        )
    };
    assert_eq!(boundary.unwrap(), (8, 7));
    assert_eq!(active.load(Ordering::SeqCst), NO_GENERATION);
    drop(held_content);
    pump.await.unwrap();
    let state = &controller.lock().unwrap().state;
    assert_eq!(state.subtitles.history, expected_history);
    assert!(state
        .subtitles
        .tracks
        .iter()
        .all(|track| !track.source.text.contains("Retired")));
}

#[test]
fn audio_input_switch_superseded_or_failed_save_never_resets_live_watermarks() {
    for (epoch, save_fails) in [(8, false), (7, true)] {
        let active = AtomicU64::new(7);
        let sequence = AtomicU64::new(epoch);
        let mut controller = TranslationSessionController::default();
        controller.did_connect();
        let final_event = LiveTranslateServerEvent::SubtitleConfirmedPair {
            source_utterance_id: Some(1),
            utterance_id: 1,
            source: "Synthetic original".into(),
            translation: "Synthetic translation".into(),
            language: Some("en".into()),
        };
        controller.handle_from(AudioSource::System, final_event.clone());
        let original_state = controller.state.clone();
        let persisted = AtomicBool::new(false);
        let result = commit_audio_input_switch_boundary(
            &active,
            &sequence,
            7,
            &mut controller,
            AudioInput::Both,
            || {
                persisted.store(true, Ordering::SeqCst);
                Err("audio_input_switch_save_failed".into())
            },
        );
        assert_eq!(
            result,
            Err(if save_fails {
                "audio_input_switch_save_failed"
            } else {
                "audio_input_switch_superseded"
            }
            .into())
        );
        assert_eq!(persisted.load(Ordering::SeqCst), save_fails);
        assert_eq!(active.load(Ordering::SeqCst), 7);
        assert_eq!(sequence.load(Ordering::SeqCst), epoch);
        assert_eq!(controller.state, original_state);
        controller.handle_from(AudioSource::System, final_event);
        assert_eq!(
            controller.state, original_state,
            "the old provider ID must still be rejected as a replay"
        );
    }
}

#[tokio::test]
async fn audio_input_switch_paused_idle_and_error_wait_for_native_release() {
    for status in [
        SessionStatus::Listening,
        SessionStatus::Idle,
        SessionStatus::Error("synthetic failure".into()),
    ] {
        let action =
            audio_input_switch_action(&status, status == SessionStatus::Listening, false, 0)
                .unwrap();
        assert_eq!(action, AudioInputSwitchAction::ReconfigureOnly);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (finished_tx, finished_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let mut started_tx = Some(started_tx);
            let mut finished_rx = Some(finished_rx);
            release_reconfigured_audio_sources(
                AudioInput::Both,
                AudioInput::System,
                false,
                |source| {
                    assert_eq!(source, AudioSource::Microphone);
                    started_tx.take().unwrap().send(()).unwrap();
                    let finished = finished_rx.take().unwrap();
                    async move {
                        finished.await.unwrap();
                        Ok(())
                    }
                },
            )
            .await?;
            Ok::<_, String>(action)
        });
        started_rx.await.unwrap();
        tokio::task::yield_now().await;
        assert!(
            !task.is_finished(),
            "selection-only changes still wait for the native microphone to release"
        );
        finished_tx.send(()).unwrap();
        assert_eq!(
            task.await.unwrap(),
            Ok(AudioInputSwitchAction::ReconfigureOnly)
        );
        assert_eq!(
            release_reconfigured_audio_sources(
                AudioInput::Both,
                AudioInput::System,
                false,
                |_| async { Err("audio_input_switch_stop_failed".into()) }
            )
            .await,
            Err("audio_input_switch_stop_failed".into())
        );
    }
}

#[tokio::test]
async fn audio_input_switch_hands_off_to_an_already_accepted_newer_pause() {
    let controller = Arc::new(Mutex::new(TranslationSessionController::default()));
    controller.lock().unwrap().did_connect();
    let active = Arc::new(AtomicU64::new(7));
    let sequence = Arc::new(AtomicU64::new(7));
    let paused = Arc::new(AtomicBool::new(false));
    let lifecycle = Arc::new(TokioMutex::new(()));
    let transition = Arc::new(Mutex::new(()));
    let switch_guard = Arc::clone(&lifecycle).lock_owned().await;
    let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
    let (claim_tx, claim_rx) = tokio::sync::oneshot::channel();
    let (claimed_tx, claimed_rx) = tokio::sync::oneshot::channel();
    let pause = {
        let controller = Arc::clone(&controller);
        let active = Arc::clone(&active);
        let sequence = Arc::clone(&sequence);
        let paused = Arc::clone(&paused);
        let lifecycle = Arc::clone(&lifecycle);
        let transition = Arc::clone(&transition);
        tokio::spawn(async move {
            assert!(pause_transition_is_valid(
                &controller.lock().unwrap().state.status,
                false,
                active.load(Ordering::SeqCst)
            ));
            accepted_tx.send(()).unwrap();
            claim_rx.await.unwrap();
            let pause_epoch = {
                let _transition = transition.lock().unwrap();
                let current = sequence.load(Ordering::SeqCst);
                advance_lifecycle_sequence_if_current(&sequence, current).unwrap()
            };
            claimed_tx.send(pause_epoch).unwrap();
            let _lifecycle = lifecycle.lock_owned().await;
            if lifecycle_sequence_matches(&sequence, pause_epoch)
                && accepted_pause_transition_is_valid(
                    &controller.lock().unwrap().state.status,
                    paused.load(Ordering::SeqCst),
                    active.load(Ordering::SeqCst),
                    true,
                )
            {
                active.store(NO_GENERATION, Ordering::SeqCst);
                paused.store(true, Ordering::SeqCst);
                controller.lock().unwrap().did_pause();
                true
            } else {
                false
            }
        })
    };
    accepted_rx.await.unwrap();
    {
        let _transition = transition.lock().unwrap();
        assert_eq!(
            commit_audio_input_switch_boundary(
                &active,
                &sequence,
                7,
                &mut controller.lock().unwrap(),
                AudioInput::Both,
                || Ok(())
            )
            .unwrap(),
            (8, 7)
        );
    }
    claim_tx.send(()).unwrap();
    assert_eq!(claimed_rx.await.unwrap(), 9);
    controller.lock().unwrap().begin_connecting();
    let resources_installed = Arc::new(AtomicBool::new(true));
    let (teardown_started_tx, teardown_started_rx) = tokio::sync::oneshot::channel();
    let (finish_teardown_tx, finish_teardown_rx) = tokio::sync::oneshot::channel();
    let teardown = {
        let resources_installed = Arc::clone(&resources_installed);
        tokio::spawn(async move {
            let _switch_guard = retain_lifecycle_during_teardown(switch_guard, async {
                teardown_started_tx.send(()).unwrap();
                finish_teardown_rx.await.unwrap();
                resources_installed.store(false, Ordering::SeqCst);
            })
            .await;
        })
    };
    teardown_started_rx.await.unwrap();
    tokio::task::yield_now().await;
    assert!(
        !pause.is_finished(),
        "Pause cannot expose Resume before old capture/client teardown finishes"
    );
    assert!(resources_installed.load(Ordering::SeqCst));
    finish_teardown_tx.send(()).unwrap();
    teardown.await.unwrap();
    assert!(pause.await.unwrap());
    let _resume_guard = Arc::clone(&lifecycle).lock_owned().await;
    assert!(
        !resources_installed.load(Ordering::SeqCst),
        "Resume cannot meet an old installed client or capture"
    );
    assert!(paused.load(Ordering::SeqCst));
    assert_eq!(
        controller.lock().unwrap().state.status,
        SessionStatus::Listening
    );
    assert_eq!(active.load(Ordering::SeqCst), NO_GENERATION);
    assert!(
        !lifecycle_sequence_matches(&sequence, 8),
        "the superseded switch cannot reconnect after pause"
    );
    assert!(resume_transition_is_valid(
        &controller.lock().unwrap().state.status,
        paused.load(Ordering::SeqCst),
        active.load(Ordering::SeqCst),
        true
    ));
}

#[test]
fn audio_input_switch_reconnects_only_a_running_unpaused_session() {
    assert_eq!(
        audio_input_switch_action(&SessionStatus::Listening, false, false, 0),
        Ok(AudioInputSwitchAction::Reconnect)
    );
    for status in [
        SessionStatus::Listening,
        SessionStatus::Idle,
        SessionStatus::Error("synthetic failure".into()),
    ] {
        assert_eq!(
            audio_input_switch_action(&status, true, false, 0),
            Ok(AudioInputSwitchAction::ReconfigureOnly)
        );
    }
    for status in [
        SessionStatus::Idle,
        SessionStatus::Error("synthetic failure".into()),
    ] {
        assert_eq!(
            audio_input_switch_action(&status, false, false, 0),
            Ok(AudioInputSwitchAction::ReconfigureOnly)
        );
    }
}

#[test]
fn audio_input_switch_rejects_inflight_lifecycle_and_recovery() {
    for status in [SessionStatus::Connecting, SessionStatus::Stopping] {
        for paused in [false, true] {
            assert_eq!(
                audio_input_switch_action(&status, paused, false, 0),
                Err("audio_input_switch_busy")
            );
        }
    }
    assert_eq!(
        audio_input_switch_action(&SessionStatus::Listening, false, true, 0),
        Err("audio_input_switch_busy")
    );
    assert_eq!(
        audio_input_switch_action(&SessionStatus::Listening, true, false, 1),
        Err("audio_input_switch_busy")
    );
}

#[tokio::test]
async fn terminal_teardown_blocks_new_start_or_profile_mutation() {
    let lock = Arc::new(TokioMutex::new(()));
    let teardown_count = Arc::new(AtomicUsize::new(1));
    let notify = Arc::new(Notify::new());
    let mut waiter = tokio::spawn(lock_after_operations(
        Arc::clone(&lock),
        Arc::clone(&teardown_count),
        Arc::clone(&notify),
    ));

    assert!(tokio::time::timeout(Duration::from_millis(20), &mut waiter)
        .await
        .is_err());
    teardown_count.store(0, Ordering::SeqCst);
    notify.notify_waiters();
    assert!(tokio::time::timeout(Duration::from_millis(200), waiter)
        .await
        .is_ok());
}

#[test]
fn stop_after_settings_switch_prevents_late_reconnect_claim() {
    let sequence = AtomicU64::new(101);
    let switch_epoch = sequence.load(Ordering::SeqCst);

    let stop_epoch = sequence.fetch_add(1, Ordering::SeqCst) + 1;
    assert_eq!(stop_epoch, 102);
    assert_eq!(
        advance_lifecycle_sequence_if_current(&sequence, switch_epoch),
        None
    );
    assert_eq!(sequence.load(Ordering::SeqCst), stop_epoch);
}

#[test]
fn profile_selection_reconnects_only_a_live_unpaused_session() {
    assert_eq!(
        profile_switch_action(&SessionStatus::Listening, false, false, 0),
        Ok(ProfileSwitchAction::Reconnect)
    );
    assert_eq!(
        profile_switch_action(&SessionStatus::Listening, true, false, 0),
        Ok(ProfileSwitchAction::KeepPaused)
    );
    for status in [
        SessionStatus::Idle,
        SessionStatus::Error("synthetic".into()),
    ] {
        assert_eq!(
            profile_switch_action(&status, false, false, 0),
            Ok(ProfileSwitchAction::SelectOnly)
        );
    }
    for status in [SessionStatus::Connecting, SessionStatus::Stopping] {
        for paused in [false, true] {
            assert_eq!(
                profile_switch_action(&status, paused, false, 0),
                Err("profile_switch_busy")
            );
        }
    }
    for (recovering, operations) in [(true, 0), (false, 1)] {
        assert_eq!(
            profile_switch_action(&SessionStatus::Listening, true, recovering, operations),
            Err("profile_switch_busy")
        );
    }
}

#[test]
fn profile_selection_commit_keeps_previous_configuration_on_failed_or_stale_selection() {
    let old = LiveTranslationConfiguration::for_provider(
        ProviderKind::AlibabaCloud,
        "synthetic-old",
        SourceLanguage::Japanese,
        crate::core::models::TargetLanguage::English,
        TranslationMode::Turbo,
    );
    let next = LiveTranslationConfiguration::for_provider(
        ProviderKind::OpenAIRealtime,
        "synthetic-next",
        SourceLanguage::Automatic,
        crate::core::models::TargetLanguage::English,
        TranslationMode::Turbo,
    );
    let active = Mutex::new(Some(old.clone()));
    let sequence = AtomicU64::new(11);
    assert_eq!(
        commit_profile_selection(&sequence, 11, &active, Some(next.clone()), || {
            Err("profile_switch_save_failed".into())
        }),
        Err("profile_switch_save_failed".into())
    );
    assert_eq!(*active.lock().unwrap(), Some(old.clone()));
    assert_eq!(
        commit_profile_selection(&sequence, 10, &active, Some(next.clone()), || {
            panic!("a superseded selection must never persist")
        }),
        Err("profile_switch_superseded".into())
    );
    assert_eq!(*active.lock().unwrap(), Some(old));
    commit_profile_selection(&sequence, 11, &active, Some(next.clone()), || Ok(())).unwrap();
    assert_eq!(*active.lock().unwrap(), Some(next));
    // The commit does not claim a new lifecycle operation; a later pause
    // or stop remains able to supersede the reconnect.
    assert_eq!(sequence.load(Ordering::SeqCst), 11);
}

#[tokio::test]
async fn switch_waiting_on_the_gate_cannot_mutate_after_a_newer_session_connects() {
    let lifecycle = Arc::new(TokioMutex::new(()));
    let sequence = Arc::new(AtomicU64::new(111));
    let preference_changed = Arc::new(AtomicBool::new(false));
    let controller = Arc::new(Mutex::new(TranslationSessionController::default()));
    let switch_epoch = sequence.load(Ordering::SeqCst);
    let newer_lifecycle = Arc::clone(&lifecycle).lock_owned().await;

    let stale_switch = {
        let lifecycle = Arc::clone(&lifecycle);
        let sequence = Arc::clone(&sequence);
        let preference_changed = Arc::clone(&preference_changed);
        tokio::spawn(async move {
            let _guard = lifecycle.lock_owned().await;
            if lifecycle_sequence_matches(&sequence, switch_epoch) {
                preference_changed.store(true, Ordering::SeqCst);
            }
        })
    };
    tokio::task::yield_now().await;

    sequence.store(switch_epoch + 1, Ordering::SeqCst);
    controller.lock().unwrap().begin_connecting();
    controller.lock().unwrap().did_connect();
    drop(newer_lifecycle);
    stale_switch.await.unwrap();

    assert!(!preference_changed.load(Ordering::SeqCst));
    assert_eq!(
        controller.lock().unwrap().state.status,
        SessionStatus::Listening
    );
}

#[tokio::test]
async fn pause_claim_after_switch_validation_inherits_the_saved_configuration() {
    let lifecycle = Arc::new(TokioMutex::new(()));
    let sequence = Arc::new(AtomicU64::new(121));
    let operations = Arc::new(AtomicUsize::new(0));
    let owner = Arc::new(AtomicU64::new(121));
    let active_settings = Arc::new(Mutex::new(Some(
        LiveTranslationConfiguration::for_provider(
            crate::core::services::provider::ProviderKind::AlibabaCloud,
            "test-key",
            SourceLanguage::English,
            crate::core::models::TargetLanguage::SimplifiedChinese,
            TranslationMode::LowLatency,
        ),
    )));

    // The switch has acquired the lifecycle gate and passed both stale
    // intent checks. Pause then claims the next epoch but must wait for
    // the durable preference write to finish.
    let switch_guard = Arc::clone(&lifecycle).lock_owned().await;
    assert!(lifecycle_sequence_matches(&sequence, 121));
    assert!(pipeline_settings_mutation_is_allowed(
        &SessionStatus::Listening,
        operations.load(Ordering::SeqCst),
    ));
    let (pause_claimed_tx, pause_claimed_rx) = tokio::sync::oneshot::channel();
    let pause = {
        let lifecycle = Arc::clone(&lifecycle);
        let sequence = Arc::clone(&sequence);
        let operations = Arc::clone(&operations);
        let active_settings = Arc::clone(&active_settings);
        tokio::spawn(async move {
            operations.fetch_add(1, Ordering::SeqCst);
            sequence.fetch_add(1, Ordering::SeqCst);
            pause_claimed_tx.send(()).unwrap();
            let _pause_guard = lifecycle.lock_owned().await;
            operations.fetch_sub(1, Ordering::SeqCst);
            active_settings.lock().unwrap().clone().unwrap()
        })
    };
    pause_claimed_rx.await.unwrap();
    assert_eq!(sequence.load(Ordering::SeqCst), 122);

    assert!(update_owned_value(
        &active_settings,
        &owner,
        |configuration| {
            configuration.source_language = SourceLanguage::Japanese;
            configuration.target_language = crate::core::models::TargetLanguage::English;
            configuration.translation_mode = TranslationMode::Turbo;
        }
    ));
    drop(switch_guard);

    let resumed_configuration = pause.await.unwrap();
    assert_eq!(
        resumed_configuration.source_language,
        SourceLanguage::Japanese
    );
    assert_eq!(
        resumed_configuration.target_language,
        crate::core::models::TargetLanguage::English
    );
    assert_eq!(
        resumed_configuration.translation_mode,
        TranslationMode::Turbo
    );
}

#[tokio::test]
async fn queued_profile_mutation_is_rejected_during_resume_failure_handoff() {
    let lifecycle = Arc::new(TokioMutex::new(()));
    let lifecycle_operations = Arc::new(AtomicUsize::new(1));
    let status = Arc::new(Mutex::new(SessionStatus::Error("resume failure".into())));
    let resume_guard = Arc::clone(&lifecycle).lock_owned().await;

    let mutation = {
        let lifecycle = Arc::clone(&lifecycle);
        let lifecycle_operations = Arc::clone(&lifecycle_operations);
        let status = Arc::clone(&status);
        tokio::spawn(async move {
            let _guard = lifecycle.lock_owned().await;
            !lifecycle_activity_is_active(
                status.lock().unwrap().is_active(),
                lifecycle_operations.load(Ordering::SeqCst),
            )
        })
    };
    tokio::task::yield_now().await;
    drop(resume_guard);

    assert!(
        !mutation.await.unwrap(),
        "profile mutation must be rejected"
    );
    lifecycle_operations.store(0, Ordering::SeqCst);
    assert!(!lifecycle_activity_is_active(false, 0));
}

#[test]
fn automatic_source_preserves_supported_turbo_modes() {
    assert_eq!(
        translation_mode_after_source_switch(
            ProviderKind::OpenAIRealtime,
            SourceLanguage::Automatic,
            TranslationMode::HighQuality,
        ),
        TranslationMode::Turbo
    );
    assert_eq!(
        translation_mode_after_source_switch(
            ProviderKind::VolcanoEngine,
            SourceLanguage::Japanese,
            TranslationMode::HighQuality,
        ),
        TranslationMode::Turbo
    );
    assert_eq!(
        translation_mode_after_source_switch(
            ProviderKind::AlibabaCloud,
            SourceLanguage::Automatic,
            TranslationMode::HighQuality,
        ),
        TranslationMode::Turbo
    );
    assert_eq!(
        translation_mode_after_source_switch(
            ProviderKind::AlibabaCloud,
            SourceLanguage::Automatic,
            TranslationMode::Turbo,
        ),
        TranslationMode::Turbo
    );

    assert!(source_switch_requires_reconnect(
        true,
        SourceLanguage::Automatic,
        crate::core::models::TargetLanguage::SimplifiedChinese,
        TranslationMode::Turbo,
        SourceLanguage::Automatic,
        crate::core::models::TargetLanguage::SimplifiedChinese,
        TranslationMode::LowLatency,
    ));
}
