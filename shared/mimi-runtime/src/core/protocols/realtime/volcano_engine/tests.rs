use super::*;

#[test]
fn shared_subtitle_contract_accepts_proto3_empty_text_and_rejects_malformed_fields() {
    let contract: serde_json::Value =
        serde_json::from_str(include_str!("../../../../../../translation-contracts.json")).unwrap();
    for case in contract["volcanoSubtitleEvents"].as_array().unwrap() {
        let hex = case["hex"].as_str().unwrap();
        let bytes = (0..hex.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).unwrap())
            .collect::<Vec<_>>();
        let decoded = VolcanoEngineServerEvent::decode(&bytes);
        let expected = &case["expected"];
        if expected.is_null() {
            assert!(decoded.is_err(), "{}", case["id"]);
            continue;
        }
        let (kind, text) = match decoded.unwrap() {
            VolcanoEngineServerEvent::SourceSubtitleDraft(text) => ("sourceDraft", text),
            VolcanoEngineServerEvent::SourceSubtitleFinal { text, .. } => ("sourceFinal", text),
            VolcanoEngineServerEvent::TranslationSubtitleDraft(text) => ("translationDraft", text),
            VolcanoEngineServerEvent::TranslationSubtitleFinal { text, .. } => {
                ("translationFinal", text)
            }
            other => panic!("unexpected event {other:?}"),
        };
        assert_eq!(kind, expected["type"].as_str().unwrap(), "{}", case["id"]);
        assert_eq!(text, expected["text"].as_str().unwrap(), "{}", case["id"]);
    }
}

#[test]
fn shared_timing_contract_applies_proto3_defaults_without_inferring_response_identity() {
    let contract: serde_json::Value =
        serde_json::from_str(include_str!("../../../../../../translation-contracts.json")).unwrap();
    for case in contract["volcanoSubtitleTimingEvents"].as_array().unwrap() {
        let hex = case["hex"].as_str().unwrap();
        let bytes = (0..hex.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).unwrap())
            .collect::<Vec<_>>();
        let decoded = VolcanoEngineServerEvent::decode(&bytes);
        if case["expected"].is_null() {
            assert!(decoded.is_err(), "{}", case["id"]);
            continue;
        }
        use serde_json::json;
        let value = match decoded.unwrap() {
            VolcanoEngineServerEvent::SourceSubtitleStarted { start_time_ms } => {
                json!({"type":"sourceStart","startTimeMs":start_time_ms})
            }
            VolcanoEngineServerEvent::TranslationSubtitleStarted { start_time_ms } => {
                json!({"type":"translationStart","startTimeMs":start_time_ms})
            }
            VolcanoEngineServerEvent::SourceSubtitleFinal {
                text,
                start_time_ms,
                end_time_ms,
            } => {
                json!({"type":"sourceFinal","text":text,"startTimeMs":start_time_ms,"endTimeMs":end_time_ms})
            }
            VolcanoEngineServerEvent::TranslationSubtitleFinal {
                text,
                start_time_ms,
                end_time_ms,
            } => {
                json!({"type":"translationFinal","text":text,"startTimeMs":start_time_ms,"endTimeMs":end_time_ms})
            }
            VolcanoEngineServerEvent::SourceSubtitleDraft(text) => {
                json!({"type":"sourceDraft","text":text})
            }
            VolcanoEngineServerEvent::TranslationSubtitleDraft(text) => {
                json!({"type":"translationDraft","text":text})
            }
            other => panic!("unexpected event {other:?}"),
        };
        assert_eq!(value, case["expected"], "{}", case["id"]);
    }
}

#[test]
fn shared_s2t_catalog_validates_all_directions_and_serializes_every_legal_setup() {
    let contract: serde_json::Value =
        serde_json::from_str(include_str!("../../../../../../translation-contracts.json")).unwrap();
    let catalog = &contract["providerLanguageCatalogs"]["volcanoEngine"];
    let pairs = catalog["targetsBySource"].as_object().unwrap();
    assert_eq!(
        serde_json::to_value(SOURCE_LANGUAGE_CODES).unwrap(),
        catalog["sourceLanguages"]
    );
    assert_eq!(
        serde_json::to_value(TARGET_LANGUAGE_CODES).unwrap(),
        catalog["targetLanguages"]
    );
    for source in SourceLanguage::ALL {
        for target in TargetLanguage::ALL {
            let allowed = pairs
                .get(source.raw_value())
                .and_then(|targets| targets.as_array())
                .is_some_and(|targets| {
                    targets
                        .iter()
                        .any(|code| code.as_str() == Some(target.raw_value()))
                });
            assert_eq!(
                VolcanoEngineRequestEncoder::validate_languages(source, target).is_ok(),
                allowed,
                "{} -> {}",
                source.raw_value(),
                target.raw_value()
            );
        }
    }
    for case in catalog["setups"].as_array().unwrap() {
        let source = serde_json::from_value(case["source"].clone()).unwrap();
        let target = serde_json::from_value(case["target"].clone()).unwrap();
        let bytes =
            VolcanoEngineRequestEncoder::start_session("catalog-session", source, target).unwrap();
        let hex = bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(hex, case["expectedHex"].as_str().unwrap());
    }
}

fn field_bytes(message: &[u8], wanted_field: u32) -> Option<&[u8]> {
    let mut reader = ProtoReader::new(message);
    while !reader.is_empty() {
        let (field, wire_type) = reader.read_key().ok()?;
        if field == wanted_field && wire_type == WIRE_LENGTH_DELIMITED {
            return reader.read_length_delimited().ok();
        }
        reader.skip_value(wire_type).ok()?;
    }
    None
}

fn field_varint(message: &[u8], wanted_field: u32) -> Option<u64> {
    let mut reader = ProtoReader::new(message);
    while !reader.is_empty() {
        let (field, wire_type) = reader.read_key().ok()?;
        if field == wanted_field && wire_type == WIRE_VARINT {
            return reader.read_varint().ok();
        }
        reader.skip_value(wire_type).ok()?;
    }
    None
}

fn response(event: u64, text: Option<&str>) -> Vec<u8> {
    let mut message = Vec::new();
    write_varint_field(&mut message, 2, event);
    if let Some(text) = text {
        write_string_field(&mut message, 4, text);
    }
    message
}

fn timed_response(event: u64, text: &str, start_time_ms: u64, end_time_ms: u64) -> Vec<u8> {
    let mut message = response(event, Some(text));
    write_varint_field(&mut message, 5, start_time_ms);
    write_varint_field(&mut message, 6, end_time_ms);
    message
}

#[test]
fn endpoint_and_audio_contract_match_the_official_ast_v2_document() {
    assert_eq!(
        VolcanoEngineEndpoint::url().unwrap().as_str(),
        "wss://openspeech.bytedance.com/api/v4/ast/v2/translate"
    );
    assert_eq!(
        VolcanoEngineEndpoint::RESOURCE_ID,
        "volc.service_type.10053"
    );
    assert_eq!(VolcanoEngineEndpoint::SAMPLE_RATE_HZ, 16_000);
    assert_eq!(VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT, 2_560);
}

#[test]
fn start_session_encodes_the_official_translate_request_fields() {
    let encoded = VolcanoEngineRequestEncoder::start_session(
        "session-123",
        SourceLanguage::Japanese,
        TargetLanguage::SimplifiedChinese,
    )
    .unwrap();

    assert_eq!(field_varint(&encoded, 2), Some(EVENT_START_SESSION as u64));
    let meta = field_bytes(&encoded, 1).unwrap();
    assert_eq!(field_bytes(meta, 6), Some("session-123".as_bytes()));

    let source_audio = field_bytes(&encoded, 4).unwrap();
    assert_eq!(field_bytes(source_audio, 4), Some("wav".as_bytes()));
    assert_eq!(field_bytes(source_audio, 5), Some("raw".as_bytes()));
    assert_eq!(field_varint(source_audio, 7), Some(16_000));
    assert_eq!(field_varint(source_audio, 8), Some(16));
    assert_eq!(field_varint(source_audio, 9), Some(1));

    let request = field_bytes(&encoded, 6).unwrap();
    assert_eq!(field_bytes(request, 1), Some("s2t".as_bytes()));
    assert_eq!(field_bytes(request, 2), Some("ja".as_bytes()));
    assert_eq!(field_bytes(request, 3), Some("zh".as_bytes()));
}

#[test]
fn task_request_carries_exactly_one_official_80_ms_audio_frame() {
    let contract: serde_json::Value =
        serde_json::from_str(include_str!("../../../../../../translation-contracts.json")).unwrap();
    let wire = &contract["volcanoAudioRequests"];
    let hex_bytes = |hex: &str| {
        (0..hex.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).unwrap())
            .collect::<Vec<_>>()
    };
    let frame_bytes = VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT;
    assert_eq!(wire["frameBytes"].as_u64().unwrap() as usize, frame_bytes);
    let session_id = wire["sessionId"].as_str().unwrap();
    let mut metadata = hex_bytes(wire["metadataPrefixHex"].as_str().unwrap());
    metadata.extend_from_slice(session_id.as_bytes());
    let mut audio_prefix = metadata.clone();
    audio_prefix.extend_from_slice(&hex_bytes(wire["audioSuffixHex"].as_str().unwrap()));
    let mut seed = 0x6d2b_79f5_u32;
    let noise = (0..frame_bytes)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as u8
        })
        .collect::<Vec<_>>();
    let patterns = [
        vec![0; frame_bytes],
        (0..frame_bytes)
            .map(|index| [0x00, 0x80, 0xff, 0x7f][index % 4])
            .collect(),
        (0..frame_bytes).map(|index| index as u8).collect(),
        noise,
    ];
    for pcm in &patterns {
        let mut expected = audio_prefix.clone();
        expected.extend_from_slice(pcm);
        assert_eq!(
            VolcanoEngineRequestEncoder::audio(session_id, pcm).unwrap(),
            expected
        );
    }
    let mut start = metadata.clone();
    start.extend_from_slice(&hex_bytes(wire["startSuffixHex"].as_str().unwrap()));
    assert_eq!(
        VolcanoEngineRequestEncoder::start_session(
            session_id,
            SourceLanguage::English,
            TargetLanguage::SimplifiedChinese,
        )
        .unwrap(),
        start
    );
    let mut finish = metadata;
    finish.extend_from_slice(&hex_bytes(wire["finishSuffixHex"].as_str().unwrap()));
    assert_eq!(
        VolcanoEngineRequestEncoder::finish_session(session_id).unwrap(),
        finish
    );

    assert_eq!(
        VolcanoEngineRequestEncoder::audio("session-audio", &[0; 10]).unwrap_err(),
        VolcanoEngineProtocolError::InvalidAudioFrame {
            expected_bytes: 2_560,
            actual_bytes: 10,
        }
    );
}

#[test]
fn finish_session_uses_the_official_event_and_session_id() {
    let encoded = VolcanoEngineRequestEncoder::finish_session("session-finish").unwrap();
    assert_eq!(field_varint(&encoded, 2), Some(EVENT_FINISH_SESSION as u64));
    assert_eq!(
        field_bytes(field_bytes(&encoded, 1).unwrap(), 6),
        Some("session-finish".as_bytes())
    );
}

#[test]
fn automatic_source_and_original_output_remain_unsupported() {
    for source in [
        SourceLanguage::Chinese,
        SourceLanguage::English,
        SourceLanguage::Japanese,
    ] {
        VolcanoEngineRequestEncoder::validate_languages(source, TargetLanguage::English).unwrap();
    }
    assert_eq!(
        VolcanoEngineRequestEncoder::validate_languages(
            SourceLanguage::Automatic,
            TargetLanguage::English,
        )
        .unwrap_err(),
        VolcanoEngineProtocolError::UnsupportedSourceLanguage
    );
    assert_eq!(
        VolcanoEngineRequestEncoder::validate_languages(
            SourceLanguage::Chinese,
            TargetLanguage::Original,
        )
        .unwrap_err(),
        VolcanoEngineProtocolError::UnsupportedTargetLanguage
    );
}

#[test]
fn subtitle_and_lifecycle_events_decode_from_official_field_numbers() {
    let cases = [
        (
            response(EVENT_SESSION_STARTED as u64, None),
            VolcanoEngineServerEvent::SessionStarted,
        ),
        (
            response(EVENT_SOURCE_SUBTITLE_START as u64, None),
            VolcanoEngineServerEvent::SourceSubtitleStarted {
                start_time_ms: Some(0),
            },
        ),
        (
            response(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, Some("hello")),
            VolcanoEngineServerEvent::SourceSubtitleDraft("hello".into()),
        ),
        (
            response(EVENT_SOURCE_SUBTITLE_END as u64, Some("hello.")),
            VolcanoEngineServerEvent::SourceSubtitleFinal {
                text: "hello.".into(),
                start_time_ms: Some(0),
                end_time_ms: Some(0),
            },
        ),
        (
            response(EVENT_TRANSLATION_SUBTITLE_START as u64, None),
            VolcanoEngineServerEvent::TranslationSubtitleStarted {
                start_time_ms: Some(0),
            },
        ),
        (
            response(EVENT_TRANSLATION_SUBTITLE_RESPONSE as u64, Some("你好")),
            VolcanoEngineServerEvent::TranslationSubtitleDraft("你好".into()),
        ),
        (
            response(EVENT_TRANSLATION_SUBTITLE_END as u64, Some("你好。")),
            VolcanoEngineServerEvent::TranslationSubtitleFinal {
                text: "你好。".into(),
                start_time_ms: Some(0),
                end_time_ms: Some(0),
            },
        ),
        (
            response(EVENT_SESSION_FINISHED as u64, None),
            VolcanoEngineServerEvent::SessionFinished,
        ),
    ];

    for (wire, expected) in cases {
        assert_eq!(VolcanoEngineServerEvent::decode(&wire).unwrap(), expected);
    }
}

#[test]
fn final_subtitle_events_preserve_official_sentence_timing_fields() {
    assert_eq!(
        VolcanoEngineServerEvent::decode(&timed_response(
            EVENT_SOURCE_SUBTITLE_END as u64,
            "hello.",
            1_200,
            2_340,
        ))
        .unwrap(),
        VolcanoEngineServerEvent::SourceSubtitleFinal {
            text: "hello.".into(),
            start_time_ms: Some(1_200),
            end_time_ms: Some(2_340),
        }
    );
    assert_eq!(
        VolcanoEngineServerEvent::decode(&timed_response(
            EVENT_TRANSLATION_SUBTITLE_END as u64,
            "你好。",
            1_200,
            2_340,
        ))
        .unwrap(),
        VolcanoEngineServerEvent::TranslationSubtitleFinal {
            text: "你好。".into(),
            start_time_ms: Some(1_200),
            end_time_ms: Some(2_340),
        }
    );
}

#[test]
fn starts_preserve_explicit_values_and_apply_proto3_omitted_zero() {
    for (event, source) in [
        (EVENT_SOURCE_SUBTITLE_START, true),
        (EVENT_TRANSLATION_SUBTITLE_START, false),
    ] {
        for start in [Some(0), Some(1_200), None] {
            let mut wire = response(event as u64, None);
            if let Some(start) = start {
                write_varint_field(&mut wire, 5, start as u64);
            }
            let expected = if source {
                VolcanoEngineServerEvent::SourceSubtitleStarted {
                    start_time_ms: Some(start.unwrap_or_default()),
                }
            } else {
                VolcanoEngineServerEvent::TranslationSubtitleStarted {
                    start_time_ms: Some(start.unwrap_or_default()),
                }
            };
            assert_eq!(VolcanoEngineServerEvent::decode(&wire).unwrap(), expected);
        }
    }
}

#[test]
fn session_failure_keeps_only_the_numeric_status_code() {
    let mut response_meta = Vec::new();
    write_varint_field(&mut response_meta, 3, 45_000_001);
    write_string_field(
        &mut response_meta,
        4,
        "private provider detail that must never reach diagnostics",
    );
    let mut message = Vec::new();
    write_bytes_field(&mut message, 1, &response_meta);
    write_varint_field(&mut message, 2, EVENT_SESSION_FAILED as u64);

    assert_eq!(
        VolcanoEngineServerEvent::decode(&message).unwrap(),
        VolcanoEngineServerEvent::SessionFailed {
            status_code: Some(45_000_001),
        }
    );
}

#[test]
fn unknown_forward_compatible_fields_are_skipped_by_wire_type() {
    let mut message = response(EVENT_SESSION_STARTED as u64, None);
    write_key(&mut message, 20, WIRE_VARINT);
    write_varint(&mut message, 42);
    write_key(&mut message, 21, WIRE_FIXED_64);
    message.extend_from_slice(&[0; 8]);
    write_key(&mut message, 22, WIRE_LENGTH_DELIMITED);
    write_varint(&mut message, 3);
    message.extend_from_slice(&[1, 2, 3]);
    write_key(&mut message, 23, WIRE_FIXED_32);
    message.extend_from_slice(&[0; 4]);
    assert_eq!(
        VolcanoEngineServerEvent::decode(&message).unwrap(),
        VolcanoEngineServerEvent::SessionStarted
    );
}

#[test]
fn malformed_or_ambiguous_protobuf_is_rejected() {
    let cases = [
        vec![],
        vec![0],
        vec![0x10, 0x80],
        vec![0x12, 0x01, 0x96],
        vec![0x10, 0x96, 0x01, 0x10, 0x96, 0x01],
        vec![0x10, 0x8b, 0x05, 0x20, 0x00],
        vec![0x10, 0x8b, 0x05, 0x22, 0x01, 0xff],
        vec![0x10, 0x96, 0x01, 0x1b],
    ];
    for wire in cases {
        assert!(VolcanoEngineServerEvent::decode(&wire).is_err(), "{wire:?}");
    }
}

#[test]
fn server_frame_and_transcript_safety_limits_are_enforced() {
    let oversized = vec![0; VolcanoEngineEndpoint::MAXIMUM_SERVER_FRAME_BYTES + 1];
    assert_eq!(
        VolcanoEngineServerEvent::decode(&oversized).unwrap_err(),
        VolcanoEngineProtocolError::MessageTooLarge
    );

    let mut message = response(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, None);
    write_key(&mut message, 4, WIRE_LENGTH_DELIMITED);
    write_varint(&mut message, (MAXIMUM_TRANSCRIPT_BYTES + 1) as u64);
    message.resize(message.len() + MAXIMUM_TRANSCRIPT_BYTES + 1, b'x');
    assert_eq!(
        VolcanoEngineServerEvent::decode(&message).unwrap_err(),
        VolcanoEngineProtocolError::MessageTooLarge
    );
}
