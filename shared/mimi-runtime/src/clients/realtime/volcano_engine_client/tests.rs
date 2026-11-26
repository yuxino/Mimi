#[tokio::test]
async fn handshake_authentication_rejections_preserve_transport_failures() {
    for (status, expected_auth_label) in
        crate::clients::transport::connection_diagnostics::handshake_failure_cases()
    {
        let (endpoint, server) =
            crate::clients::transport::connection_diagnostics::rejected_websocket_endpoint(status)
                .await;
        let (events, _receiver) = provider_event_channel();
        let mut client = VolcanoEngineClient::with_endpoint(
            "test-key-not-real",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            events,
            endpoint,
        )
        .unwrap();
        client.network = crate::clients::transport::provider_network::ProviderNetwork::resolve(
            &crate::core::services::network_proxy::ProxyConfig {
                mode: crate::core::services::network_proxy::ProxyMode::Direct,
                url: None,
            },
        )
        .unwrap();
        let error = client.connect().await.unwrap_err();
        server.await.unwrap();
        if let Some(expected_label) = expected_auth_label {
            assert_eq!(error, VolcanoEngineClientError::AuthenticationFailed);
            assert_eq!(error.to_string(), expected_label);
        } else {
            assert_eq!(error, VolcanoEngineClientError::TransportFailure);
        }
        assert!(!error.to_string().contains("private-handshake-body"));
        assert!(!client.inner.ready.load(Ordering::SeqCst));
    }
}

#[tokio::test]
async fn clear_removes_pending_text_but_tombstones_its_real_interval() {
    let (sender, mut receiver) = provider_event_channel();
    let client = VolcanoEngineClient::with_endpoint(
        "test-key-not-real",
        SourceLanguage::English,
        TargetLanguage::Japanese,
        sender.clone(),
        url::Url::parse("ws://127.0.0.1:1/translate").unwrap(),
    )
    .unwrap();
    {
        let mut committer = client.inner.committer.lock().await;
        committer.source_in_progress = true;
        committer.source_draft = "old source preview".into();
        committer.translation_draft = "old translation preview".into();
        committer
            .push_source("old source".into(), Some(0), Some(10))
            .unwrap();
    }
    sender
        .send(LiveTranslateServerEvent::SessionFinished)
        .unwrap();
    assert_eq!(client.clear_content().await, 1);
    assert_eq!(client.content_revision(), 1);
    let mut committer = client.inner.committer.lock().await;
    assert!(committer.sources.is_empty() && committer.translations.is_empty());
    assert!(committer.source_draft.is_empty() && committer.translation_draft.is_empty());
    assert_eq!(committer.pending_text_bytes, 0);
    assert!(committer.discard_source && committer.discard_translation);
    assert!(committer
        .push_translation("late old".into(), Some(0), Some(10))
        .unwrap()
        .is_none());
    assert!(committer
        .push_source("old replay".into(), Some(0), Some(10))
        .unwrap()
        .is_none());
    committer.discard_source = false;
    committer.discard_translation = false;
    committer
        .push_source("new source".into(), Some(11), Some(20))
        .unwrap();
    let pair = committer
        .push_translation("new translation".into(), Some(11), Some(20))
        .unwrap();
    assert!(
        matches!(pair, Some(LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {source,translation,..} | LiveTranslateServerEvent::SubtitleFinalPair {source,translation,..})
        if source == "new source" && translation == "new translation")
    );
    drop(committer);
    assert_eq!(
        receiver.recv().await,
        Some(LiveTranslateServerEvent::SessionFinished)
    );
}

use super::*;
use crate::clients::transport::provider_events::{provider_event_channel, ProviderEventReceiver};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;

const EVENT_START_SESSION: u32 = 100;
const EVENT_FINISH_SESSION: u32 = 102;
const EVENT_SESSION_STARTED: u32 = 150;
const EVENT_SESSION_FINISHED: u32 = 152;
const EVENT_SESSION_FAILED: u32 = 153;
const EVENT_TASK_REQUEST: u32 = 200;
const EVENT_SOURCE_SUBTITLE_START: u32 = 650;
const EVENT_SOURCE_SUBTITLE_RESPONSE: u32 = 651;
const EVENT_SOURCE_SUBTITLE_END: u32 = 652;
const EVENT_TRANSLATION_SUBTITLE_START: u32 = 653;
const EVENT_TRANSLATION_SUBTITLE_RESPONSE: u32 = 654;
const EVENT_TRANSLATION_SUBTITLE_END: u32 = 655;

async fn test_client(
    server: impl FnOnce(
            WebSocketStream<TcpStream>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        + Send
        + 'static,
) -> (VolcanoEngineClient, ProviderEventReceiver) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        server(socket).await;
    });
    let (events, receiver) = provider_event_channel();
    let endpoint = url::Url::parse(&format!("ws://{address}/translate")).unwrap();
    let client = VolcanoEngineClient::with_endpoint(
        "volcano-test-key-not-real",
        SourceLanguage::English,
        TargetLanguage::Japanese,
        events,
        endpoint,
    )
    .unwrap();
    (client, receiver)
}

fn write_varint(output: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        output.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    output.push(value as u8);
}

fn write_varint_field(output: &mut Vec<u8>, field: u32, value: u64) {
    write_varint(output, u64::from(field) << 3);
    write_varint(output, value);
}

fn write_bytes_field(output: &mut Vec<u8>, field: u32, value: &[u8]) {
    write_varint(output, (u64::from(field) << 3) | 2);
    write_varint(output, value.len() as u64);
    output.extend_from_slice(value);
}

fn server_event(event: u64, text: Option<&str>) -> Vec<u8> {
    let mut message = Vec::new();
    write_varint_field(&mut message, 2, event);
    if let Some(text) = text {
        write_bytes_field(&mut message, 4, text.as_bytes());
    }
    message
}

fn started_server_event(event: u64, start_time_ms: u64) -> Vec<u8> {
    let mut message = server_event(event, None);
    write_varint_field(&mut message, 5, start_time_ms);
    message
}

async fn ownership_context() -> (
    VolcanoEngineClient,
    ReceiveContext,
    ProviderEventReceiver,
    WebSocketStream<TcpStream>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = url::Url::parse(&format!(
        "ws://{}/translate",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let (connected, accepted) =
        tokio::join!(tokio_tungstenite::connect_async(endpoint.clone()), async {
            let (stream, _) = listener.accept().await.unwrap();
            tokio_tungstenite::accept_async(stream).await
        });
    let (socket, _) = connected.unwrap();
    let server = accepted.unwrap();
    let (_, stream) = socket.split();
    let (sender, receiver) = provider_event_channel();
    let client = VolcanoEngineClient::with_endpoint(
        "test-key-not-real",
        SourceLanguage::English,
        TargetLanguage::Japanese,
        sender.clone(),
        endpoint,
    )
    .unwrap();
    let (setup, _) = watch::channel(SetupState::Ready);
    let context = ReceiveContext {
        inner: Arc::clone(&client.inner),
        stream,
        events: sender,
        setup,
        generation: 0,
    };
    (client, context, receiver, server)
}

async fn handle_wire(context: &ReceiveContext, wire: Vec<u8>) {
    assert!(!handle_server_event(context, VolcanoEngineServerEvent::decode(&wire).unwrap()).await);
}

fn drain_into(
    events: &mut ProviderEventReceiver,
    controller: &mut crate::core::lifecycle::session::TranslationSessionController,
) {
    while let Ok(event) = events.try_recv() {
        controller.handle(event);
    }
}

#[tokio::test]
async fn ownership_keeps_admitted_and_queued_b_through_late_a_source_and_translation_end() {
    for drain_before_final in [false, true] {
        let (_client, context, mut events, _server) = ownership_context().await;
        let mut controller =
            crate::core::lifecycle::session::TranslationSessionController::default();
        for wire in [
            started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 100),
            server_event(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, Some("Synthetic A")),
            started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 300),
            server_event(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, Some("Synthetic B")),
            started_server_event(EVENT_TRANSLATION_SUBTITLE_START as u64, 300),
            server_event(EVENT_TRANSLATION_SUBTITLE_RESPONSE as u64, Some("合成 B")),
        ] {
            handle_wire(&context, wire).await;
            if drain_before_final {
                drain_into(&mut events, &mut controller);
            }
        }
        handle_wire(
            &context,
            timed_server_event(EVENT_SOURCE_SUBTITLE_END as u64, "Synthetic A.", 100, 200),
        )
        .await;
        handle_wire(
            &context,
            timed_server_event(EVENT_TRANSLATION_SUBTITLE_END as u64, "合成 A。", 100, 200),
        )
        .await;
        drain_into(&mut events, &mut controller);
        let snapshot = &controller.state.subtitles;
        assert_eq!(snapshot.source.text, "Synthetic B");
        assert_eq!(snapshot.translation.text, "合成 B");
        assert_eq!(
            snapshot.source.utterance_id,
            snapshot.translation.utterance_id
        );
        assert_eq!(snapshot.history.len(), 1);
        assert_eq!(snapshot.history[0].source, "Synthetic A.");
        assert!(controller.state.is_translation_pending);
        let committer = context.inner.committer.lock().await;
        assert_eq!(committer.source_draft, "Synthetic B");
        assert_eq!(committer.translation_draft, "合成 B");
        assert!(committer.source_in_progress && committer.translation_in_progress);
    }
}

#[tokio::test]
async fn ownership_proven_late_unseen_clock_does_not_become_a_newer_source() {
    let (_client, context, mut events, _server) = ownership_context().await;
    let mut controller = crate::core::lifecycle::session::TranslationSessionController::default();
    for wire in [
        started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 300),
        server_event(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, Some("Synthetic B")),
    ] {
        handle_wire(&context, wire).await;
        drain_into(&mut events, &mut controller);
    }
    for wire in [
        timed_server_event(
            EVENT_SOURCE_SUBTITLE_END as u64,
            "Synthetic unseen A.",
            100,
            200,
        ),
        timed_server_event(EVENT_TRANSLATION_SUBTITLE_END as u64, "合成 A。", 100, 200),
    ] {
        handle_wire(&context, wire).await;
        drain_into(&mut events, &mut controller);
    }
    assert_eq!(controller.state.subtitles.source.text, "Synthetic B");
    assert_eq!(controller.state.subtitles.history.len(), 1);
    assert_eq!(
        controller.state.subtitles.history[0].source,
        "Synthetic unseen A."
    );
}

#[tokio::test]
async fn ownership_omitted_zero_a_before_b_preserves_admitted_queued_and_stopping_b() {
    for (drain_before_final, stopping) in [(false, false), (true, false), (true, true)] {
        let (_client, context, mut events, _server) = ownership_context().await;
        let mut controller =
            crate::core::lifecycle::session::TranslationSessionController::default();
        for wire in [
            server_event(EVENT_SOURCE_SUBTITLE_START as u64, None),
            server_event(
                EVENT_SOURCE_SUBTITLE_RESPONSE as u64,
                Some("Synthetic zero A"),
            ),
            started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 1000),
            server_event(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, Some("Synthetic B")),
        ] {
            handle_wire(&context, wire).await;
            if drain_before_final {
                drain_into(&mut events, &mut controller);
            }
        }
        let b_owner = context.inner.committer.lock().await.source_owner.unwrap();
        if stopping {
            controller.begin_stopping();
        }
        for (event, text) in [
            (EVENT_SOURCE_SUBTITLE_END, "Synthetic zero A."),
            (EVENT_TRANSLATION_SUBTITLE_END, "合成 A。"),
        ] {
            let mut wire = server_event(event as u64, Some(text));
            write_varint_field(&mut wire, 6, 500);
            handle_wire(&context, wire).await;
        }
        drain_into(&mut events, &mut controller);
        let snapshot = &controller.state.subtitles;
        assert_eq!(snapshot.source.text, "Synthetic B");
        assert_eq!(snapshot.history.len(), 1);
        assert_eq!(snapshot.history[0].source, "Synthetic zero A.");
        let pair = snapshot.display_pair.as_ref().unwrap();
        assert_eq!(pair.source, "Synthetic zero A.");
        assert_ne!(snapshot.source.utterance_id, pair.utterance_id);
        let committer = context.inner.committer.lock().await;
        assert_eq!(committer.source_owner, Some(b_owner));
        assert_eq!(committer.source_draft, "Synthetic B");
        assert!(committer.source_in_progress);
        let b = committer
            .source_blocks
            .iter()
            .find(|block| block.owner == b_owner)
            .unwrap();
        assert_eq!(b.start_time_ms, Some(1000));
        assert!(!b.finished);
        assert!(
            committer.owner_for_start(Some(500)).is_none(),
            "END must not become START"
        );
    }
}

#[tokio::test]
async fn ownership_omitted_and_explicit_zero_starts_keep_first_preview_and_late_final_owned() {
    for omitted in [false, true] {
        for translation_first in [false, true] {
            let (_client, context, mut events, _server) = ownership_context().await;
            let mut controller =
                crate::core::lifecycle::session::TranslationSessionController::default();
            let start = |event| {
                if omitted {
                    server_event(event, None)
                } else {
                    started_server_event(event, 0)
                }
            };
            let source = [
                start(EVENT_SOURCE_SUBTITLE_START as u64),
                server_event(
                    EVENT_SOURCE_SUBTITLE_RESPONSE as u64,
                    Some("Synthetic zero A"),
                ),
            ];
            let translation = [
                start(EVENT_TRANSLATION_SUBTITLE_START as u64),
                server_event(EVENT_TRANSLATION_SUBTITLE_RESPONSE as u64, Some("合成零 A")),
            ];
            let lanes = if translation_first {
                [translation, source]
            } else {
                [source, translation]
            };
            for lane in lanes {
                for wire in lane {
                    handle_wire(&context, wire).await;
                    drain_into(&mut events, &mut controller);
                }
            }
            let snapshot = &controller.state.subtitles;
            assert_eq!(snapshot.source.text, "Synthetic zero A");
            assert_eq!(snapshot.translation.text, "合成零 A");
            assert!(snapshot.source.utterance_id.is_some());
            assert_eq!(
                snapshot.source.utterance_id,
                snapshot.translation.utterance_id
            );
            let preview = snapshot.realtime_preview.as_ref().unwrap();
            assert_eq!(preview.source.text, "Synthetic zero A");
            assert_eq!(preview.translation.text, "合成零 A");
            assert_eq!(
                preview.source.utterance_id,
                preview.translation.utterance_id
            );
            for wire in [
                started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 300),
                server_event(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, Some("Synthetic B")),
                started_server_event(EVENT_TRANSLATION_SUBTITLE_START as u64, 300),
                server_event(EVENT_TRANSLATION_SUBTITLE_RESPONSE as u64, Some("合成 B")),
            ] {
                handle_wire(&context, wire).await;
            }
            for (event, text) in [
                (EVENT_SOURCE_SUBTITLE_END, "Synthetic zero A."),
                (EVENT_TRANSLATION_SUBTITLE_END, "合成零 A。"),
            ] {
                let mut wire = server_event(event as u64, Some(text));
                if !omitted {
                    write_varint_field(&mut wire, 5, 0);
                }
                write_varint_field(&mut wire, 6, 200);
                handle_wire(&context, wire).await;
            }
            drain_into(&mut events, &mut controller);
            let snapshot = &controller.state.subtitles;
            assert_eq!(snapshot.source.text, "Synthetic B");
            assert_eq!(snapshot.translation.text, "合成 B");
            assert_eq!(
                snapshot.source.utterance_id,
                snapshot.translation.utterance_id
            );
            assert_eq!(snapshot.history.len(), 1);
            assert_eq!(snapshot.history[0].source, "Synthetic zero A.");
            assert_eq!(snapshot.history[0].translation, "合成零 A。");
        }
    }
}

#[tokio::test]
async fn ownership_empty_zero_duration_end_never_confirms_and_stop_keeps_a_real_tail() {
    for stopping in [false, true] {
        for tail_start in [0, 300] {
            let (_client, context, mut events, _server) = ownership_context().await;
            let mut controller =
                crate::core::lifecycle::session::TranslationSessionController::default();
            for event in [
                EVENT_SOURCE_SUBTITLE_START,
                EVENT_TRANSLATION_SUBTITLE_START,
                EVENT_SOURCE_SUBTITLE_END,
                EVENT_TRANSLATION_SUBTITLE_END,
            ] {
                handle_wire(&context, server_event(event as u64, None)).await;
                drain_into(&mut events, &mut controller);
            }
            assert!(controller.state.subtitles.history.is_empty());
            assert!(controller.state.subtitles.display_pair.is_none());
            for wire in [
                started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, tail_start),
                server_event(
                    EVENT_SOURCE_SUBTITLE_RESPONSE as u64,
                    Some("Synthetic real tail"),
                ),
                started_server_event(EVENT_TRANSLATION_SUBTITLE_START as u64, tail_start),
                server_event(
                    EVENT_TRANSLATION_SUBTITLE_RESPONSE as u64,
                    Some("合成真实尾句"),
                ),
            ] {
                handle_wire(&context, wire).await;
                drain_into(&mut events, &mut controller);
            }
            let preview = controller
                .state
                .subtitles
                .realtime_preview
                .as_ref()
                .unwrap();
            assert_eq!(preview.source.text, "Synthetic real tail");
            assert_eq!(preview.translation.text, "合成真实尾句");
            assert_eq!(
                preview.source.utterance_id,
                preview.translation.utterance_id
            );
            if stopping {
                controller.begin_stopping();
            }
            for wire in [
                server_event(EVENT_SOURCE_SUBTITLE_END as u64, None),
                server_event(EVENT_TRANSLATION_SUBTITLE_END as u64, None),
                timed_server_event(
                    EVENT_SOURCE_SUBTITLE_END as u64,
                    "Synthetic real tail.",
                    tail_start,
                    400,
                ),
                timed_server_event(
                    EVENT_TRANSLATION_SUBTITLE_END as u64,
                    "合成真实尾句。",
                    tail_start,
                    400,
                ),
            ] {
                handle_wire(&context, wire).await;
                drain_into(&mut events, &mut controller);
            }
            assert_eq!(controller.state.subtitles.history.len(), 1);
            assert_eq!(
                controller.state.subtitles.history[0].source,
                "Synthetic real tail."
            );
            assert!(controller.state.subtitles.display_pair_final);
        }
    }
}

#[tokio::test]
async fn ownership_zero_start_cannot_reopen_pending_confirmed_or_older_source() {
    let (_client, context, mut events, _server) = ownership_context().await;
    let mut controller = crate::core::lifecycle::session::TranslationSessionController::default();
    for wire in [
        server_event(EVENT_SOURCE_SUBTITLE_START as u64, None),
        server_event(
            EVENT_SOURCE_SUBTITLE_RESPONSE as u64,
            Some("Synthetic zero A"),
        ),
        timed_server_event(
            EVENT_SOURCE_SUBTITLE_END as u64,
            "Synthetic zero A.",
            0,
            200,
        ),
        server_event(EVENT_SOURCE_SUBTITLE_START as u64, None),
        server_event(
            EVENT_SOURCE_SUBTITLE_RESPONSE as u64,
            Some("Synthetic pending replay"),
        ),
    ] {
        handle_wire(&context, wire).await;
        drain_into(&mut events, &mut controller);
    }
    assert_eq!(controller.state.subtitles.source.text, "Synthetic zero A.");
    assert!(context.inner.committer.lock().await.discard_source);
    handle_wire(
        &context,
        timed_server_event(EVENT_TRANSLATION_SUBTITLE_END as u64, "合成零 A。", 0, 200),
    )
    .await;
    drain_into(&mut events, &mut controller);
    for wire in [
        server_event(EVENT_SOURCE_SUBTITLE_START as u64, None),
        server_event(
            EVENT_SOURCE_SUBTITLE_RESPONSE as u64,
            Some("Synthetic confirmed replay"),
        ),
    ] {
        handle_wire(&context, wire).await;
        drain_into(&mut events, &mut controller);
    }
    assert_eq!(controller.state.subtitles.history.len(), 1);
    assert_eq!(controller.state.subtitles.source.text, "Synthetic zero A.");
    for wire in [
        started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 300),
        server_event(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, Some("Synthetic B")),
        server_event(EVENT_SOURCE_SUBTITLE_START as u64, None),
        server_event(
            EVENT_SOURCE_SUBTITLE_RESPONSE as u64,
            Some("Synthetic older replay"),
        ),
    ] {
        handle_wire(&context, wire).await;
        drain_into(&mut events, &mut controller);
    }
    assert_eq!(controller.state.subtitles.source.text, "Synthetic B");
    assert_eq!(
        context.inner.committer.lock().await.source_draft,
        "Synthetic B"
    );
}

#[tokio::test]
async fn ownership_internal_unknown_start_does_not_claim_late_a_translation_end() {
    let (_client, context, mut events, _server) = ownership_context().await;
    let mut controller = crate::core::lifecycle::session::TranslationSessionController::default();
    for wire in [
        started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 100),
        started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 300),
        server_event(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, Some("Synthetic B")),
    ] {
        handle_wire(&context, wire).await;
        drain_into(&mut events, &mut controller);
    }
    // Internal unknown timing is a synthetic compatibility case. An
    // omitted scalar in an actual START wire frame means zero instead.
    assert!(
        !handle_server_event(
            &context,
            VolcanoEngineServerEvent::TranslationSubtitleStarted {
                start_time_ms: None
            }
        )
        .await
    );
    for wire in [
        server_event(EVENT_TRANSLATION_SUBTITLE_RESPONSE as u64, Some("匿名 B")),
        timed_server_event(EVENT_SOURCE_SUBTITLE_END as u64, "Synthetic A.", 100, 200),
        timed_server_event(EVENT_TRANSLATION_SUBTITLE_END as u64, "合成 A。", 100, 200),
    ] {
        handle_wire(&context, wire).await;
        drain_into(&mut events, &mut controller);
    }
    assert_eq!(controller.state.subtitles.source.text, "Synthetic B");
    assert_eq!(controller.state.subtitles.history.len(), 1);
    {
        let committer = context.inner.committer.lock().await;
        assert_eq!(committer.translation_draft, "匿名 B");
        assert!(committer.translation_in_progress);
        assert!(committer.translation_owner.is_none());
        assert!(committer.translation_start_time_ms.is_none());
    }
    handle_wire(
        &context,
        server_event(EVENT_TRANSLATION_SUBTITLE_RESPONSE as u64, Some(" 后续")),
    )
    .await;
    let committer = context.inner.committer.lock().await;
    assert_eq!(committer.translation_draft, "匿名 B 后续");
    assert!(committer.translation_owner.is_none());
}

#[tokio::test]
async fn ownership_no_start_legacy_end_barrier_keeps_successive_blocks_separate() {
    let (_client, context, mut events, _server) = ownership_context().await;
    let mut controller = crate::core::lifecycle::session::TranslationSessionController::default();
    for (source, translation, start, end) in [
        ("Synthetic A", "合成 A", 100, 200),
        ("Synthetic B", "合成 B", 300, 400),
    ] {
        handle_wire(
            &context,
            server_event(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, Some(source)),
        )
        .await;
        handle_wire(
            &context,
            server_event(
                EVENT_TRANSLATION_SUBTITLE_RESPONSE as u64,
                Some(translation),
            ),
        )
        .await;
        drain_into(&mut events, &mut controller);
        assert_eq!(controller.state.subtitles.source.text, source);
        assert_eq!(controller.state.subtitles.translation.text, translation);
        assert!(controller.state.subtitles.source.utterance_id.is_none());
        assert!(controller
            .state
            .subtitles
            .translation
            .utterance_id
            .is_none());
        handle_wire(
            &context,
            timed_server_event(EVENT_SOURCE_SUBTITLE_END as u64, source, start, end),
        )
        .await;
        handle_wire(
            &context,
            timed_server_event(
                EVENT_TRANSLATION_SUBTITLE_END as u64,
                translation,
                start,
                end,
            ),
        )
        .await;
        drain_into(&mut events, &mut controller);
        assert_eq!(controller.state.subtitles.source.text, source);
        assert_eq!(controller.state.subtitles.translation.text, translation);
        let committer = context.inner.committer.lock().await;
        assert!(committer.source_draft.is_empty() && committer.translation_draft.is_empty());
        assert!(!committer.source_in_progress && !committer.translation_in_progress);
        assert!(committer.translation_owner.is_none());
        assert!(committer.translation_start_time_ms.is_none());
        assert!(
            committer.last_matched_source_owner.is_none(),
            "legacy END cannot infer ownership"
        );
    }
    assert_eq!(controller.state.subtitles.history.len(), 2);
    assert_eq!(controller.state.subtitles.history[1].source, "Synthetic B");
}

#[tokio::test]
async fn ownership_empty_boundary_retains_completed_a_without_labeling_its_text_b() {
    let (_client, context, mut events, _server) = ownership_context().await;
    let mut controller = crate::core::lifecycle::session::TranslationSessionController::default();
    for wire in [
        started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 100),
        server_event(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, Some("Synthetic A")),
        started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 300),
        timed_server_event(EVENT_SOURCE_SUBTITLE_END as u64, "Synthetic A.", 100, 200),
        timed_server_event(EVENT_TRANSLATION_SUBTITLE_END as u64, "合成 A。", 100, 200),
    ] {
        handle_wire(&context, wire).await;
        drain_into(&mut events, &mut controller);
    }
    let snapshot = &controller.state.subtitles;
    assert_eq!(snapshot.source.text, "Synthetic A.");
    let pair = snapshot.display_pair.as_ref().unwrap();
    assert_eq!(pair.source, "Synthetic A.");
    assert_eq!(snapshot.source.utterance_id, pair.utterance_id);
    assert!(snapshot.display_pair_final);
}

#[tokio::test]
async fn ownership_translation_first_start_binds_only_when_source_time_is_proved() {
    let (_client, context, mut events, _server) = ownership_context().await;
    let mut controller = crate::core::lifecycle::session::TranslationSessionController::default();
    for wire in [
        started_server_event(EVENT_TRANSLATION_SUBTITLE_START as u64, 0),
        server_event(EVENT_TRANSLATION_SUBTITLE_RESPONSE as u64, Some("合成")),
    ] {
        handle_wire(&context, wire).await;
        drain_into(&mut events, &mut controller);
    }
    assert!(controller
        .state
        .subtitles
        .translation
        .utterance_id
        .is_none());
    for wire in [
        started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 0),
        server_event(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, Some("Synthetic")),
    ] {
        handle_wire(&context, wire).await;
        drain_into(&mut events, &mut controller);
    }
    let snapshot = &controller.state.subtitles;
    assert!(snapshot.source.utterance_id.is_some());
    assert_eq!(
        snapshot.source.utterance_id,
        snapshot.translation.utterance_id
    );
    assert_eq!(snapshot.translation.text, "合成");
}

#[test]
fn ownership_internal_unknown_time_never_guesses_the_translation_owner() {
    let mut committer = VolcanoSubtitlePairCommitter::new(SourceLanguage::English);
    assert!(committer.start_source(None));
    let owner = committer.source_owner.unwrap();
    assert!(committer.start_translation(Some(100)));
    assert!(committer.translation_owner.is_none());
    assert_eq!(committer.source_end_owner(Some(100), Some(200)), None);
    assert_eq!(committer.translation_owner, None);
    assert_eq!(committer.source_owner, Some(owner));
    assert_eq!(committer.source_blocks[0].start_time_ms, None);
    assert!(!committer.source_blocks[0].finished);
    committer.source_draft = "Synthetic anonymous B".into();
    committer
        .push_source("Synthetic unknown A.".into(), Some(100), Some(200))
        .unwrap();
    assert!(matches!(
        committer
            .push_translation("合成 A。".into(), Some(100), Some(200))
            .unwrap(),
        Some(LiveTranslateServerEvent::SubtitleFinalPair { .. })
    ));
    assert_eq!(committer.source_draft, "Synthetic anonymous B");
    assert!(!committer.source_blocks[0].finished);
    committer.reset();
    let owner = committer.ensure_source_owner().unwrap();
    committer.translation_draft = "Unowned synthetic translation".into();
    committer.bind_translation_owner();
    assert!(committer.translation_owner.is_none());
    assert!(matches!(
        committer.translation_text_event(),
        LiveTranslateServerEvent::TranslationDraft(_)
    ));
    assert_eq!(committer.source_end_owner(Some(0), None), None);
    committer.bind_translation_owner();
    assert!(committer.translation_owner.is_none());
    assert_eq!(committer.source_blocks[0].start_time_ms, None);
    assert!(matches!(
        committer.source_text_event(owner, "Synthetic".into(), false),
        LiveTranslateServerEvent::SourceDraft { .. }
    ));
    assert!(committer.start_source(Some(0)));
    assert!(committer.start_translation(Some(0)));
    assert_eq!(committer.translation_owner, committer.source_owner);
}

#[test]
fn ownership_duplicate_and_retired_starts_preserve_bounded_buffers() {
    let mut committer = VolcanoSubtitlePairCommitter::new(SourceLanguage::English);
    assert!(committer.start_source(Some(0)));
    committer.source_draft = "Synthetic".into();
    assert!(!committer.start_source(Some(0)));
    assert_eq!(committer.source_draft, "Synthetic");
    assert!(committer.start_translation(Some(0)));
    committer.translation_draft = "合成".into();
    assert!(!committer.start_translation(Some(0)));
    assert_eq!(committer.translation_draft, "合成");
    for start in 0..(MAXIMUM_SOURCE_BLOCKS as i32 * 4) {
        let start = start * 10;
        committer.start_source(Some(start));
        committer
            .push_source("S".into(), Some(start), Some(start + 1))
            .unwrap();
        committer
            .push_translation("T".into(), Some(start), Some(start + 1))
            .unwrap();
    }
    assert!(committer.source_blocks.len() <= MAXIMUM_SOURCE_BLOCKS);
    assert!(committer.retired_starts.len() <= MAXIMUM_SOURCE_BLOCKS);
    assert!(committer.recently_committed.len() <= MAXIMUM_RECENT_COMMITTED_TIMINGS);
    assert!(!committer.start_source(Some(0)));
    assert!(!committer.start_translation(Some(0)));
    committer.reset();
    committer.next_source_owner = u64::from(u32::MAX);
    assert!(committer.new_source_owner(Some(0)).is_none());
    assert!(committer.source_blocks.is_empty());
}

#[tokio::test]
async fn ownership_retired_start_conservatively_drops_unidentified_drafts_until_known_start() {
    let (_client, context, mut events, _server) = ownership_context().await;
    let mut controller = crate::core::lifecycle::session::TranslationSessionController::default();
    for wire in [
        started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 0),
        timed_server_event(EVENT_SOURCE_SUBTITLE_END as u64, "Synthetic A.", 0, 10),
        timed_server_event(EVENT_TRANSLATION_SUBTITLE_END as u64, "合成 A。", 0, 10),
        started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 20),
        server_event(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, Some("Synthetic B")),
        started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 0),
        server_event(
            EVENT_SOURCE_SUBTITLE_RESPONSE as u64,
            Some(" ambiguous tail"),
        ),
    ] {
        handle_wire(&context, wire).await;
        drain_into(&mut events, &mut controller);
    }
    assert_eq!(controller.state.subtitles.source.text, "Synthetic B");
    assert_eq!(
        context.inner.committer.lock().await.source_draft,
        "Synthetic B"
    );
    // With no ID on RESPONSE, the old A tail and a new B tail are
    // indistinguishable. Preserve B's buffer but suppress both until START.
    handle_wire(
        &context,
        started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 20),
    )
    .await;
    handle_wire(
        &context,
        server_event(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, Some(" resumed")),
    )
    .await;
    drain_into(&mut events, &mut controller);
    assert_eq!(
        controller.state.subtitles.source.text,
        "Synthetic B resumed"
    );
}

#[tokio::test]
async fn ownership_clear_and_reconnect_reject_retired_frames_and_refresh_epoch() {
    let (client, mut context, mut events, _server) = ownership_context().await;
    handle_wire(
        &context,
        started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 0),
    )
    .await;
    handle_wire(
        &context,
        server_event(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, Some("Synthetic old")),
    )
    .await;
    let old_id = context.inner.committer.lock().await.owner_id(1);
    while events.try_recv().is_ok() {}
    client.clear_content().await;
    for wire in [
        started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 0),
        server_event(
            EVENT_SOURCE_SUBTITLE_RESPONSE as u64,
            Some("Synthetic replay"),
        ),
        timed_server_event(EVENT_SOURCE_SUBTITLE_END as u64, "Synthetic old.", 0, 10),
        timed_server_event(EVENT_TRANSLATION_SUBTITLE_END as u64, "合成旧句。", 0, 10),
    ] {
        handle_wire(&context, wire).await;
    }
    assert!(events.try_recv().is_err());
    handle_wire(
        &context,
        started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 20),
    )
    .await;
    assert!(events.try_recv().is_ok());
    client.disconnect().await;
    handle_wire(
        &context,
        started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 30),
    )
    .await;
    assert!(events.try_recv().is_err());
    context.generation = context.inner.generation.load(Ordering::SeqCst);
    handle_wire(
        &context,
        started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 0),
    )
    .await;
    let new_id = context.inner.committer.lock().await.owner_id(1);
    assert_ne!(old_id, new_id);
    assert!(events.try_recv().is_ok());
}

fn timed_server_event(event: u64, text: &str, start_time_ms: u64, end_time_ms: u64) -> Vec<u8> {
    let mut message = server_event(event, Some(text));
    write_varint_field(&mut message, 5, start_time_ms);
    write_varint_field(&mut message, 6, end_time_ms);
    message
}

fn failed_server_event(status: u64, private_message: &str) -> Vec<u8> {
    let mut meta = Vec::new();
    write_varint_field(&mut meta, 3, status);
    write_bytes_field(&mut meta, 4, private_message.as_bytes());
    let mut message = Vec::new();
    write_bytes_field(&mut message, 1, &meta);
    write_varint_field(&mut message, 2, EVENT_SESSION_FAILED as u64);
    message
}

fn read_varint(input: &mut &[u8]) -> Option<u64> {
    let mut value = 0_u64;
    for shift in (0..70).step_by(7) {
        let byte = *input.first()?;
        *input = &input[1..];
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

fn find_varint(message: &[u8], wanted_field: u32) -> Option<u64> {
    let mut input = message;
    while !input.is_empty() {
        let key = read_varint(&mut input)?;
        let field = u32::try_from(key >> 3).ok()?;
        let wire = (key & 7) as u8;
        match wire {
            0 => {
                let value = read_varint(&mut input)?;
                if field == wanted_field {
                    return Some(value);
                }
            }
            2 => {
                let length = usize::try_from(read_varint(&mut input)?).ok()?;
                if length > input.len() {
                    return None;
                }
                input = &input[length..];
            }
            _ => return None,
        }
    }
    None
}

fn find_bytes(message: &[u8], wanted_field: u32) -> Option<&[u8]> {
    let mut input = message;
    while !input.is_empty() {
        let key = read_varint(&mut input)?;
        let field = u32::try_from(key >> 3).ok()?;
        let wire = (key & 7) as u8;
        match wire {
            0 => {
                read_varint(&mut input)?;
            }
            2 => {
                let length = usize::try_from(read_varint(&mut input)?).ok()?;
                if length > input.len() {
                    return None;
                }
                let (value, rest) = input.split_at(length);
                input = rest;
                if field == wanted_field {
                    return Some(value);
                }
            }
            _ => return None,
        }
    }
    None
}

fn binary(message: Message) -> Vec<u8> {
    match message {
        Message::Binary(frame) => frame.to_vec(),
        other => panic!("expected binary protobuf frame, got {other:?}"),
    }
}

fn assert_pair(event: SubtitleCommitResult, expected_source: &str, expected_translation: &str) {
    assert!(matches!(
        event,
        Ok(Some(LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
            source,
            language,
            translation,
            ..
        } | LiveTranslateServerEvent::SubtitleFinalPair {
            source,
            language,
            translation,
            ..
        })) if source == expected_source
            && language.as_deref() == Some("en")
            && translation == expected_translation
    ));
}

#[test]
fn final_pair_committer_accepts_either_final_arrival_order() {
    let mut source_first = VolcanoSubtitlePairCommitter::new(SourceLanguage::English);
    assert!(source_first
        .push_source("one".into(), Some(10), Some(90))
        .unwrap()
        .is_none());
    assert_pair(
        source_first.push_translation("一".into(), Some(10), Some(90)),
        "one",
        "一",
    );

    let mut translation_first = VolcanoSubtitlePairCommitter::new(SourceLanguage::English);
    assert!(translation_first
        .push_translation("二".into(), Some(100), Some(190))
        .unwrap()
        .is_none());
    assert_pair(
        translation_first.push_source("two".into(), Some(100), Some(190)),
        "two",
        "二",
    );
}

#[test]
fn timing_identity_prevents_cross_sentence_pairing_during_reordering() {
    let mut committer = VolcanoSubtitlePairCommitter::new(SourceLanguage::English);
    assert!(committer
        .push_source("sentence A".into(), Some(0), Some(100))
        .unwrap()
        .is_none());
    assert!(committer
        .push_source("sentence B".into(), Some(100), Some(200))
        .unwrap()
        .is_none());

    // B completes first, followed by A. Each translation must select the
    // source with the same official input interval rather than the oldest
    // or most recently received source.
    assert_pair(
        committer.push_translation("译文 B".into(), Some(100), Some(200)),
        "sentence B",
        "译文 B",
    );
    assert_pair(
        committer.push_translation("译文 A".into(), Some(0), Some(100)),
        "sentence A",
        "译文 A",
    );
}

#[test]
fn mismatched_or_incomplete_tails_are_never_forged_into_a_pair() {
    let mut committer = VolcanoSubtitlePairCommitter::new(SourceLanguage::English);
    assert!(committer
        .push_source("old source".into(), Some(0), Some(100))
        .unwrap()
        .is_none());
    assert!(committer
        .push_translation("new translation".into(), Some(100), Some(200))
        .unwrap()
        .is_none());
    assert!(committer
        .push_source("missing timing".into(), None, None)
        .unwrap()
        .is_none());

    committer.reset();
    assert!(committer.sources.is_empty());
    assert!(committer.translations.is_empty());
    assert!(committer
        .push_translation("late unmatched tail".into(), Some(0), Some(100))
        .unwrap()
        .is_none());
}

#[test]
fn a_committed_timing_cannot_emit_a_duplicate_pair() {
    let mut committer = VolcanoSubtitlePairCommitter::new(SourceLanguage::English);
    assert!(committer
        .push_source("once".into(), Some(0), Some(100))
        .unwrap()
        .is_none());
    assert_pair(
        committer.push_translation("一次".into(), Some(0), Some(100)),
        "once",
        "一次",
    );
    assert!(committer
        .push_source("duplicate source".into(), Some(0), Some(100))
        .unwrap()
        .is_none());
    assert!(committer
        .push_translation("重复译文".into(), Some(0), Some(100))
        .unwrap()
        .is_none());
    assert!(committer.sources.is_empty());
    assert!(committer.translations.is_empty());
}

#[test]
fn pending_final_text_budget_resets_and_allows_the_next_pair() {
    let mut committer = VolcanoSubtitlePairCommitter::new(SourceLanguage::English);
    let oversized_half = MAXIMUM_PENDING_FINAL_TEXT_BYTES / 2 + 1;
    assert!(committer
        .push_source("s".repeat(oversized_half), Some(0), Some(100))
        .unwrap()
        .is_none());
    assert_eq!(committer.pending_text_bytes, oversized_half);

    assert_eq!(
        committer.push_translation("t".repeat(oversized_half), Some(100), Some(200),),
        Err(PendingFinalTextLimitExceeded)
    );
    assert!(committer.sources.is_empty());
    assert!(committer.translations.is_empty());
    assert_eq!(committer.pending_text_bytes, 0);

    assert!(committer
        .push_source("next sentence".into(), Some(200), Some(300))
        .unwrap()
        .is_none());
    assert_pair(
        committer.push_translation("次の文".into(), Some(200), Some(300)),
        "next sentence",
        "次の文",
    );
    assert_eq!(committer.pending_text_bytes, 0);
}

#[test]
fn constructor_validates_languages_without_exposing_the_key() {
    let secret = "volcano-private-key-never-log";
    let (events, _receiver) = provider_event_channel();
    let result = VolcanoEngineClient::with_endpoint(
        secret,
        SourceLanguage::Automatic,
        TargetLanguage::English,
        events,
        url::Url::parse("ws://127.0.0.1:9/translate").unwrap(),
    );
    let error = match result {
        Ok(_) => panic!("automatic source language must be rejected"),
        Err(error) => error,
    };
    assert_eq!(error, VolcanoEngineClientError::UnsupportedSourceLanguage);
    assert!(!format!("{error:?}").contains(secret));
}

#[allow(clippy::result_large_err)]
#[tokio::test]
async fn websocket_upgrade_uses_the_current_api_key_headers() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_hdr_async(
            stream,
            |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                assert_eq!(
                    request.headers().get("X-Api-Key").unwrap(),
                    "volcano-current-api-key-not-real"
                );
                assert_eq!(
                    request.headers().get("X-Api-Resource-Id").unwrap(),
                    VolcanoEngineEndpoint::RESOURCE_ID
                );
                Ok(response)
            },
        )
        .await
        .unwrap();
        let _ = socket.next().await;
        socket
            .send(Message::Binary(
                server_event(EVENT_SESSION_STARTED as u64, None).into(),
            ))
            .await
            .unwrap();
        while socket.next().await.is_some() {}
    });

    let (events, _receiver) = provider_event_channel();
    let client = VolcanoEngineClient::with_endpoint(
        "volcano-current-api-key-not-real",
        SourceLanguage::English,
        TargetLanguage::Japanese,
        events,
        url::Url::parse(&format!("ws://{address}/translate")).unwrap(),
    )
    .unwrap();
    client
        .connect_with_timeout(Duration::from_millis(500))
        .await
        .unwrap();
    client.disconnect().await;
}

#[tokio::test]
async fn connect_waits_for_the_official_session_started_event() {
    let (client, _events) = test_client(|mut socket| {
        Box::pin(async move {
            let start = binary(socket.next().await.unwrap().unwrap());
            assert_eq!(find_varint(&start, 2), Some(EVENT_START_SESSION as u64));
            socket
                .send(Message::Binary(
                    server_event(EVENT_SESSION_STARTED as u64, None).into(),
                ))
                .await
                .unwrap();
            while socket.next().await.is_some() {}
        })
    })
    .await;
    client
        .connect_with_timeout(Duration::from_millis(500))
        .await
        .unwrap();
    client.disconnect().await;
}

#[tokio::test]
async fn offline_reading_boundary_volcano_staggered_drafts_preserve_newer_owner() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = url::Url::parse(&format!(
        "ws://{}/translate",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let (connected, accepted) =
        tokio::join!(tokio_tungstenite::connect_async(endpoint.clone()), async {
            let (stream, _) = listener.accept().await.unwrap();
            tokio_tungstenite::accept_async(stream).await
        });
    let (socket, _) = connected.unwrap();
    let _server = accepted.unwrap();
    let (_, stream) = socket.split();
    let (sender, mut events) = provider_event_channel();
    let client = VolcanoEngineClient::with_endpoint(
        "test-key-not-real",
        SourceLanguage::English,
        TargetLanguage::Japanese,
        sender.clone(),
        endpoint,
    )
    .unwrap();
    let (setup, _) = watch::channel(SetupState::Ready);
    let context = ReceiveContext {
        inner: Arc::clone(&client.inner),
        stream,
        events: sender,
        setup,
        generation: 0,
    };
    let mut controller = crate::core::lifecycle::session::TranslationSessionController::default();
    let frames = [
        started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 100),
        server_event(
            EVENT_SOURCE_SUBTITLE_RESPONSE as u64,
            Some("Synthetic sentence A"),
        ),
        timed_server_event(
            EVENT_SOURCE_SUBTITLE_END as u64,
            "Synthetic sentence A.",
            100,
            200,
        ),
        started_server_event(EVENT_TRANSLATION_SUBTITLE_START as u64, 100),
        server_event(EVENT_TRANSLATION_SUBTITLE_RESPONSE as u64, Some("合成訳 A")),
        started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 300),
        server_event(
            EVENT_SOURCE_SUBTITLE_RESPONSE as u64,
            Some("Synthetic sentence B"),
        ),
        timed_server_event(
            EVENT_TRANSLATION_SUBTITLE_END as u64,
            "合成訳 A。",
            100,
            200,
        ),
        started_server_event(EVENT_TRANSLATION_SUBTITLE_START as u64, 300),
        server_event(EVENT_TRANSLATION_SUBTITLE_RESPONSE as u64, Some("合成訳 B")),
        timed_server_event(
            EVENT_SOURCE_SUBTITLE_END as u64,
            "Synthetic sentence B.",
            300,
            400,
        ),
        timed_server_event(
            EVENT_TRANSLATION_SUBTITLE_END as u64,
            "合成訳 B。",
            300,
            400,
        ),
    ];
    for (index, frame) in frames.into_iter().enumerate() {
        assert!(
            !handle_server_event(&context, VolcanoEngineServerEvent::decode(&frame).unwrap()).await
        );
        while let Ok(event) = events.try_recv() {
            controller.handle(event);
        }
        if index == 6 {
            // A's owned translation cannot accompany B's current original.
            let preview = controller
                .state
                .subtitles
                .realtime_preview
                .as_ref()
                .unwrap();
            assert_eq!(preview.source.text, "Synthetic sentence B");
            assert_eq!(preview.translation.text, "");
            assert!(preview.source.utterance_id.is_some());
            assert!(controller.state.subtitles.history.is_empty());
        }
        if index == 7 {
            // A stays durable while the newer B original remains live.
            assert_eq!(
                controller.state.subtitles.source.text,
                "Synthetic sentence B"
            );
            assert_eq!(controller.state.subtitles.history.len(), 1);
        }
    }
    let history = &controller.state.subtitles.history;
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].source, "Synthetic sentence A.");
    assert_eq!(history[0].translation, "合成訳 A。");
    assert_eq!(history[1].source, "Synthetic sentence B.");
    assert_eq!(history[1].translation, "合成訳 B。");
    assert_eq!(
        controller.state.subtitles.source.text,
        "Synthetic sentence B."
    );
    assert_eq!(controller.state.subtitles.translation.text, "合成訳 B。");
}

#[tokio::test]
async fn shared_incremental_subtitle_sequences_keep_full_previews_and_sentence_boundaries() {
    let contract: serde_json::Value =
        serde_json::from_str(include_str!("../../../../../translation-contracts.json")).unwrap();
    for case in contract["volcanoSubtitleSequences"].as_array().unwrap() {
        let frames = case["frames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|hex| {
                let hex = hex.as_str().unwrap();
                (0..hex.len())
                    .step_by(2)
                    .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).unwrap())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = url::Url::parse(&format!(
            "ws://{}/translate",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let (connected, accepted) =
            tokio::join!(tokio_tungstenite::connect_async(endpoint.clone()), async {
                let (stream, _) = listener.accept().await.unwrap();
                tokio_tungstenite::accept_async(stream).await
            });
        let (socket, _) = connected.unwrap();
        let _server = accepted.unwrap();
        let (_, stream) = socket.split();
        let (sender, mut events) = provider_event_channel();
        let client = VolcanoEngineClient::with_endpoint(
            "test-key-not-real",
            SourceLanguage::Japanese,
            TargetLanguage::SimplifiedChinese,
            sender.clone(),
            endpoint,
        )
        .unwrap();
        let (setup, _) = watch::channel(SetupState::Ready);
        let context = ReceiveContext {
            inner: Arc::clone(&client.inner),
            stream,
            events: sender,
            setup,
            generation: 0,
        };
        let mut source_drafts = Vec::new();
        let mut translation_drafts = Vec::new();
        let mut source_finals = Vec::new();
        let mut translation_finals = Vec::new();
        let expected = &case["expected"];
        // Drain after each real handler call. Draft transport intentionally
        // coalesces snapshots, so a burst socket test cannot assert every preview.
        for frame in frames {
            assert!(
                !handle_server_event(&context, VolcanoEngineServerEvent::decode(&frame).unwrap())
                    .await
            );
            while let Ok(event) = events.try_recv() {
                match event {
                    LiveTranslateServerEvent::UtteranceText {
                        role: UtteranceRole::Source,
                        is_final: false,
                        text,
                        ..
                    }
                    | LiveTranslateServerEvent::SourceDraft { text, .. } => {
                        source_drafts.push(text)
                    }
                    LiveTranslateServerEvent::UtteranceText {
                        role: UtteranceRole::Translation,
                        is_final: false,
                        text,
                        ..
                    }
                    | LiveTranslateServerEvent::TranslationDraft(text) => {
                        translation_drafts.push(text)
                    }
                    LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                        source,
                        translation,
                        ..
                    }
                    | LiveTranslateServerEvent::SubtitleFinalPair {
                        source,
                        translation,
                        ..
                    } => {
                        source_finals.push(source);
                        translation_finals.push(translation);
                    }
                    LiveTranslateServerEvent::Error { code, .. } => {
                        panic!("unexpected failure {code}")
                    }
                    _ => {}
                }
            }
        }
        assert_eq!(
            serde_json::json!({
                "sourceDrafts": source_drafts, "translationDrafts": translation_drafts,
                "sourceFinals": source_finals, "translationFinals": translation_finals,
            }),
            *expected,
            "{}",
            case["id"]
        );
    }
}

#[tokio::test]
async fn incremental_draft_overflow_is_bounded_on_both_sides() {
    for event in [
        EVENT_SOURCE_SUBTITLE_RESPONSE,
        EVENT_TRANSLATION_SUBTITLE_RESPONSE,
    ] {
        for character in ["a", "日"] {
            let (client, mut events) = test_client(move |mut socket| {
                Box::pin(async move {
                    socket.next().await.unwrap().unwrap();
                    socket
                        .send(Message::Binary(
                            server_event(EVENT_SESSION_STARTED as u64, None).into(),
                        ))
                        .await
                        .unwrap();
                    socket.next().await.unwrap().unwrap();
                    for text in [
                        character
                            .repeat(crate::core::models::MAX_SUBTITLE_TEXT_BYTES / character.len()),
                        character.into(),
                    ] {
                        socket
                            .send(Message::Binary(
                                server_event(event as u64, Some(&text)).into(),
                            ))
                            .await
                            .unwrap();
                    }
                    while socket.next().await.is_some() {}
                })
            })
            .await;
            client.connect().await.unwrap();
            client
                .send_audio(&vec![0; VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT])
                .await
                .unwrap();
            loop {
                let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
                    .await
                    .unwrap()
                    .unwrap();
                if let LiveTranslateServerEvent::Error { code, .. } = event {
                    assert_eq!(code, "volcano_transcript_safety_limit");
                    break;
                }
            }
            client.disconnect().await;
        }
    }
}

#[tokio::test]
async fn empty_subtitle_events_do_not_disconnect_or_block_the_next_final_pair() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            socket.next().await.unwrap().unwrap();
            socket
                .send(Message::Binary(
                    server_event(EVENT_SESSION_STARTED as u64, None).into(),
                ))
                .await
                .unwrap();
            // Send only after connect has completed, as with normal audio.
            socket.next().await.unwrap().unwrap();
            for event in [
                EVENT_SOURCE_SUBTITLE_RESPONSE,
                EVENT_SOURCE_SUBTITLE_END,
                EVENT_TRANSLATION_SUBTITLE_RESPONSE,
                EVENT_TRANSLATION_SUBTITLE_END,
            ] {
                for text in [None, Some("")] {
                    socket
                        .send(Message::Binary(server_event(event as u64, text).into()))
                        .await
                        .unwrap();
                }
            }
            for response in [
                timed_server_event(
                    EVENT_SOURCE_SUBTITLE_END as u64,
                    "Synthetic source.",
                    100,
                    200,
                ),
                timed_server_event(
                    EVENT_TRANSLATION_SUBTITLE_END as u64,
                    "合成テスト。",
                    100,
                    200,
                ),
            ] {
                socket.send(Message::Binary(response.into())).await.unwrap();
            }
            let finish = binary(socket.next().await.unwrap().unwrap());
            assert_eq!(find_varint(&finish, 2), Some(EVENT_FINISH_SESSION as u64));
            socket
                .send(Message::Binary(
                    server_event(EVENT_SESSION_FINISHED as u64, None).into(),
                ))
                .await
                .unwrap();
        })
    })
    .await;
    client.connect().await.unwrap();
    client
        .send_audio(&vec![0; VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT])
        .await
        .unwrap();
    loop {
        let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .unwrap()
            .unwrap();
        match event {
            LiveTranslateServerEvent::Error { code, .. } => panic!("unexpected failure {code}"),
            LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                source,
                translation,
                ..
            }
            | LiveTranslateServerEvent::SubtitleFinalPair {
                source,
                translation,
                ..
            } => {
                assert_eq!(source, "Synthetic source.");
                assert_eq!(translation, "合成テスト。");
                break;
            }
            _ => {}
        }
    }
    client.finish(Duration::from_secs(1)).await;
    let remaining = std::iter::from_fn(|| events.try_recv().ok()).collect::<Vec<_>>();
    assert!(remaining
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
    assert!(!remaining.iter().any(|event| matches!(
        event,
        LiveTranslateServerEvent::Error { .. }
            | LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { .. }
            | LiveTranslateServerEvent::SubtitleFinalPair { .. }
    )));
    client.disconnect().await;
}

#[tokio::test]
async fn full_mock_lifecycle_frames_audio_and_emits_subtitles() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let start = binary(socket.next().await.unwrap().unwrap());
            assert_eq!(find_varint(&start, 2), Some(EVENT_START_SESSION as u64));
            socket
                .send(Message::Binary(
                    server_event(EVENT_SESSION_STARTED as u64, None).into(),
                ))
                .await
                .unwrap();

            let audio = binary(socket.next().await.unwrap().unwrap());
            assert_eq!(find_varint(&audio, 2), Some(EVENT_TASK_REQUEST as u64));
            let source_audio = find_bytes(&audio, 4).unwrap();
            let pcm = find_bytes(source_audio, 14).unwrap();
            assert_eq!(pcm.len(), VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT);
            assert_eq!(&pcm[..3], &[1, 2, 3]);
            assert!(pcm[3..].iter().all(|byte| *byte == 0));

            for response in [
                server_event(EVENT_SOURCE_SUBTITLE_START as u64, None),
                server_event(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, Some("Hello")),
            ] {
                socket.send(Message::Binary(response.into())).await.unwrap();
            }

            let finish = binary(socket.next().await.unwrap().unwrap());
            assert_eq!(find_varint(&finish, 2), Some(EVENT_FINISH_SESSION as u64));
            for response in [
                server_event(EVENT_TRANSLATION_SUBTITLE_START as u64, None),
                server_event(
                    EVENT_TRANSLATION_SUBTITLE_RESPONSE as u64,
                    Some("こんにちは"),
                ),
                // Finals can arrive in either order. Sentence timing, not
                // arrival order, is the pairing identity.
                timed_server_event(
                    EVENT_TRANSLATION_SUBTITLE_END as u64,
                    "こんにちは。",
                    1_000,
                    1_800,
                ),
                timed_server_event(EVENT_SOURCE_SUBTITLE_END as u64, "Hello.", 1_000, 1_800),
                server_event(EVENT_SESSION_FINISHED as u64, None),
            ] {
                socket.send(Message::Binary(response.into())).await.unwrap();
            }
        })
    })
    .await;
    client
        .connect_with_timeout(Duration::from_millis(500))
        .await
        .unwrap();
    let mut pcm = vec![0; VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT];
    pcm[..3].copy_from_slice(&[1, 2, 3]);
    client.send_audio(&pcm).await.unwrap();

    let mut received = Vec::new();
    loop {
        let event = tokio::time::timeout(Duration::from_millis(500), events.recv())
            .await
            .unwrap()
            .unwrap();
        let saw_source_draft = matches!(
            &event,
            LiveTranslateServerEvent::UtteranceText { role: UtteranceRole::Source, text, language, is_final: false, .. } | LiveTranslateServerEvent::SourceDraft {text, language, ..}
                if text == "Hello" && language.as_deref() == Some("en")
        );
        received.push(event);
        if saw_source_draft {
            break;
        }
    }

    client.finish(Duration::from_secs(1)).await;
    while let Ok(event) = events.try_recv() {
        received.push(event);
    }
    assert!(received.iter().any(|event| matches!(
        event,
        LiveTranslateServerEvent::UtteranceText { role: UtteranceRole::Source, text, language, is_final: false, .. } | LiveTranslateServerEvent::SourceDraft {text, language, ..}
            if text == "Hello" && language.as_deref() == Some("en")
    )));
    assert!(received.iter().any(|event| matches!(
        event,
        LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { source, language, translation, .. } | LiveTranslateServerEvent::SubtitleFinalPair { source, language, translation, .. }
            if source == "Hello."
                && language.as_deref() == Some("en")
                && translation == "こんにちは。"
    )));
    let pair_index = received
        .iter()
        .position(|event| {
            matches!(
                event,
                LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { .. }
                    | LiveTranslateServerEvent::SubtitleFinalPair { .. }
            )
        })
        .unwrap();
    let finished_index = received
        .iter()
        .position(|event| matches!(event, LiveTranslateServerEvent::SessionFinished))
        .unwrap();
    assert!(pair_index < finished_index);
    assert!(!received.iter().any(|event| matches!(
        event,
        LiveTranslateServerEvent::SourceFinal { .. }
            | LiveTranslateServerEvent::TranslationFinal(_)
    )));
    assert!(received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
    assert!(!received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::Error { .. })));
}

#[tokio::test]
async fn graceful_finish_drops_an_incomplete_confirmed_tail() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket
                .send(Message::Binary(
                    server_event(EVENT_SESSION_STARTED as u64, None).into(),
                ))
                .await
                .unwrap();
            let finish = binary(socket.next().await.unwrap().unwrap());
            assert_eq!(find_varint(&finish, 2), Some(EVENT_FINISH_SESSION as u64));
            socket
                .send(Message::Binary(
                    timed_server_event(
                        EVENT_SOURCE_SUBTITLE_END as u64,
                        "unmatched final source",
                        2_000,
                        2_800,
                    )
                    .into(),
                ))
                .await
                .unwrap();
            socket
                .send(Message::Binary(
                    server_event(EVENT_SESSION_FINISHED as u64, None).into(),
                ))
                .await
                .unwrap();
        })
    })
    .await;
    client
        .connect_with_timeout(Duration::from_millis(500))
        .await
        .unwrap();
    client.finish(Duration::from_secs(1)).await;

    let mut received = Vec::new();
    while let Ok(event) = events.try_recv() {
        received.push(event);
    }
    assert!(received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
    assert!(!received.iter().any(|event| matches!(
        event,
        LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { .. }
            | LiveTranslateServerEvent::SubtitleFinalPair { .. }
            | LiveTranslateServerEvent::SourceFinal { .. }
            | LiveTranslateServerEvent::TranslationFinal(_)
    )));
}

#[tokio::test]
async fn unsolicited_session_finished_is_a_safe_local_error() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket
                .send(Message::Binary(
                    server_event(EVENT_SESSION_STARTED as u64, None).into(),
                ))
                .await
                .unwrap();

            let audio = binary(socket.next().await.unwrap().unwrap());
            assert_eq!(find_varint(&audio, 2), Some(EVENT_TASK_REQUEST as u64));
            socket
                .send(Message::Binary(
                    server_event(EVENT_SESSION_FINISHED as u64, None).into(),
                ))
                .await
                .unwrap();
        })
    })
    .await;
    client
        .connect_with_timeout(Duration::from_millis(500))
        .await
        .unwrap();
    client
        .send_audio(&vec![0; VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT])
        .await
        .unwrap();

    let mut received = Vec::new();
    loop {
        let event = tokio::time::timeout(Duration::from_millis(500), events.recv())
            .await
            .unwrap()
            .unwrap();
        let is_error = matches!(
            &event,
            LiveTranslateServerEvent::Error { code, message }
                if code == "volcano_unexpected_session_finished"
                    && message == UNEXPECTED_SESSION_FINISHED_ERROR
        );
        received.push(event);
        if is_error {
            break;
        }
    }
    assert!(!received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
    assert!(!client.inner.ready.load(Ordering::SeqCst));
    client.disconnect().await;
}

#[tokio::test]
async fn oversized_unmatched_final_buffer_emits_safe_error_and_recovers() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket
                .send(Message::Binary(
                    server_event(EVENT_SESSION_STARTED as u64, None).into(),
                ))
                .await
                .unwrap();
            let finish = binary(socket.next().await.unwrap().unwrap());
            assert_eq!(find_varint(&finish, 2), Some(EVENT_FINISH_SESSION as u64));

            let oversized_half = MAXIMUM_PENDING_FINAL_TEXT_BYTES / 2 + 1;
            for response in [
                timed_server_event(
                    EVENT_SOURCE_SUBTITLE_END as u64,
                    &"private source".repeat(oversized_half / "private source".len() + 1),
                    0,
                    100,
                ),
                timed_server_event(
                    EVENT_TRANSLATION_SUBTITLE_END as u64,
                    &"private translation".repeat(oversized_half / "private translation".len() + 1),
                    100,
                    200,
                ),
                timed_server_event(EVENT_SOURCE_SUBTITLE_END as u64, "next sentence", 200, 300),
                timed_server_event(EVENT_TRANSLATION_SUBTITLE_END as u64, "次の文", 200, 300),
                server_event(EVENT_SESSION_FINISHED as u64, None),
            ] {
                socket.send(Message::Binary(response.into())).await.unwrap();
            }
        })
    })
    .await;
    client
        .connect_with_timeout(Duration::from_millis(500))
        .await
        .unwrap();
    client.finish(Duration::from_secs(1)).await;

    let mut received = Vec::new();
    while let Ok(event) = events.try_recv() {
        received.push(event);
    }
    assert!(received.iter().any(|event| matches!(
        event,
        LiveTranslateServerEvent::Error { code, message }
            if code == "volcano_transcript_safety_limit"
                && message == TRANSCRIPT_SAFETY_LIMIT_ERROR
    )));
    assert!(received.iter().any(|event| matches!(
        event,
        LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { source, language, translation, .. } | LiveTranslateServerEvent::SubtitleFinalPair { source, language, translation, .. }
            if source == "next sentence"
                && language.as_deref() == Some("en")
                && translation == "次の文"
    )));
    assert!(!received.iter().any(|event| matches!(
        event,
        LiveTranslateServerEvent::SourceFinal { text, .. }
            if text.contains("private")
    ) || matches!(
        event,
        LiveTranslateServerEvent::TranslationFinal(translation)
            if translation.contains("private")
    )));
}

#[tokio::test]
async fn provider_setup_failure_is_content_free() {
    let private_detail = "private transcript and volcano-secret-key";
    let (client, _events) = test_client(move |mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket
                .send(Message::Binary(
                    failed_server_event(45_000_001, private_detail).into(),
                ))
                .await
                .unwrap();
        })
    })
    .await;
    let error = client
        .connect_with_timeout(Duration::from_millis(500))
        .await
        .unwrap_err();
    assert_eq!(error, VolcanoEngineClientError::SessionSetupRejected);
    assert!(!format!("{error:?}").contains(private_detail));
}

#[tokio::test]
async fn setup_timeout_is_finite() {
    let (client, _events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            tokio::time::sleep(Duration::from_secs(2)).await;
        })
    })
    .await;
    assert_eq!(
        client
            .connect_with_timeout(Duration::from_millis(50))
            .await
            .unwrap_err(),
        VolcanoEngineClientError::SessionSetupTimedOut
    );
}

#[tokio::test]
async fn an_immediate_pong_cannot_be_lost() {
    let (client, _events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket
                .send(Message::Binary(
                    server_event(EVENT_SESSION_STARTED as u64, None).into(),
                ))
                .await
                .unwrap();
            while let Some(Ok(message)) = socket.next().await {
                if let Message::Ping(payload) = message {
                    socket.send(Message::Pong(payload)).await.unwrap();
                    break;
                }
            }
        })
    })
    .await;
    client
        .connect_with_timeout(Duration::from_millis(500))
        .await
        .unwrap();
    client.ping(Duration::from_millis(200)).await.unwrap();
    client.disconnect().await;
}

#[test]
fn audio_buffer_keeps_only_a_partial_tail() {
    let frame_size = VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT;
    let mut pending = vec![0x22; frame_size * 2 + 17];
    let messages = take_complete_audio_messages("session-buffer", &mut pending).unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(pending, vec![0x22; 17]);
    for message in messages {
        assert_eq!(find_varint(&message, 2), Some(EVENT_TASK_REQUEST as u64));
        assert_eq!(
            find_bytes(find_bytes(&message, 4).unwrap(), 14)
                .unwrap()
                .len(),
            frame_size
        );
    }
}

#[test]
fn failed_audio_encoding_still_consumes_the_complete_batch() {
    let frame_size = VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT;
    let mut pending = vec![0x22; frame_size * 2 + 17];

    assert_eq!(
        take_complete_audio_messages("", &mut pending),
        Err(VolcanoEngineClientError::TransportFailure)
    );
    assert_eq!(pending, vec![0x22; 17]);
}

#[test]
fn final_partial_audio_frame_is_zero_padded() {
    let mut pending = vec![1, 2, 3];
    let message = take_padded_audio_message("session-tail", &mut pending)
        .unwrap()
        .unwrap();
    let pcm = find_bytes(find_bytes(&message, 4).unwrap(), 14).unwrap();
    assert_eq!(pcm.len(), VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT);
    assert_eq!(&pcm[..3], &[1, 2, 3]);
    assert!(pcm[3..].iter().all(|byte| *byte == 0));
    assert!(pending.is_empty());
}

#[tokio::test]
async fn malformed_response_after_setup_is_a_content_free_protocol_error() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket
                .send(Message::Binary(
                    server_event(EVENT_SESSION_STARTED as u64, None).into(),
                ))
                .await
                .unwrap();
            socket
                .send(Message::Binary(vec![0x10, 0x80].into()))
                .await
                .unwrap();
        })
    })
    .await;
    client
        .connect_with_timeout(Duration::from_millis(500))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    let mut received = Vec::new();
    while let Ok(event) = events.try_recv() {
        received.push(event);
    }
    assert!(received.iter().any(|event| matches!(
        event,
        LiveTranslateServerEvent::Error { code, message }
            if code == "volcano_protocol_error" && message == GENERIC_PROTOCOL_ERROR
    )));
    client.disconnect().await;
}
