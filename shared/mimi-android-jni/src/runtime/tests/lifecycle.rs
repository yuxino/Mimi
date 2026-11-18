//! Native lifecycle regressions.

use super::*;

#[test]
fn retired_health_waiting_for_continuity_cannot_refresh_the_next_owner_budget() {
    for stopping in [false, true] {
        let native = fixture(configuration());
        native.epoch.store(2, Ordering::SeqCst);
        let now = std::time::Instant::now();
        let mut schedule = RecoverySchedule::new(1, 2, Duration::ZERO);
        schedule.next_attempt().unwrap();
        let mut continuity = native.recovery_continuity.lock().unwrap();
        continuity.record_connected(schedule, now);
        let (entered, waiting) = std::sync::mpsc::channel();
        let health = std::thread::spawn({
            let native = Arc::clone(&native);
            move || {
                entered.send(()).unwrap();
                native.record_health_success(
                    2,
                    now + mimi_runtime::core::health::HEALTH_CHECK_INTERVAL,
                )
            }
        });
        waiting.recv_timeout(Duration::from_secs(1)).unwrap();
        // A completed probe cannot acquire continuity until this exact
        // failure/stop transition has already retired its authority.
        if stopping {
            native.stopping.store(true, Ordering::SeqCst);
        } else {
            assert_eq!(
                advance_lifecycle_sequence_if_current(&native.epoch, 2),
                Some(3)
            );
        }
        drop(continuity);
        let reset = health.join().unwrap();
        let next = native
            .recovery_continuity
            .lock()
            .unwrap()
            .take_for_failure(2, 3, Duration::ZERO)
            .next_attempt()
            .unwrap();
        assert!(!reset, "retired health must not reset recovery continuity");
        assert_eq!(
            next.budget_index, 1,
            "the already-used recovery must survive the late probe"
        );
    }
}

#[test]
fn recovery_claim_transfers_used_budget_and_rejects_the_retired_probe() {
    let native = fixture(configuration());
    native.epoch.store(2, Ordering::SeqCst);
    let now = std::time::Instant::now();
    let mut prior = RecoverySchedule::new(1, 2, Duration::ZERO);
    prior.next_attempt().unwrap();
    native
        .recovery_continuity
        .lock()
        .unwrap()
        .record_connected(prior, now);

    let mut recovery = None;
    assert!(native.claim_recovery(2, &mut recovery, Duration::ZERO));
    assert_eq!(native.epoch.load(Ordering::SeqCst), 3);
    assert!(
        !native.record_health_success(2, now + mimi_runtime::core::health::HEALTH_CHECK_INTERVAL)
    );
    let mut resumed = recovery.unwrap();
    assert_eq!(resumed.generation(), 3);
    let attempt = resumed.next_attempt().unwrap();
    assert_eq!(attempt.index, 0);
    assert_eq!(attempt.budget_index, 1);
    native
        .recovery_continuity
        .lock()
        .unwrap()
        .record_connected(resumed, now);
    assert!(
        !native.record_health_success(2, now + mimi_runtime::core::health::HEALTH_CHECK_INTERVAL)
    );
    assert!(
        native.record_health_success(3, now + mimi_runtime::core::health::HEALTH_CHECK_INTERVAL)
    );
    let mut fresh = None;
    assert!(native.claim_recovery(3, &mut fresh, Duration::ZERO));
    assert_eq!(fresh.unwrap().next_attempt().unwrap().budget_index, 0);
}

#[tokio::test]
async fn pcm_backpressure_reports_one_retryable_error_for_its_connection() {
    use mimi_runtime::audio::send_pipeline::AudioIngressError;
    let native = fixture(configuration());
    let mut receiver = native.connection(1).unwrap();
    let ingress = native.ingress().unwrap();
    let mut backpressure = 0;
    // The actual pipeline waits for provider readiness. Twenty-two chunks
    // exceed the bounded queue even if its worker has taken one already.
    for _ in 0..22 {
        if ingress.try_send(vec![0, 0]) == Err(AudioIngressError::Backpressure) {
            backpressure += 1;
        }
    }
    assert_eq!(backpressure, 1);
    let event = receiver
        .try_recv_with_revision()
        .expect("a closed PCM ingress must notify the drive instead of silently losing audio");
    let LiveTranslateServerEvent::Error { code, .. } = event.event else {
        panic!("PCM backpressure must use the provider recovery lane");
    };
    assert_eq!(code, "transport_error");
    assert!(provider_error_is_retryable(&code));
    for _ in 0..10 {
        assert_eq!(ingress.try_send(vec![0, 0]), Err(AudioIngressError::Closed));
    }
    assert!(receiver.try_recv_with_revision().is_err());
    native.begin_stop().await;
}

#[tokio::test]
async fn pcm_backpressure_from_an_old_ingress_cannot_fail_a_new_connection() {
    use mimi_runtime::audio::send_pipeline::AudioIngressError;
    let native = fixture(configuration());
    let mut old_receiver = native.connection(1).unwrap();
    let old_ingress = native.ingress().unwrap();
    // Keep the old resources alive across replacement to model a native
    // submission that acquired its endpoint before the generation changed.
    let old_connection = native.take_connection_owned().unwrap();
    let mut current_receiver = native.connection(2).unwrap();
    for _ in 0..22 {
        let _ = old_ingress.try_send(vec![0, 0]);
    }
    assert!(matches!(
        old_receiver.try_recv_with_revision().unwrap().event,
        LiveTranslateServerEvent::Error { .. }
    ));
    assert!(current_receiver.try_recv_with_revision().is_err());
    assert_eq!(native.ingress().unwrap().try_send(vec![0, 0]), Ok(()));
    for _ in 0..10 {
        assert_eq!(
            old_ingress.try_send(vec![0, 0]),
            Err(AudioIngressError::Closed)
        );
    }
    assert!(current_receiver.try_recv_with_revision().is_err());
    drop(old_connection);
    native.begin_stop().await;
}

#[tokio::test]
async fn closed_pcm_ingress_after_hard_stop_does_not_request_recovery() {
    use mimi_runtime::audio::send_pipeline::AudioIngressError;
    let native = fixture(configuration());
    let mut receiver = native.connection(1).unwrap();
    let ingress = native.ingress().unwrap();
    native.begin_stop().await;
    let epoch = native.epoch.load(Ordering::SeqCst);
    assert_eq!(ingress.try_send(vec![0, 0]), Err(AudioIngressError::Closed));
    assert!(receiver.try_recv_with_revision().is_err());
    assert!(native.connection.lock().unwrap().is_none());
    assert_eq!(native.epoch.load(Ordering::SeqCst), epoch);
}

#[tokio::test]
async fn dropping_an_unpolled_connection_lease_stops_its_pipeline_and_finishes_cleanup() {
    let native = fixture(configuration());
    let _events = native.connection(1).unwrap();
    let ingress = native.ingress().unwrap();
    let lease = native.take_connection_owned().unwrap();
    assert!(native.connection.lock().unwrap().is_none());
    let owners = native.cleanup_owners.wait_registered();
    tokio::pin!(owners);
    assert!(tokio::time::timeout(Duration::from_millis(10), &mut owners)
        .await
        .is_err());

    drop(lease);

    tokio::time::timeout(Duration::from_secs(1), owners)
        .await
        .expect("a dropped lease must release its registered disconnect owner");
    assert!(ingress.try_send(vec![0, 0]).is_err());
}

#[tokio::test]
async fn same_generation_connections_keep_independent_cleanup_owners() {
    let native = fixture(configuration());
    let _first_events = native.connection(7).unwrap();
    let first = native.take_connection_owned().unwrap();
    let first_key = first.connection.cleanup_id;
    // A rebuild or another lane must not reuse the first client's factory,
    // even when a caller supplies the same lifecycle generation.
    let _second_events = native.connection(7).unwrap();
    let second = native.take_connection_owned().unwrap();
    assert_ne!(first_key, second.connection.cleanup_id);
    let owners = native.cleanup_owners.wait_registered();
    tokio::pin!(owners);
    drop(first);
    assert!(tokio::time::timeout(Duration::from_millis(10), &mut owners)
        .await
        .is_err());

    drop(second);

    tokio::time::timeout(Duration::from_secs(1), owners)
        .await
        .expect("each actual client must release its own owner");
}

#[tokio::test]
async fn hard_stop_joins_a_cancelled_drive_with_an_already_taken_connection() {
    let native = fixture(configuration());
    let _events = native.connection(1).unwrap();
    let ingress = native.ingress().unwrap();
    let (taken, was_taken) = oneshot::channel();
    let task = tokio::spawn({
        let native = native.clone();
        async move {
            let lease = native.take_connection_owned().unwrap();
            taken.send(()).unwrap();
            std::future::pending::<()>().await;
            drop(lease);
        }
    });
    *native.task.lock().unwrap() = Some(task);
    was_taken.await.unwrap();

    // Leave the stop ticket unpolled: aborting the drive must still drop
    // its lease and start final disconnect before another waiter joins.
    let stop = native.begin_stop();
    tokio::time::timeout(
        Duration::from_secs(1),
        native.cleanup_owners.wait_registered(),
    )
    .await
    .expect("drive cancellation must not cancel final disconnect");
    tokio::time::timeout(Duration::from_secs(1), stop)
        .await
        .expect("the stop completion must join the cancelled drive");
    assert!(native.connection.lock().unwrap().is_none());
    assert!(ingress.try_send(vec![0, 0]).is_err());
}

#[tokio::test]
async fn hard_stop_waits_for_late_installation_before_retiring_the_slot() {
    struct InstallOnDrop {
        native: Arc<Session>,
        entered: Option<oneshot::Sender<()>>,
        release: std::sync::mpsc::Receiver<()>,
        installed: Option<oneshot::Sender<ConnectionIngress>>,
    }
    impl Drop for InstallOnDrop {
        fn drop(&mut self) {
            let _ = self.entered.take().unwrap().send(());
            self.release.recv_timeout(Duration::from_secs(2)).unwrap();
            let _events = self.native.connection(1).unwrap();
            let _ = self
                .installed
                .take()
                .unwrap()
                .send(self.native.ingress().unwrap());
        }
    }
    let native = fixture(configuration());
    let (started, was_started) = oneshot::channel();
    let (entered, was_entered) = oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let (installed, was_installed) = oneshot::channel();
    let task = executor().spawn({
        let native = native.clone();
        async move {
            let installation = InstallOnDrop {
                native,
                entered: Some(entered),
                release: released,
                installed: Some(installed),
            };
            started.send(()).unwrap();
            std::future::pending::<()>().await;
            drop(installation);
        }
    });
    *native.task.lock().unwrap() = Some(task);
    was_started.await.unwrap();
    let stop = native.begin_stop();
    was_entered.await.unwrap();
    tokio::pin!(stop);
    let still_waiting = tokio::time::timeout(Duration::from_millis(20), &mut stop)
        .await
        .is_err();
    // Always release the worker before asserting, including a regression
    // where Stop incorrectly finishes while installation is still gated.
    release.send(()).unwrap();
    let ingress = was_installed.await.unwrap();
    assert!(still_waiting, "Stop must join installation before sweeping");
    tokio::time::timeout(Duration::from_secs(1), stop)
        .await
        .expect("the installed client must be retired after joining");
    assert!(native.connection.lock().unwrap().is_none());
    assert!(ingress.try_send(vec![0, 0]).is_err());
}

#[tokio::test]
async fn retiring_a_connection_preserves_request_budget_for_the_retry() {
    use mimi_runtime::core::preview_pacing::PreviewRequestPacer;
    let native = fixture(configuration());
    let _events = native.connection(7).unwrap();
    let client = native
        .connection
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .client
        .clone();
    let now = std::time::Instant::now();
    let mut pacer = PreviewRequestPacer::default();
    pacer.record_start(now);
    pacer.suppress_after_rate_limit(now);
    pacer.set_shared_cooldown(Some(now + Duration::from_secs(4)));
    let budget = pacer.export_budget();
    client.restore_mt_request_budget(Some(budget)).await;
    drop(client);

    native.retire(7).await;

    let _retry_events = native.connection(8).unwrap();
    let retained = native
        .connection
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .budget
        .expect("normal retirement must preserve request accounting");
    assert_eq!(
        retained.shared_cooldown_until(),
        budget.shared_cooldown_until()
    );
    assert!(retained.is_preview_suppressed(now));
    native.begin_stop().await;
}

#[tokio::test]
async fn a_retained_stopped_session_rejects_start_before_installing_resources() {
    // A failed regression must still be confined to loopback if it
    // mistakenly starts the drive instead of rejecting this owner.
    let mut configuration = configuration();
    configuration.provider = ProviderKind::CustomOpenAIASR;
    configuration.target_language = TargetLanguage::Original;
    configuration.credentials =
        mimi_runtime::core::credentials::ProviderCredentials::CustomSpeech {
            endpoint: "ws://127.0.0.1:9/v1/realtime".into(),
            model: "synthetic-stop-owner".into(),
            api_key: "synthetic-key".into(),
        };
    let native = fixture(configuration);
    native.begin_stop().await;

    assert_eq!(native.begin_start(), Err("native_runtime_stale_handle"));
    assert_eq!(native.cleanup_sequence.load(Ordering::SeqCst), 0);
    assert!(native.connection.lock().unwrap().is_none());
    assert!(native.task.lock().unwrap().is_none());
}

#[tokio::test]
async fn native_finish_drains_current_reliable_pairs_but_not_stale_content_or_drafts() {
    // Instantiate the real shared factory/pipeline without connecting it.
    // Events are synthetic and no provider endpoint is contacted.
    let native = fixture(configuration());
    native.publish(TranslationSessionController::begin_session);
    let mut receiver = native.connection(1).unwrap();
    native.publish(TranslationSessionController::did_connect);
    let events = native
        .connection
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .events
        .clone();
    let pair = |source: &str| LiveTranslateServerEvent::SubtitleFinalPair {
        source: source.into(),
        translation: source.into(),
        language: Some("en".into()),
    };
    events.send(pair("Synthetic stale pair")).unwrap();
    events.advance_content_revision();
    events
        .send(LiveTranslateServerEvent::SourceDraft {
            text: "Synthetic teardown draft".into(),
            language: Some("en".into()),
        })
        .unwrap();
    events.send(pair("Synthetic reliable tail")).unwrap();
    native.stopping.store(true, Ordering::SeqCst);
    tokio::time::timeout(
        Duration::from_secs(10),
        native.finish_connection(&mut receiver),
    )
    .await
    .unwrap();
    let state = serde_json::to_value(&native.controller.lock().unwrap().state.subtitles).unwrap();
    assert_eq!(state["displayPair"]["source"], "Synthetic reliable tail");
    assert_eq!(state["history"].as_array().unwrap().len(), 1);
    assert!(!state.to_string().contains("Synthetic teardown draft"));
    assert!(native.finished.load(Ordering::SeqCst));
    assert!(native.ingress().is_none());
}

#[tokio::test]
async fn setup_pcm_backpressure_recovers_the_same_handle_and_accepts_fresh_audio() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut configuration = configuration();
    configuration.provider = ProviderKind::CustomOpenAIASR;
    configuration.target_language = TargetLanguage::Original;
    configuration.credentials =
        mimi_runtime::core::credentials::ProviderCredentials::CustomSpeech {
            endpoint: format!("ws://{}/v1/realtime", listener.local_addr().unwrap()),
            model: "synthetic-backpressure-recovery".into(),
            api_key: "synthetic-key".into(),
        };
    let native = fixture(configuration.validated().unwrap());
    const HANDLE: u64 = u64::MAX - 2;
    sessions().lock().unwrap().insert(HANDLE, native.clone());
    struct StopSession;
    impl Drop for StopSession {
        fn drop(&mut self) {
            let native = sessions().lock().unwrap().remove(&HANDLE);
            if let Some(native) = native {
                drop(native.begin_stop());
            }
        }
    }
    let _cleanup = StopSession;
    let (first_setup, saw_first_setup) = oneshot::channel();
    let (release_setup, setup_released) = oneshot::channel();
    let (second_setup, saw_second_setup) = oneshot::channel();
    let (fresh_audio, saw_fresh_audio) = oneshot::channel();
    let server = tokio::spawn(async move {
        let mut first_setup = Some(first_setup);
        let mut setup_released = Some(setup_released);
        let mut second_setup = Some(second_setup);
        let mut fresh_audio = Some(fresh_audio);
        for attempt in 0..2 {
            let (stream, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let update = socket.next().await.unwrap().unwrap();
            let mut update: Value = serde_json::from_str(update.to_text().unwrap()).unwrap();
            assert_eq!(update["type"], "session.update");
            if attempt == 0 {
                first_setup.take().unwrap().send(()).unwrap();
                setup_released.take().unwrap().await.unwrap();
            }
            update["type"] = json!("session.updated");
            socket
                .send(Message::Text(update.to_string().into()))
                .await
                .unwrap();
            if attempt == 1 {
                second_setup.take().unwrap().send(()).unwrap();
            }
            loop {
                let message = tokio::time::timeout(Duration::from_secs(3), socket.next())
                    .await
                    .unwrap();
                match message {
                    Some(Ok(Message::Text(text))) => {
                        let message: Value = serde_json::from_str(&text).unwrap();
                        assert_eq!(message["type"], "input_audio_buffer.append");
                        if attempt == 1 {
                            let audio =
                                STANDARD.decode(message["audio"].as_str().unwrap()).unwrap();
                            fresh_audio.take().unwrap().send(audio).unwrap();
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    message => panic!("unexpected loopback message: {message:?}"),
                }
            }
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err()
        );
    });
    exchange(&json!({"operation":"start","handle":HANDLE}).to_string()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), saw_first_setup)
        .await
        .unwrap()
        .unwrap();
    let old_ingress = native.ingress().unwrap();
    let accepted = (0..22).filter(|_| pcm(HANDLE, vec![0, 0]).unwrap()).count();
    assert!(accepted <= 21);
    release_setup.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), saw_second_setup)
        .await
        .expect("the same native handle must reconnect after its PCM queue fails closed")
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while native.snapshot()["connectionStatus"] != "listening" {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    for _ in 0..10 {
        assert_eq!(
            old_ingress.try_send(vec![0, 0]),
            Err(AudioIngressError::Closed)
        );
    }
    // Custom ASR deliberately suppresses silence. One voiced 20 ms frame
    // proves that the replacement transport receives fresh PCM.
    let fresh = [0_u8, 16].repeat(480);
    assert!(pcm(HANDLE, fresh.clone()).unwrap());
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(3), saw_fresh_audio)
            .await
            .unwrap()
            .unwrap(),
        fresh
    );
    assert_eq!(native.epoch.load(Ordering::SeqCst), 2);
    exchange(&json!({"operation":"stop","handle":HANDLE}).to_string()).unwrap();
    assert_eq!(pcm(HANDLE, vec![0, 0]), Err("native_runtime_stale_handle"));
    server.await.unwrap();
}

#[tokio::test]
async fn native_pcm_reaches_the_real_shared_transport_and_stop_rejects_the_handle() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut configuration = configuration();
    configuration.provider = ProviderKind::CustomOpenAIASR;
    configuration.target_language = TargetLanguage::Original;
    configuration.credentials =
        mimi_runtime::core::credentials::ProviderCredentials::CustomSpeech {
            endpoint: format!("ws://{}/v1/realtime", listener.local_addr().unwrap()),
            model: "mock-transcribe".into(),
            api_key: "synthetic-key".into(),
        };
    let native = fixture(configuration.validated().unwrap());
    const HANDLE: u64 = u64::MAX - 1;
    sessions().lock().unwrap().insert(HANDLE, native.clone());
    let (audio_tx, audio_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let message = socket.next().await.unwrap().unwrap();
        let mut update: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
        assert_eq!(update["type"], "session.update");
        update["type"] = json!("session.updated");
        socket
            .send(Message::Text(update.to_string().into()))
            .await
            .unwrap();
        while let Some(Ok(message)) = socket.next().await {
            if !message.is_text() {
                continue;
            }
            let value: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
            if value["type"] == "input_audio_buffer.append" {
                let data = STANDARD.decode(value["audio"].as_str().unwrap()).unwrap();
                audio_tx.send(data).unwrap();
                break;
            }
        }
    });
    exchange(&json!({"operation":"start","handle":HANDLE}).to_string()).unwrap();
    assert!(pcm(HANDLE, [0_u8, 16].repeat(2400)).unwrap());
    let received = tokio::time::timeout(Duration::from_secs(3), audio_rx)
        .await
        .unwrap()
        .unwrap();
    assert!(!received.is_empty());
    let (samples, remainder) = received.as_chunks::<2>();
    assert!(remainder.is_empty(), "PCM ended with a partial sample");
    assert!(samples.iter().all(|sample| *sample == [0, 16]));
    exchange(&json!({"operation":"stop","handle":HANDLE}).to_string()).unwrap();
    assert_eq!(
        pcm(HANDLE, vec![0, 0]).unwrap_err(),
        "native_runtime_stale_handle"
    );
    assert_eq!(
        exchange(&json!({"operation":"poll","handle":HANDLE}).to_string()).unwrap_err(),
        "native_runtime_stale_handle"
    );
    server.await.unwrap();
}
