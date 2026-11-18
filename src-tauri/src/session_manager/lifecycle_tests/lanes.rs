//! Lanes behavior regressions.

use super::*;

#[tokio::test]
async fn audio_attempt_failure_cannot_claim_a_rebuilt_session_with_the_same_generation() {
    let active = Arc::new(AtomicU64::new(7));
    let sequence = Arc::new(AtomicU64::new(7));
    let transition = Arc::new(Mutex::new(Some((7, 100))));
    let (queued_tx, queued_rx) = tokio::sync::oneshot::channel();
    let (dispatch_tx, dispatch_rx) = tokio::sync::oneshot::channel();
    let delayed_failure = {
        let active = Arc::clone(&active);
        let sequence = Arc::clone(&sequence);
        let transition = Arc::clone(&transition);
        tokio::spawn(async move {
            // The old native/DSP callback has already reported a failure,
            // but its receiver has not reached the invalidation boundary.
            queued_tx.send(()).unwrap();
            dispatch_rx.await.unwrap();
            let mut owner = transition.lock().unwrap();
            invalidate_audio_attempt_atoms(&active, &sequence, &mut owner, 7, 100)
        })
    };
    queued_rx.await.unwrap();
    {
        let mut owner = transition.lock().unwrap();
        *owner = None; // cleanup retires the old attempt under the gate
        *owner = Some((7, 101)); // settings rebuild keeps generation 7
    }
    dispatch_tx.send(()).unwrap();
    assert_eq!(delayed_failure.await.unwrap(), None);
    assert_eq!(active.load(Ordering::SeqCst), 7);
    assert_eq!(sequence.load(Ordering::SeqCst), 7);
    let mut owner = transition.lock().unwrap();
    assert_eq!(
        invalidate_audio_attempt_atoms(&active, &sequence, &mut owner, 7, 101),
        Some(8)
    );
    assert_eq!(*owner, None);
    assert_eq!(active.load(Ordering::SeqCst), NO_GENERATION);
}

#[tokio::test]
async fn capture_status_reports_sound_silence_and_missing_pcm_per_lane() {
    let system = capture_status_test_pipeline();
    let microphone = capture_status_test_pipeline();
    system.ingress().unwrap().try_send(vec![128, 0]).unwrap();
    let system_slot = Mutex::new(Some(Arc::clone(&system)));
    let microphone_slot = Mutex::new(Some(Arc::clone(&microphone)));
    let generation = AtomicU64::new(7);
    let sound = source_capture_observation(&system_slot, &generation, 7).unwrap();
    let no_pcm = source_capture_observation(&microphone_slot, &generation, 7).unwrap();
    assert_eq!(
        sound,
        crate::audio::CaptureSignal {
            pcm_data_recent: true,
            sound_recent: true
        }
    );
    assert_eq!(
        no_pcm,
        crate::audio::CaptureSignal {
            pcm_data_recent: false,
            sound_recent: false
        }
    );
    // Receiving silent PCM is different from receiving no samples. Sound
    // on the system lane must not leak into this microphone observation.
    microphone.ingress().unwrap().try_send(vec![0, 0]).unwrap();
    let silent = source_capture_observation(&microphone_slot, &generation, 7).unwrap();
    assert_eq!(
        silent,
        crate::audio::CaptureSignal {
            pcm_data_recent: true,
            sound_recent: false
        }
    );
    assert_eq!(
        source_capture_observation(&system_slot, &generation, 7),
        Some(sound)
    );
    system.stop();
    microphone.stop();
}

#[tokio::test]
async fn capture_status_does_not_reuse_retired_or_removed_pipeline_activity() {
    let old = capture_status_test_pipeline();
    old.ingress().unwrap().try_send(vec![128, 0]).unwrap();
    let slot = Mutex::new(Some(Arc::clone(&old)));
    let generation = AtomicU64::new(7);
    assert!(
        source_capture_observation(&slot, &generation, 7)
            .unwrap()
            .sound_recent
    );
    // Stop/pause invalidates the active generation before teardown awaits.
    assert!(source_capture_observation(&slot, &generation, NO_GENERATION).is_none());
    assert!(source_capture_observation(&slot, &generation, 8).is_none());
    let removed = slot.lock().unwrap().take().unwrap();
    assert!(source_capture_observation(&slot, &generation, 7).is_none());
    // A new pipeline begins unobserved even while a retired Arc survives.
    let next = capture_status_test_pipeline();
    *slot.lock().unwrap() = Some(Arc::clone(&next));
    generation.store(8, Ordering::SeqCst);
    assert_eq!(
        source_capture_observation(&slot, &generation, 8),
        Some(crate::audio::CaptureSignal {
            pcm_data_recent: false,
            sound_recent: false,
        })
    );
    removed.stop();
    next.stop();
}

#[tokio::test]
async fn retired_capture_and_connections_release_together_before_resume() {
    let lifecycle = Arc::new(TokioMutex::new(()));
    let guard = Arc::clone(&lifecycle).lock_owned().await;
    let started = Arc::new(tokio::sync::Semaphore::new(0));
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let finished = Arc::new(tokio::sync::Semaphore::new(0));
    let (last_tx, last_rx) = tokio::sync::oneshot::channel();
    let last = Arc::new(Mutex::new(Some(last_rx)));
    let cleanup = tokio::spawn({
        let started = Arc::clone(&started);
        let release = Arc::clone(&release);
        let finished = Arc::clone(&finished);
        async move {
            let _guard = guard;
            let release_resource = |is_last: bool| {
                let started = Arc::clone(&started);
                let release = Arc::clone(&release);
                let last = Arc::clone(&last);
                let finished = Arc::clone(&finished);
                async move {
                    started.add_permits(1);
                    if is_last {
                        let finish = last.lock().unwrap().take().unwrap();
                        finish.await.unwrap();
                    } else {
                        release.acquire().await.unwrap().forget();
                        finished.add_permits(1);
                    }
                }
            };
            release_retired_sources(
                |_| release_resource(false),
                |source| release_resource(source == AudioSource::Microphone),
            )
            .await;
        }
    });

    let all_started = tokio::time::timeout(Duration::from_secs(1), started.acquire_many(4))
        .await
        .expect("a retired resource must not wait for another resource to finish")
        .unwrap();
    all_started.forget();
    release.add_permits(3);
    tokio::time::timeout(Duration::from_secs(1), finished.acquire_many(3))
        .await
        .unwrap()
        .unwrap()
        .forget();
    assert!(Arc::clone(&lifecycle).try_lock_owned().is_err());
    last_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(1), cleanup)
        .await
        .unwrap()
        .unwrap();
    assert!(lifecycle.try_lock_owned().is_ok());
}

#[test]
fn either_source_finishing_or_failing_ends_all_selected_sources() {
    for source in [AudioSource::System, AudioSource::Microphone] {
        for event in [
            LiveTranslateServerEvent::SessionFinished,
            LiveTranslateServerEvent::Error {
                code: "fatal".into(),
                message: "unavailable".into(),
            },
        ] {
            let mut controller = TranslationSessionController::default();
            controller.set_audio_input(AudioInput::Both);
            controller.did_connect();
            for &input in AudioInput::Both.sources() {
                controller.handle_from(input, LiveTranslateServerEvent::TranslationStarted);
            }
            assert!(apply_provider_event(&mut controller, source, &event).is_none());
            assert!(!controller.state.status.is_active());
            assert!(!controller.state.is_translation_pending);
            assert!(controller
                .state
                .subtitles
                .tracks
                .iter()
                .all(|track| !track.is_translation_pending));
            // A normal start following terminal cleanup can establish both again.
            controller.begin_connecting();
            controller.did_connect();
            assert_eq!(controller.state.status, SessionStatus::Listening);
        }
    }
}

#[tokio::test]
async fn dual_source_start_failure_rolls_back_every_installed_lane() {
    let installed = Arc::new(Mutex::new(Vec::new()));
    let rollbacks = AtomicUsize::new(0);
    let result = connect_source_group(
        AudioInput::Both.sources(),
        |source| {
            let installed = Arc::clone(&installed);
            async move {
                installed.lock().unwrap().push(source);
                if source == AudioSource::Microphone {
                    Err("microphone unavailable".into())
                } else {
                    Ok(())
                }
            }
        },
        |_| async {
            rollbacks.fetch_add(1, Ordering::SeqCst);
            assert_eq!(*installed.lock().unwrap(), AudioInput::Both.sources());
            installed.lock().unwrap().clear();
        },
    )
    .await;
    assert_eq!(result, Err("microphone unavailable".into()));
    assert!(installed.lock().unwrap().is_empty());
    assert_eq!(rollbacks.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn dual_source_stop_cancels_pending_second_start_and_rolls_back_first() {
    let active = Arc::new(AtomicU64::new(1));
    let sequence = Arc::new(AtomicU64::new(1));
    let notify = Arc::new(Notify::new());
    let installed = Arc::new(Mutex::new(Vec::new()));
    let result = connect_source_group(
        AudioInput::Both.sources(),
        |source| {
            let active = Arc::clone(&active);
            let sequence = Arc::clone(&sequence);
            let notify = Arc::clone(&notify);
            let installed = Arc::clone(&installed);
            async move {
                installed.lock().unwrap().push(source);
                if source == AudioSource::System {
                    return Ok(());
                }
                active.store(NO_GENERATION, Ordering::SeqCst);
                sequence.store(2, Ordering::SeqCst);
                notify.notify_waiters();
                run_generation_bound_operation(
                    active,
                    sequence,
                    notify,
                    1,
                    std::future::pending::<()>(),
                )
                .await
                .map(|_| ())
            }
        },
        |_| async {
            installed.lock().unwrap().clear();
        },
    )
    .await;
    assert_eq!(result, Err(SESSION_START_CANCELLED.into()));
    assert!(installed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn dual_source_success_keeps_both_lanes_and_never_rolls_back() {
    let started = Mutex::new(Vec::new());
    let rollback = AtomicBool::new(false);
    connect_source_group(
        AudioInput::Both.sources(),
        |source| {
            started.lock().unwrap().push(source);
            std::future::ready(Ok(()))
        },
        |_| async {
            rollback.store(true, Ordering::SeqCst);
        },
    )
    .await
    .unwrap();
    assert_eq!(*started.lock().unwrap(), AudioInput::Both.sources());
    assert!(!rollback.load(Ordering::SeqCst));
}
