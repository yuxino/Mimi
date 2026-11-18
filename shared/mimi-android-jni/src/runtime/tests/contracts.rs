//! Native contracts regressions.

use super::*;

#[test]
fn qwen_streaming_text_contracts_cross_native_exchange_without_network() {
    let contract: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../translation-contracts.json"
    )))
    .unwrap();
    for case in contract["qwenStreamingText"].as_array().unwrap() {
        let mut request = case.clone();
        request["operation"] = json!("qwen_streaming_text");
        let response: Value =
            serde_json::from_str(&exchange(&request.to_string()).unwrap()).unwrap();
        assert_eq!(response["outputs"], case["expected"], "{}", case["id"]);
    }
}

#[test]
fn volcano_timing_contracts_cross_native_exchange_without_network() {
    let contract: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../translation-contracts.json"
    )))
    .unwrap();
    let cases = contract["volcanoSubtitleTimingEvents"].as_array().unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        let request = json!({"operation":"volcano_decode","hex":case["hex"]});
        let response: Value =
            serde_json::from_str(&exchange(&request.to_string()).unwrap()).unwrap();
        assert_eq!(response["event"], case["expected"], "{}", case["id"]);
    }
}

#[test]
fn preview_content_change_contracts_cross_native_exchange_without_network() {
    let contract: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../translation-contracts.json"
    )))
    .unwrap();
    let cases = contract["previewContentChanges"].as_array().unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        let mut request = case.clone();
        request["operation"] = json!("preview_content_changed");
        let response: Value =
            serde_json::from_str(&exchange(&request.to_string()).unwrap()).unwrap();
        assert!(response.is_boolean());
        assert_eq!(response, case["expected"], "{}", case["id"]);
    }
}

#[test]
fn invalid_preview_content_fields_have_content_free_errors() {
    let request = || {
        json!({"operation":"preview_content_changed", "draft":"synthetic-private-marker",
                "previousDraft":"synthetic-private-marker", "language":"en", "previousLanguage":"en"})
    };
    for field in ["draft", "previousDraft"] {
        for invalid in [
            Value::Null,
            json!(12),
            json!(true),
            json!("x".repeat(mimi_runtime::core::models::MAX_SUBTITLE_TEXT_BYTES + 1)),
            json!("字".repeat(mimi_runtime::core::models::MAX_SUBTITLE_TEXT_BYTES / 3 + 1)),
        ] {
            let mut invalid_request = request();
            invalid_request[field] = invalid;
            assert_eq!(
                exchange(&invalid_request.to_string()).unwrap_err(),
                "native_runtime_invalid_request"
            );
        }
    }
    for field in ["language", "previousLanguage"] {
        for invalid in [
            json!(12),
            json!(true),
            json!(""),
            json!("auto"),
            json!("synthetic-private-marker"),
            json!("x".repeat(65_537)),
        ] {
            let mut invalid_request = request();
            invalid_request[field] = invalid;
            assert_eq!(
                exchange(&invalid_request.to_string()).unwrap_err(),
                "native_runtime_invalid_request"
            );
        }
    }
    for field in ["draft", "previousDraft", "language", "previousLanguage"] {
        let mut invalid_request = request();
        invalid_request.as_object_mut().unwrap().remove(field);
        assert_eq!(
            exchange(&invalid_request.to_string()).unwrap_err(),
            "native_runtime_invalid_request"
        );
    }
    let maximum = "x".repeat(mimi_runtime::core::models::MAX_SUBTITLE_TEXT_BYTES);
    let valid = json!({"operation":"preview_content_changed", "draft":maximum,
            "previousDraft":maximum, "language":null, "previousLanguage":null});
    assert_eq!(exchange(&valid.to_string()).unwrap(), "false");
}

#[test]
fn invalid_volcano_decode_requests_have_content_free_errors() {
    for hex in [
        Value::Null,
        json!(12),
        json!("0"),
        json!("synthetic-private-marker"),
        json!("日"),
        json!("00".repeat(VolcanoEngineEndpoint::MAXIMUM_SERVER_FRAME_BYTES + 1)),
    ] {
        assert_eq!(
            exchange(&json!({"operation":"volcano_decode","hex":hex}).to_string()).unwrap_err(),
            "native_runtime_invalid_request"
        );
    }
}

#[test]
fn malformed_native_requests_and_pcm_have_content_free_errors() {
    assert_eq!(
        exchange("invalid synthetic secret").unwrap_err(),
        "native_runtime_invalid_request"
    );
    assert_eq!(
        exchange(r#"{"operation":"create","configuration":{"secret":"synthetic"}}"#).unwrap_err(),
        "invalid_configuration"
    );
    assert_eq!(
        pcm(u64::MAX, vec![1]).unwrap_err(),
        "native_runtime_invalid_pcm"
    );
    assert_eq!(
        pcm(u64::MAX, vec![1, 2]).unwrap_err(),
        "native_runtime_stale_handle"
    );
}
