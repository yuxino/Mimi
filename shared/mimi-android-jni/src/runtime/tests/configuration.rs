//! Native configuration regressions.

use super::*;

#[test]
fn android_text_endpoint_validation_uses_shared_contracts() {
    let contract: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../translation-contracts.json"
    )))
    .unwrap();
    for case in contract["androidEndpoints"].as_array().unwrap() {
        let result = exchange(
            &json!({
                "operation":"normalize_text_endpoint", "route":case["route"],
                "input":case["input"], "allowLocalHttp":case["allowLocalHttp"]
            })
            .to_string(),
        );
        if case["expected"].is_null() {
            assert_eq!(
                result.unwrap_err(),
                case["error"].as_str().unwrap(),
                "{}",
                case["id"]
            );
        } else {
            let result: Value = serde_json::from_str(&result.unwrap()).unwrap();
            assert_eq!(result["endpoint"], case["expected"], "{}", case["id"]);
        }
    }
    for (key, expected) in [
        ("synthetic:fx", "https://api-free.deepl.com/v2/translate"),
        ("synthetic", "https://api.deepl.com/v2/translate"),
    ] {
        let result: Value = serde_json::from_str(&exchange(&json!({"operation":"normalize_text_endpoint", "route":"deepL", "input":key, "allowLocalHttp":false}).to_string()).unwrap()).unwrap();
        assert_eq!(result["endpoint"], expected);
    }
    for request in [
        json!({"operation":"normalize_text_endpoint", "route":"xAI", "input":"https://example.test", "allowLocalHttp":true}),
        json!({"operation":"normalize_text_endpoint", "route":"openAICompatible", "input":"https://example.test"}),
    ] {
        assert_eq!(
            exchange(&request.to_string()).unwrap_err(),
            "native_runtime_invalid_request"
        );
    }
}

#[test]
fn text_endpoint_identity_uses_shared_protocols_and_rejects_invalid_pairs() {
    for (route, suffix) in [
        ("openAICompatible", "chat/completions"),
        ("chatMock", "chat/completions"),
        ("deepLX", "translate"),
    ] {
        let equivalent = |saved: &str, draft: &str| -> bool {
            let output: Value = serde_json::from_str(
                &exchange(
                    &json!({"operation":"text_endpoints_equivalent", "route":route,
                        "savedEndpoint":saved,"draftEndpoint":draft})
                    .to_string(),
                )
                .unwrap(),
            )
            .unwrap();
            output["equivalent"].as_bool().unwrap()
        };
        let saved = "https://example.invalid/v1";
        for draft in [
            "https://example.invalid/v1/".to_string(),
            "https://EXAMPLE.invalid:443/v1".to_string(),
            format!("https://example.invalid/v1/{suffix}"),
        ] {
            assert!(equivalent(saved, &draft));
        }
        for draft in [
            "",
            "invalid",
            "https://other.invalid/v1",
            "https://example.invalid/v2",
            "https://example.invalid:8443/v1",
            "https://example.invalid/v1?token=synthetic",
            "https://example.invalid/v1#fragment",
            "http://example.invalid/v1",
        ] {
            assert!(!equivalent(saved, draft));
        }
        assert!(!equivalent("", ""));
        assert!(!equivalent("invalid", "invalid"));
    }
    assert!(exchange(r#"{"operation":"text_endpoints_equivalent","route":"xAI","savedEndpoint":"","draftEndpoint":""}"#).is_err());
}

#[test]
fn typed_grok_probe_and_xai_asr_capabilities_follow_the_selected_text_route() {
    let probe = json!({"credentials":{"kind":"xAI","apiKey":"synthetic-text-key","model":"selected-grok-model"},
            "sourceLanguage":"en","targetLanguage":"ja","qwenMtModel":"lite"});
    let response: Value = serde_json::from_str(
        &exchange(&json!({"operation":"text_probe_endpoint","configuration":probe}).to_string())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        response["textEndpoint"],
        mimi_runtime::core::services::provider::XAI_TEXT_ENDPOINT
    );
    let capabilities = |route: &str| -> Value {
        serde_json::from_str(&exchange(&json!({"operation":"capabilities","provider":"xAIRealtime","xaiSpeechMode":"asrMt","route":route,"target":"ja"}).to_string()).unwrap()).unwrap()
    };
    let grok = capabilities("xAI");
    let deepl = capabilities("deepL");
    assert_eq!(grok["sampleRateHz"], 16000);
    let codes = |value: &Value| -> std::collections::BTreeSet<String> {
        value["sourceLanguages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|code| code.as_str().unwrap().to_owned())
            .collect()
    };
    let speech: std::collections::BTreeSet<String> = std::iter::once("auto")
        .chain(
            mimi_runtime::core::protocols::recognition::xai_stt::LANGUAGE_CODES
                .iter()
                .copied(),
        )
        .map(str::to_owned)
        .collect();
    let expected_deepl: std::collections::BTreeSet<String> = speech
        .iter()
        .filter(|code| {
            code.as_str() == "auto"
                || mimi_runtime::core::protocols::translation::deepl_languages::DEEPL_SOURCE_CODES
                    .contains(&code.as_str())
        })
        .cloned()
        .collect();
    assert_eq!(codes(&grok), speech);
    assert_eq!(codes(&deepl), expected_deepl);
    let original: Value = serde_json::from_str(
        &exchange(
            &json!({
                "operation":"capabilities","provider":"xAIRealtime","xaiSpeechMode":"asrMt",
                "route":"deepL","target":"original"
            })
            .to_string(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(codes(&original), speech);
    let probe=text_probe_configuration(&json!({"sessionConfiguration":{
            "provider":"xAIRealtime","xaiSpeechMode":"asrMt","qwenMtModel":"lite","credentials":{"kind":"apiKey","apiKey":""},
            "textCredentials":{"kind":"xAI","apiKey":"independent-key","model":"selected-model"},"sourceLanguage":"en","targetLanguage":"ja","translationMode":"turbo","networkProxy":{"mode":"direct"},"textNetworkProxy":{"mode":"direct"}
        }})).unwrap();
    assert!(
        matches!(probe.credentials,mimi_runtime::core::services::configuration::TextTranslationProbeCredentials::Independent(mimi_runtime::core::services::credentials::TextTranslationCredentials::XAI {api_key,..}) if api_key=="independent-key")
    );
}

#[test]
fn credential_draft_resolution_uses_shared_validation_and_content_free_errors() {
    let request = json!({"operation":"resolve_credentials_draft", "provider":"openAIRealtime",
            "speechTranslationMode":"simultaneous", "draft":{"kind":"apiKey", "apiKey":""},
            "saved":{"kind":"apiKey", "apiKey":"synthetic-saved-key"}});
    let response: Value = serde_json::from_str(&exchange(&request.to_string()).unwrap()).unwrap();
    assert_eq!(response["credentials"]["apiKey"], "synthetic-saved-key");
    let mut missing = request.clone();
    missing["saved"] = Value::Null;
    assert_eq!(
        exchange(&missing.to_string()).unwrap_err(),
        "missing_credentials_draft"
    );
    for field in ["provider", "speechTranslationMode", "draft"] {
        let mut invalid = request.clone();
        invalid[field] = json!("synthetic-private-value");
        assert_eq!(
            exchange(&invalid.to_string()).unwrap_err(),
            "native_runtime_invalid_request"
        );
    }
    let mut malformed_saved = request;
    malformed_saved["saved"] = json!({"synthetic-private-field":"synthetic-private-value"});
    assert_eq!(
        exchange(&malformed_saved.to_string()).unwrap_err(),
        "native_runtime_invalid_request"
    );
}

#[test]
fn alibaba_mode_capabilities_and_normalization_use_shared_catalog() {
    let request = json!({"operation":"capabilities", "provider":"alibabaCloud",
            "route":"deepL", "target":"fr", "alibabaRealtimeMode":"liveTranslate"});
    let response: Value = serde_json::from_str(&exchange(&request.to_string()).unwrap()).unwrap();
    let shared = AlibabaRealtimeMode::LiveTranslate.capabilities();
    assert_eq!(response["sourceLanguages"], json!(shared.source_languages));
    assert_eq!(response["targetLanguages"], json!(shared.target_languages));
    assert_eq!(response["sampleRateHz"], json!(shared.input_sample_rate_hz));
    let mut normalize = request.clone();
    normalize["operation"] = json!("normalize");
    normalize["source"] = json!("en");
    let normalized: Value =
        serde_json::from_str(&exchange(&normalize.to_string()).unwrap()).unwrap();
    assert_eq!(
        normalized,
        json!({"sourceLanguage":"auto", "targetLanguage":"fr"})
    );
    let mut legacy = request.clone();
    legacy
        .as_object_mut()
        .unwrap()
        .remove("alibabaRealtimeMode");
    let response: Value = serde_json::from_str(&exchange(&legacy.to_string()).unwrap()).unwrap();
    assert!(response["sourceLanguages"].as_array().unwrap().len() > 1);
    for invalid in ["unknown", "asrMt"] {
        let mut request = request.clone();
        request["alibabaRealtimeMode"] = json!(invalid);
        assert_eq!(
            exchange(&request.to_string()).unwrap_err(),
            "invalid_configuration"
        );
    }
    let mut wrong_provider = request;
    wrong_provider["provider"] = json!("tencentCloud");
    assert_eq!(
        exchange(&wrong_provider.to_string()).unwrap_err(),
        "invalid_configuration"
    );
}
