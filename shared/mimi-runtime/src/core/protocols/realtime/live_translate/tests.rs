use super::*;

#[test]
fn optional_models_require_a_valid_beijing_workspace_host() {
    let mode = AlibabaRealtimeMode::LiveTranslate;
    assert_eq!(
        LiveTranslateEndpoint::for_mode(mode, "").err(),
        Some(LiveTranslateProtocolError::MissingWorkspace)
    );
    for invalid in [
        "https://example.com",
        "other.workspace",
        "bad_id",
        "-workspace",
        "workspace-",
        "test@host",
    ] {
        assert!(LiveTranslateEndpoint::for_mode(mode, invalid).is_err());
    }
    let endpoint = LiveTranslateEndpoint::for_mode(mode, "synthetic-workspace").unwrap();
    assert_eq!(
        endpoint.url.host_str(),
        Some("synthetic-workspace.cn-beijing.maas.aliyuncs.com")
    );
    assert_eq!(
        endpoint
            .url
            .query_pairs()
            .find(|(key, _)| key == "model")
            .unwrap()
            .1,
        mode.model_name().unwrap()
    );
}

#[test]
fn livetranslate_requests_only_text_with_automatic_source_recognition() {
    let update = LiveTranslateRequestEncoder::session_update_for_mode(
        AlibabaRealtimeMode::LiveTranslate,
        SourceLanguage::Automatic,
        TargetLanguage::SimplifiedChinese,
        &BTreeMap::new(),
        None,
    )
    .unwrap();
    let session = &update["session"];
    assert_eq!(session["output_modalities"], json!(["text"]));
    assert!(session.get("modalities").is_none());
    assert!(session.get("input_audio_transcription").is_none());
    assert!(session.get("audio").is_none());
    assert_eq!(session["translation"]["language"], "zh");
}

#[test]
fn livetranslate_source_deltas_append() {
    let mut live = AlibabaRealtimeEventDecoder::new(AlibabaRealtimeMode::LiveTranslate);
    live.decode(r#"{"type":"conversation.item.input_audio_transcription.delta","item_id":"source","delta":"Hello"}"#).unwrap();
    let events = live.decode(r#"{"type":"conversation.item.input_audio_transcription.delta","item_id":"source","delta":" world"}"#).unwrap();
    assert!(
        matches!(&events[0].0, LiveTranslateServerEvent::SourceDraft {text,..} if text == "Hello world")
    );
}

#[test]
fn text_done_waits_for_successful_response_done_and_deduplicates() {
    let mut decoder = AlibabaRealtimeEventDecoder::new(AlibabaRealtimeMode::LiveTranslate);
    decoder.decode(r#"{"type":"response.text.delta","item_id":"out","response_id":"response","delta":"你"}"#).unwrap();
    let delta = decoder.decode(r#"{"type":"response.text.delta","item_id":"out","response_id":"response","delta":"好"}"#).unwrap();
    assert_eq!(
        delta[0].0,
        LiveTranslateServerEvent::TranslationDraft("你好".into())
    );
    let text_done = decoder.decode(r#"{"type":"response.text.done","item_id":"out","response_id":"response","text":"你好。"}"#).unwrap();
    assert_eq!(
        text_done[0].0,
        LiveTranslateServerEvent::TranslationDraft("你好。".into())
    );
    let wire =
        r#"{"type":"response.done","response":{"id":"response","status":"completed","output":[]}}"#;
    let finals = decoder.decode(wire).unwrap();
    assert_eq!(
        finals[0].0,
        LiveTranslateServerEvent::TranslationFinal("你好。".into())
    );
    assert!(decoder.decode(wire).unwrap().is_empty());
}

#[test]
fn cancelled_and_failed_responses_never_confirm_translation() {
    for status in ["cancelled", "incomplete", "failed"] {
        let mut decoder = AlibabaRealtimeEventDecoder::new(AlibabaRealtimeMode::LiveTranslate);
        decoder.decode(r#"{"type":"response.text.done","item_id":"out","response_id":"response","text":"Incomplete preview"}"#).unwrap();
        let wire = json!({"type":"response.done","response":{"id":"response","status":status,"output":[]}}).to_string();
        assert!(!decoder
            .decode(&wire)
            .unwrap()
            .iter()
            .any(|(event, _)| matches!(event, LiveTranslateServerEvent::TranslationFinal(_))));
    }
}

#[test]
fn common_response_timeout_requires_the_complete_observed_service_reason() {
    use crate::core::lifecycle::recovery::{
        normalize_provider_error_code, provider_error_is_retryable,
    };
    let reason = "Response stream timeout: timeout=60 seconds, elapsed=60001 ms";
    let mut cases = vec![
        (
            json!({"code":"COMMON_ERROR","message":"response_stream_timeout(timeout_seconds=60, elapsed_ms=101831)"}),
            true,
        ),
        (
            json!({"code":"COMMON_ERROR","message":"response_stream_timeout(timeout_seconds=.60, elapsed_ms=101831)"}),
            false,
        ),
        (
            json!({"code":"COMMON_ERROR","message":"response_stream_timeout(timeout_seconds=60, elapsed_ms=101831) private"}),
            false,
        ),
        (
            json!({"code":"COMMON_ERROR","type":"server_error","message":reason}),
            true,
        ),
        (json!({"code":"COMMON_ERROR","message":reason}), true),
        (
            json!({"code":"COMMON_ERROR","message":format!("{reason}. ")}),
            false,
        ),
        (
            json!({"code":"COMMON_ERROR","message":format!("{reason}.private")}),
            false,
        ),
        (
            json!({"code":"COMMON_ERROR","message":format!("private {reason}")}),
            false,
        ),
        (
            json!({"code":"COMMON_ERROR","message":format!("{reason} private")}),
            false,
        ),
        (
            json!({"code":"COMMON_ERROR","message":"A different timeout occurred."}),
            false,
        ),
        (
            json!({"code":"COMMON_ERROR.private","message":reason}),
            false,
        ),
        (json!({"code":null,"message":reason}), false),
        (json!({"code":50002,"message":reason}), false),
        (json!({"message":reason}), false),
    ];
    for kind in [
        "authentication_error",
        "authorization_error",
        "invalid_request_error",
        "insufficient_quota",
        "rate_limit_error",
        "private-error-type",
    ] {
        cases.push((
            json!({"code":"COMMON_ERROR","type":kind,"message":reason}),
            false,
        ));
    }
    for kind in [Value::Null, json!(401), json!(false), json!([]), json!({})] {
        cases.push((
            json!({"code":"COMMON_ERROR","type":kind,"message":reason}),
            false,
        ));
    }
    for changed in [
        "-60", "+60", ".60", "60.0", "6e1", "60/1", "060", "0", "59", "61", "6000000",
    ] {
        cases.push((json!({"code":"COMMON_ERROR","message":reason.replace("=60 seconds", &format!("={changed} seconds"))}), false));
    }
    for changed in [
        "59999",
        "060001",
        "60001.0",
        ".60001",
        "18446744073709551616",
        "60001 1",
    ] {
        cases.push((json!({"code":"COMMON_ERROR","message":reason.replace("60001 ms", &format!("{changed} ms"))}), false));
    }
    for changed in [
        reason.replace("seconds", "minutes"),
        reason.replace(' ', "\u{a0}"),
        reason.replace("elapsed", "elapsed_private"),
        format!("{reason} https://private.invalid/key"),
    ] {
        cases.push((json!({"code":"COMMON_ERROR","message":changed}), false));
    }
    for (error, expected_retry) in cases {
        for wire in [
            json!({"type":"error","error":error}),
            json!({"type":"conversation.item.input_audio_transcription.failed","error":error}),
            json!({"type":"response.done","response":{"id":"test","status":"failed","status_details":{"error":error},"output":[]}}),
        ] {
            let mut decoder = AlibabaRealtimeEventDecoder::new(AlibabaRealtimeMode::LiveTranslate);
            let events = decoder.decode(&wire.to_string()).unwrap();
            let failure = events
                .iter()
                .find_map(|(event, _)| match event {
                    LiveTranslateServerEvent::Error { code, message } => Some((code, message)),
                    _ => None,
                })
                .unwrap();
            let code = normalize_provider_error_code(failure.0);
            assert_eq!(provider_error_is_retryable(code), expected_retry, "{wire}");
            if expected_retry {
                assert_eq!(code, "alibaba_realtime_idle_timeout");
            }
            assert!(!failure.1.contains("private"));
            assert!(!failure.1.contains("60001"));
        }
    }
    let authentication = safe_realtime_error(
        Some(&json!({"code":"InvalidApiKey","message":reason})),
        false,
    );
    assert!(
        matches!(authentication, LiveTranslateServerEvent::Error {code,..} if code == "credential_authentication_failed")
    );
}

#[test]
fn realtime_errors_retry_only_exact_known_transient_codes_and_never_keep_raw_details() {
    use crate::core::lifecycle::recovery::{
        normalize_provider_error_code, provider_error_is_retryable,
    };
    for (code, expected, retryable) in [
        (
            "response_idle_timeout",
            "alibaba_realtime_idle_timeout",
            true,
        ),
        (
            "ServiceUnavailable",
            "alibaba_realtime_service_unavailable",
            true,
        ),
        (
            "InternalError",
            "alibaba_realtime_service_unavailable",
            true,
        ),
        (
            "internal_error",
            "alibaba_realtime_service_unavailable",
            true,
        ),
        ("InvalidApiKey", "credential_authentication_failed", false),
        ("invalid_api_key", "credential_authentication_failed", false),
        ("unauthorized", "credential_authentication_failed", false),
        (
            "authentication_error",
            "credential_authentication_failed",
            false,
        ),
        ("ServiceUnavailable.private", "provider_error", false),
        ("response_idle_timeout.private", "provider_error", false),
        ("InvalidParameter", "provider_error", false),
        ("insufficient_quota", "provider_error", false),
        ("private-unknown-code", "provider_error", false),
    ] {
        let error =
            json!({"code":code,"message":"private-server-detail","type":"private-error-type"});
        for wire in [
            json!({"type":"error","error":error}),
            json!({"type":"conversation.item.input_audio_transcription.failed","error":error}),
            json!({"type":"response.done","response":{"id":"test-response","status":"failed","status_details":{"error":error},"output":[]}}),
        ] {
            let mut decoder = AlibabaRealtimeEventDecoder::new(AlibabaRealtimeMode::LiveTranslate);
            let decoded = decoder.decode(&wire.to_string()).unwrap();
            let LiveTranslateServerEvent::Error {
                code: safe_code,
                message,
            } = &decoded.last().unwrap().0
            else {
                panic!("expected a classified error");
            };
            assert_eq!(normalize_provider_error_code(safe_code), expected, "{code}");
            assert_eq!(provider_error_is_retryable(safe_code), retryable, "{code}");
            assert!(!safe_code.contains("private"));
            assert!(!message.contains("private"));
        }
    }
}

#[test]
fn clear_and_safety_limits_cannot_restore_old_text_or_error_bodies() {
    let mut decoder = AlibabaRealtimeEventDecoder::new(AlibabaRealtimeMode::LiveTranslate);
    decoder.decode(r#"{"type":"conversation.item.input_audio_transcription.delta","item_id":"old","delta":"Old source"}"#).unwrap();
    decoder.clear_content();
    assert!(decoder.decode(r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"old","transcript":"Old source."}"#).unwrap().is_empty());
    assert_eq!(
        decoder
            .decode(r#"{"type":"session.finished","item_id":"old"}"#)
            .unwrap()[0]
            .0,
        LiveTranslateServerEvent::SessionFinished
    );
    let huge = json!({"type":"response.text.delta","item_id":"out","delta":"x".repeat(crate::core::models::MAX_SUBTITLE_TEXT_BYTES + 1)}).to_string();
    assert!(decoder.decode(&huge).is_err());
    assert!(!decoder.translation.contains_key("out"));
    let error = decoder.decode(r#"{"type":"error","error":{"code":"private value","message":"private source or credential"}}"#).unwrap();
    assert!(
        matches!(&error[0].0, LiveTranslateServerEvent::Error {code,message} if code == "alibaba_realtime_provider_error" && !message.contains("private"))
    );
}

#[test]
fn clear_rejects_late_items_from_a_previously_created_response() {
    let mut decoder = AlibabaRealtimeEventDecoder::new(AlibabaRealtimeMode::LiveTranslate);
    decoder
        .decode(r#"{"type":"response.created","response":{"id":"old-response"}}"#)
        .unwrap();
    decoder.clear_content();
    let late = decoder.decode(r#"{"type":"response.text.delta","item_id":"late-item","response_id":"old-response","delta":"Obsolete translation"}"#).unwrap();
    assert!(late.is_empty());
}

#[test]
fn shared_pairing_identity_and_language_are_bounded_before_retention() {
    let oversized = "x".repeat(crate::core::models::MAX_SUBTITLE_TEXT_BYTES + 1);
    for json in [
        json!({"type":"conversation.item.created","item_id":oversized,"previous_item_id":"valid"}),
        json!({"type":"conversation.item.created","item_id":"valid","previous_item_id":oversized}),
        json!({"type":"conversation.item.input_audio_transcription.completed","item_id":"valid","transcript":"Valid source","language":oversized}),
    ] {
        assert_eq!(
            LiveTranslateServerEvent::decode_value_with_identity(&json),
            Err(LiveTranslateProtocolError::InvalidJSON)
        );
    }
}

#[test]
fn expanded_app_languages_do_not_expand_the_legacy_realtime_wire_contract() {
    for source in SourceLanguage::ALL.into_iter().filter(|source| {
        !matches!(
            source,
            SourceLanguage::Automatic
                | SourceLanguage::Chinese
                | SourceLanguage::English
                | SourceLanguage::Japanese
                | SourceLanguage::Korean
        )
    }) {
        assert_eq!(
            LiveTranslateRequestEncoder::session_update(
                source,
                TargetLanguage::English,
                &BTreeMap::new(),
                None
            )
            .unwrap_err(),
            LiveTranslateProtocolError::UnsupportedLanguage
        );
    }
    for target in TargetLanguage::ALL.into_iter().filter(|target| {
        !matches!(
            target,
            TargetLanguage::Original
                | TargetLanguage::SimplifiedChinese
                | TargetLanguage::English
                | TargetLanguage::Japanese
        )
    }) {
        assert_eq!(
            LiveTranslateRequestEncoder::session_update(
                SourceLanguage::Automatic,
                target,
                &BTreeMap::new(),
                None
            )
            .unwrap_err(),
            LiveTranslateProtocolError::UnsupportedLanguage
        );
    }
}

fn decode(text: &str) -> LiveTranslateServerEvent {
    LiveTranslateServerEvent::decode_with_identity(text)
        .expect("the test frame is a documented event")
        .0
}

#[test]
fn endpoint_builds_the_unified_realtime_url() {
    let endpoint = LiveTranslateEndpoint::new().unwrap();
    assert_eq!(
        endpoint.url.as_str(),
        "wss://dashscope.aliyuncs.com/api-ws/v1/realtime?model=qwen3.5-livetranslate-flash-realtime"
    );
}

#[test]
fn session_update_requests_text_only_chinese_translation_and_source_transcript() {
    let data = LiveTranslateRequestEncoder::session_update(
        SourceLanguage::Japanese,
        TargetLanguage::SimplifiedChinese,
        &BTreeMap::new(),
        Some("event-session"),
    )
    .unwrap();
    let session = &data["session"];
    let transcription = &session["input_audio_transcription"];
    let translation = &session["translation"];

    assert_eq!(data["type"], "session.update");
    assert_eq!(session["modalities"], json!(["text"]));
    assert_eq!(session["sample_rate"], 16_000);
    assert_eq!(session["input_audio_format"], "pcm");
    assert_eq!(transcription["model"], "qwen3-asr-flash-realtime");
    assert_eq!(transcription["language"], "ja");
    assert_eq!(translation["language"], "zh");
}

#[test]
fn session_update_includes_hotword_corpus_when_present() {
    let hotwords = BTreeMap::from([("mimi".to_string(), "耳".to_string())]);
    let data = LiveTranslateRequestEncoder::session_update(
        SourceLanguage::Japanese,
        TargetLanguage::SimplifiedChinese,
        &hotwords,
        Some("event-hotwords"),
    )
    .unwrap();
    assert_eq!(
        data["session"]["translation"]["corpus"]["phrases"]["mimi"],
        "耳"
    );
}

#[test]
fn audio_append_base64_encodes_pcm_bytes() {
    let data = LiveTranslateRequestEncoder::audio_append(&[0x00, 0x7F, 0xFF], Some("event-audio"))
        .unwrap();
    assert_eq!(
        data,
        json!({
            "event_id": "event-audio",
            "type": "input_audio_buffer.append",
            "audio": "AH//",
        })
    );
}

#[test]
fn session_update_selects_an_explicit_target_language() {
    let data = LiveTranslateRequestEncoder::session_update(
        SourceLanguage::English,
        TargetLanguage::Japanese,
        &BTreeMap::new(),
        Some("event-target"),
    )
    .unwrap();
    assert_eq!(data["session"]["translation"]["language"], "ja");
}

#[test]
fn finish_event_uses_the_documented_type() {
    let json = LiveTranslateRequestEncoder::finish(Some("event-finish")).unwrap();
    assert_eq!(json["type"], "session.finish");
}

#[test]
fn source_preview_combines_confirmed_and_tentative_text() {
    let event = decode(
        r#"{"type":"conversation.item.input_audio_transcription.text","text":"Hello","stash":" world","language":"en"}"#,
    );
    assert_eq!(
        event,
        LiveTranslateServerEvent::SourceDraft {
            text: "Hello world".into(),
            language: Some("en".into())
        }
    );
}

#[test]
fn asr_preview_combines_confirmed_text_and_stash() {
    let event = decode(
        r#"{"type":"conversation.item.input_audio_transcription.text","text":"今日は","stash":"晴れです","language":"ja"}"#,
    );
    assert_eq!(
        event,
        LiveTranslateServerEvent::SourceDraft {
            text: "今日は晴れです".into(),
            language: Some("ja".into())
        }
    );
}

#[test]
fn source_completion_decodes_the_final_transcript() {
    let event = decode(
        r#"{"type":"conversation.item.input_audio_transcription.completed","transcript":"Hello world.","language":"en"}"#,
    );
    assert_eq!(
        event,
        LiveTranslateServerEvent::SourceFinal {
            text: "Hello world.".into(),
            language: Some("en".into())
        }
    );
}

#[test]
fn identity_keeps_the_recognition_item_of_source_events() {
    let (event, identity) = LiveTranslateServerEvent::decode_with_identity(
        r#"{"type":"conversation.item.input_audio_transcription.completed","transcript":"Hello world.","item_id":"item_source"}"#,
    )
    .unwrap();
    assert_eq!(
        event,
        LiveTranslateServerEvent::SourceFinal {
            text: "Hello world.".into(),
            language: None
        }
    );
    assert_eq!(
        identity,
        LiveTranslateEventIdentity {
            item_id: Some("item_source".into()),
            previous_item_id: None,
        }
    );
}

#[test]
fn identity_keeps_the_response_item_of_translation_events() {
    let (event, identity) = LiveTranslateServerEvent::decode_with_identity(
        r#"{"type":"response.text.done","text":"你好，世界。","item_id":"item_response","response_id":"resp_1"}"#,
    )
    .unwrap();
    assert_eq!(
        event,
        LiveTranslateServerEvent::TranslationFinal("你好，世界。".into())
    );
    assert_eq!(identity.item_id.as_deref(), Some("item_response"));
    assert_eq!(identity.previous_item_id, None);
}

#[test]
fn identity_links_a_created_response_item_to_its_input_item() {
    let (event, identity) = LiveTranslateServerEvent::decode_with_identity(
        r#"{"type":"conversation.item.created","item":{"id":"item_response","role":"assistant"},"previous_item_id":"item_source"}"#,
    )
    .unwrap();
    assert_eq!(
        event,
        LiveTranslateServerEvent::Ignored {
            kind: "conversation.item.created".into()
        }
    );
    assert_eq!(identity.item_id.as_deref(), Some("item_response"));
    assert_eq!(identity.previous_item_id.as_deref(), Some("item_source"));
}

#[test]
fn translation_preview_and_completion_decode() {
    let preview = decode(r#"{"type":"response.text.text","text":"你好","stash":"，世界"}"#);
    let final_event = decode(r#"{"type":"response.text.done","text":"你好，世界。"}"#);

    assert_eq!(
        preview,
        LiveTranslateServerEvent::TranslationDraft("你好，世界".into())
    );
    assert_eq!(
        final_event,
        LiveTranslateServerEvent::TranslationFinal("你好，世界。".into())
    );
}

#[test]
fn session_and_error_events_decode() {
    let updated = decode(r#"{"type":"session.updated"}"#);
    let finished = decode(r#"{"type":"session.finished"}"#);
    let failure = decode(
        r#"{"type":"error","error":{"code":"invalid_value","message":"Bad language secret-key"}}"#,
    );
    let auth_failure = decode(
        r#"{"type":"error","error":{"code":"InvalidApiKey","message":"secret-key-rejected"}}"#,
    );

    assert_eq!(updated, LiveTranslateServerEvent::SessionUpdated);
    assert_eq!(finished, LiveTranslateServerEvent::SessionFinished);
    assert_eq!(
        failure,
        LiveTranslateServerEvent::Error {
            code: "alibaba_realtime_provider_error".into(),
            message: "The Alibaba realtime service rejected the request.".into(),
        }
    );
    assert_eq!(
        auth_failure,
        LiveTranslateServerEvent::Error {
            code: "credential_authentication_failed".into(),
            message: "credential_authentication_failed".into(),
        }
    );
}

#[test]
fn unknown_events_are_ignored_without_failing_the_receive_loop() {
    let event = decode(r#"{"type":"response.created"}"#);
    assert_eq!(
        event,
        LiveTranslateServerEvent::Ignored {
            kind: "response.created".into()
        }
    );
}

#[test]
fn malformed_json_and_missing_type_fail_cleanly() {
    assert!(matches!(
        LiveTranslateServerEvent::decode_with_identity("not json"),
        Err(LiveTranslateProtocolError::InvalidJSON)
    ));
    assert!(matches!(
        LiveTranslateServerEvent::decode_with_identity(r#"{"event_id":"x"}"#),
        Err(LiveTranslateProtocolError::MissingEventType)
    ));
}
