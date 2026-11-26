use super::*;

#[test]
fn apple_local_failures_retry_only_transient_request_failures() {
    use mimi_core::translation_policy::RetryClass;
    for label in [
        "apple_translation_assets_missing",
        "apple_translation_language_unsupported",
        "apple_translation_unavailable",
        "apple_translation_cancelled",
    ] {
        let error = QwenMTClientError::Apple(label);
        assert_eq!(error.retry_class(), RetryClass::Permanent);
        assert!(!error.is_authentication_failure());
        assert_eq!(error.diagnostic_label(), label);
    }
    assert_eq!(
        QwenMTClientError::Apple("apple_translation_timeout").retry_class(),
        RetryClass::Temporary
    );
    assert_eq!(
        QwenMTClientError::Apple("apple_translation_busy").retry_class(),
        RetryClass::Temporary
    );
}

#[test]
fn all_lite_targets_and_explicit_intersection_sources_encode_without_model_or_prompt_changes() {
    let names = [
        "English",
        "Chinese",
        "Traditional Chinese",
        "Russian",
        "Japanese",
        "Korean",
        "Spanish",
        "French",
        "Portuguese",
        "German",
        "Italian",
        "Thai",
        "Vietnamese",
        "Indonesian",
        "Malay",
        "Arabic",
        "Hindi",
        "Hebrew",
        "Urdu",
        "Bengali",
        "Polish",
        "Dutch",
        "Turkish",
        "Khmer",
        "Czech",
        "Swedish",
        "Hungarian",
        "Danish",
        "Finnish",
        "Tagalog",
        "Persian",
    ];
    // Independent upstream full-English-name oracle, in Audio3 hint order.
    // The MT API specifies names even though the overview also lists codes.
    let source_names = [
        "auto",
        "Chinese",
        "English",
        "Japanese",
        "Korean",
        "Vietnamese",
        "Thai",
        "Indonesian",
        "Malay",
        "Tagalog",
        "Hindi",
        "Arabic",
        "French",
        "German",
        "Spanish",
        "Portuguese",
        "Russian",
        "Italian",
        "Dutch",
        "Swedish",
        "Danish",
        "Finnish",
        "Norwegian Bokmål",
        "Greek",
        "Polish",
        "Czech",
        "Hungarian",
        "Romanian",
        "Bulgarian",
        "Croatian",
        "Slovak",
    ];
    for (code, name) in QWEN_MT_LITE_LANGUAGE_CODES.iter().zip(names) {
        let target = TargetLanguage::ALL
            .into_iter()
            .find(|target| target.raw_value() == *code)
            .unwrap();
        for (source, expected_source) in SourceLanguage::ALL.into_iter().zip(source_names) {
            let result = QwenMTRequestEncoder::request(
                "Synthetic fixture.",
                source,
                target,
                REALTIME_MT_MODEL,
                true,
                Some("existing domain"),
                &[],
                &[],
            );
            if source == SourceLanguage::Automatic
                || QWEN_MT_LITE_LANGUAGE_CODES.contains(&source.raw_value())
            {
                let request = result.unwrap();
                assert_eq!(request["model"], "qwen-mt-lite");
                assert_eq!(request["translation_options"]["target_lang"], name);
                assert_eq!(
                    request["translation_options"]["source_lang"],
                    expected_source
                );
                assert!(request["translation_options"].get("domains").is_none());
                assert!(request["translation_options"].get("tm_list").is_none());
                assert_eq!(request["stream"], true);
            } else {
                assert_eq!(
                    result.unwrap_err(),
                    QwenMTProtocolError::UnsupportedLanguage
                );
            }
        }
    }
    for code in ["no", "el", "ro", "bg", "hr", "sk"] {
        assert!(!REALTIME_MT_MODEL.supports_reported_source(Some(code)));
    }
    assert!(REALTIME_MT_MODEL.supports_reported_source(None));
    assert!(REALTIME_MT_MODEL.supports_reported_source(Some("unknown")));
}

#[test]
fn rejection_codes_classify_documented_aliases_in_both_response_shapes() {
    let cases = [
        (
            "Throttling.RateQuota",
            QwenMTRejectionCategory::RequestRate,
            "request_rate",
        ),
        (
            "LimitRequests",
            QwenMTRejectionCategory::RequestRate,
            "request_rate",
        ),
        (
            "limit_requests",
            QwenMTRejectionCategory::RequestRate,
            "request_rate",
        ),
        (
            "ResourceExhausted",
            QwenMTRejectionCategory::RequestRate,
            "request_rate",
        ),
        (
            "Throttling.BurstRate",
            QwenMTRejectionCategory::BurstRate,
            "burst_rate",
        ),
        (
            "limit_burst_rate",
            QwenMTRejectionCategory::BurstRate,
            "burst_rate",
        ),
        (
            "Throttling.AllocationQuota",
            QwenMTRejectionCategory::TokenRate,
            "token_rate",
        ),
        (
            "insufficient_quota",
            QwenMTRejectionCategory::TokenRate,
            "token_rate",
        ),
        (
            "Throttling.Concurrency",
            QwenMTRejectionCategory::Concurrency,
            "concurrency",
        ),
        (
            "Throttling.ServiceOverloaded",
            QwenMTRejectionCategory::Capacity,
            "capacity",
        ),
        (
            "ServiceOverloaded",
            QwenMTRejectionCategory::Capacity,
            "capacity",
        ),
        (
            "Throttling.ResourceExhausted",
            QwenMTRejectionCategory::Capacity,
            "capacity",
        ),
        (
            "CommodityNotPurchased",
            QwenMTRejectionCategory::Billing,
            "billing",
        ),
        (
            "PrepaidBillOverdue",
            QwenMTRejectionCategory::Billing,
            "billing",
        ),
        (
            "PostpaidBillOverdue",
            QwenMTRejectionCategory::Billing,
            "billing",
        ),
        (
            "BudgetLimitExceeded",
            QwenMTRejectionCategory::Billing,
            "billing",
        ),
    ];
    for (code, category, label) in cases {
        for response in [json!({"code": code}), json!({"error": {"code": code}})] {
            let decoded = QwenMTRejectionCategory::from_response(response.to_string().as_bytes());
            assert_eq!(decoded, category);
            assert_eq!(decoded.diagnostic_label(), label);
        }
    }
}

#[test]
fn rejection_category_never_interprets_messages_or_returns_untrusted_codes() {
    const PRIVATE: &str = "private-subtitle-and-key-sentinel";
    let responses = [
        json!({"error": {"message": "insufficient_quota", "content": PRIVATE}}),
        json!({"error": {"code": PRIVATE, "message": "Throttling.RateQuota"}}),
        json!({"error": {"code": "Throttling.RateQuota\nprivate-subtitle-and-key-sentinel"}}),
        json!({"error": {"code": "throttling.ratequota"}}),
        json!({"error": {"code": 429}}),
        json!({"error": {"code": ["insufficient_quota", PRIVATE]}}),
        json!({"error": {"code": {"secret": PRIVATE}}}),
        json!({"error": {"code": PRIVATE}, "code": "insufficient_quota"}),
        json!({"message": PRIVATE, "choices": [{"content": PRIVATE}]}),
    ];
    for response in responses {
        let category = QwenMTRejectionCategory::from_response(response.to_string().as_bytes());
        assert_eq!(category, QwenMTRejectionCategory::Unknown);
        assert_eq!(category.diagnostic_label(), "unknown");
        assert!(!format!("{category:?}:{}", category.diagnostic_label()).contains(PRIVATE));
    }
    let category = QwenMTRejectionCategory::from_response(
        json!({"error": {"code": "insufficient_quota", "message": PRIVATE}, "prompt": PRIVATE})
            .to_string()
            .as_bytes(),
    );
    assert_eq!(category.diagnostic_label(), "token_rate");
}

#[test]
fn rejection_category_is_bounded_and_invalid_json_remains_unknown() {
    for response in [b"".as_slice(), b"not JSON", b"{", b"[]", b"null"] {
        assert_eq!(
            QwenMTRejectionCategory::from_response(response),
            QwenMTRejectionCategory::Unknown
        );
    }
    assert_eq!(
        QwenMTRejectionCategory::from_response(&vec![b' '; 1024 * 1024 + 1]),
        QwenMTRejectionCategory::Unknown,
    );
    assert_eq!(
        QwenMTRejectionCategory::from_response(
            br#"{"error":{"message":"ignored"},"code":"Throttling.Concurrency"}"#
        ),
        QwenMTRejectionCategory::Concurrency,
    );
}

#[test]
fn oversized_responses_are_content_free_and_not_retried() {
    let error = QwenMTClientError::ResponseTooLarge;
    assert!(!error.is_authentication_failure());
    assert_eq!(
        error.diagnostic_label(),
        "QwenMTClientError.responseTooLarge"
    );
    assert_eq!(QwenMTRetryPolicy::delay(&error, 1), None);
}

fn request(text: &str, source_language: SourceLanguage) -> Value {
    QwenMTRequestEncoder::request(
        text,
        source_language,
        TargetLanguage::SimplifiedChinese,
        QwenMTModel::Flash,
        false,
        None,
        &[],
        &[],
    )
    .unwrap()
}

#[test]
fn endpoint_builds_the_unified_chat_completions_url() {
    let endpoint = QwenMTEndpoint::new().unwrap();
    assert_eq!(
        endpoint.url.as_str(),
        "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions"
    );
}

#[test]
fn request_selects_model_and_language_names() {
    let json = request("今日は晴れです。", SourceLanguage::Japanese);
    let messages = json["messages"].as_array().unwrap();
    let options = &json["translation_options"];

    assert_eq!(json["model"], "qwen-mt-flash");
    assert_eq!(json["stream"], false);
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[0]["content"], "今日は晴れです。");
    assert_eq!(options["source_lang"], "Japanese");
    assert_eq!(options["target_lang"], "Chinese");
}

#[test]
fn request_can_enable_incremental_streaming() {
    let json = QwenMTRequestEncoder::request(
        "今日は晴れです。",
        SourceLanguage::Japanese,
        TargetLanguage::SimplifiedChinese,
        QwenMTModel::Flash,
        true,
        None,
        &[],
        &[],
    )
    .unwrap();
    assert_eq!(json["stream"], true);
}

#[test]
fn lite_default_streams_and_omits_only_unsupported_domain_prompt() {
    assert_eq!(REALTIME_MT_MODEL, QwenMTModel::Lite);
    let terms = [QwenMTTerm::new("synthetic", "fixture")];
    let memory = [QwenMTMemoryPair {
        source: "previous".into(),
        target: "confirmed".into(),
    }];
    for model in [QwenMTModel::Lite, QwenMTModel::Flash, QwenMTModel::Plus] {
        let body = QwenMTRequestEncoder::request(
            "synthetic",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            model,
            true,
            Some("unchanged domain fixture"),
            &terms,
            &memory,
        )
        .unwrap();
        assert_eq!(body["model"], model.raw_name());
        assert_eq!(body["stream"], true);
        assert_eq!(
            body["translation_options"].get("domains").is_some(),
            model != QwenMTModel::Lite
        );
        assert_eq!(body["translation_options"]["terms"], json!(terms));
        assert_eq!(body["translation_options"]["tm_list"], json!(memory));
    }
    assert_eq!(
        QwenMTRequestEncoder::request(
            "synthetic",
            SourceLanguage::English,
            TargetLanguage::Original,
            QwenMTModel::Lite,
            true,
            None,
            &[],
            &[]
        ),
        Err(QwenMTProtocolError::UnsupportedLanguage)
    );
}

#[test]
fn expanded_explicit_source_registry_respects_model_languages_and_names() {
    let bengali = serde_json::from_value(json!("bn")).unwrap();
    let body = QwenMTRequestEncoder::request(
        "Synthetic fixture.",
        bengali,
        TargetLanguage::English,
        QwenMTModel::Lite,
        false,
        None,
        &[],
        &[],
    )
    .unwrap();
    assert_eq!(body["translation_options"]["source_lang"], "Bengali");
    for code in ["ast", "ar-EG", "pt-BR", "zh_en"] {
        let source = serde_json::from_value(json!(code)).unwrap();
        assert_eq!(
            QwenMTRequestEncoder::request(
                "Synthetic fixture.",
                source,
                TargetLanguage::English,
                QwenMTModel::Lite,
                false,
                None,
                &[],
                &[],
            ),
            Err(QwenMTProtocolError::UnsupportedLanguage)
        );
    }
}

#[test]
fn model_language_registries_are_exact_unique_and_preserve_auto_boundary() {
    use std::collections::HashSet;
    assert_eq!(QWEN_MT_LITE_LANGUAGE_CODES.len(), 31);
    assert_eq!(QWEN_MT_FLASH_LANGUAGE_CODES.len(), 92);
    for model in [QwenMTModel::Lite, QwenMTModel::Flash, QwenMTModel::Plus] {
        let codes = model.supported_language_codes();
        assert_eq!(codes.iter().collect::<HashSet<_>>().len(), codes.len());
        assert!(!codes.contains(&"auto"));
        assert!(model.supports_reported_source(None));
        for code in codes {
            assert!(model.supports_reported_source(Some(code)));
        }
        for reported in ["en-US", "zh-CN", "zh-Hant", "ja-JP", "fil"] {
            assert!(model.supports_reported_source(Some(reported)));
        }
        assert!(model.supports_reported_source(Some("unknown-private-value")));
    }
    assert!(QWEN_MT_LITE_LANGUAGE_CODES
        .iter()
        .all(|code| QWEN_MT_FLASH_LANGUAGE_CODES.contains(code)));
    for audio3_only in ["no", "ro", "el", "bg", "hr", "sk"] {
        assert!(!QwenMTModel::Lite.supports_reported_source(Some(audio3_only)));
        assert!(QwenMTModel::Flash.supports_reported_source(Some(audio3_only)));
    }
}

#[test]
fn unsupported_source_is_explicit_content_free_and_not_retried() {
    let error = QwenMTClientError::UnsupportedSource;
    assert_eq!(error.to_string(), "translation_source_unsupported");
    assert_eq!(
        error.diagnostic_label(),
        "QwenMTClientError.unsupportedSource"
    );
    assert!(!error.is_authentication_failure());
    assert_eq!(error.recovery_reason(), None);
    assert_eq!(QwenMTRetryPolicy::delay(&error, 1), None);
}

#[test]
fn request_selects_an_explicit_target_language() {
    let json = QwenMTRequestEncoder::request(
        "今日は晴れです。",
        SourceLanguage::Japanese,
        TargetLanguage::English,
        QwenMTModel::Flash,
        false,
        None,
        &[],
        &[],
    )
    .unwrap();
    assert_eq!(json["translation_options"]["target_lang"], "English");
}

#[test]
fn request_can_select_the_highest_quality_plus_model() {
    let json = QwenMTRequestEncoder::request(
        "今日はいい天気ですね。",
        SourceLanguage::Japanese,
        TargetLanguage::SimplifiedChinese,
        QwenMTModel::Plus,
        false,
        None,
        &[],
        &[],
    )
    .unwrap();
    assert_eq!(json["model"], "qwen-mt-plus");
}

#[test]
fn automatic_source_uses_auto() {
    let json = request("Hello, world.", SourceLanguage::Automatic);
    assert_eq!(json["translation_options"]["source_lang"], "auto");
}

#[test]
fn spoken_dialogue_guidance_preserves_vocal_sounds() {
    let guidance = QwenMTDomainHint::spoken_dialogue(
        SourceLanguage::Japanese,
        TargetLanguage::SimplifiedChinese,
    );

    assert!(guidance.contains("gasps, moans, and cries"));
    assert!(guidance.contains("嗯、啊、呢、吧、嘛"));
    assert!(guidance.contains("えっと"));
    assert!(guidance.contains("polite or formal Japanese"));
    assert!(guidance.contains("Output only the translation text"));
    assert!(guidance.contains("tm_list"));
    assert!(guidance.contains("ongoing dialogue"));
    assert!(guidance.contains("translate only the new input"));
    assert!(guidance.contains("Do not sanitize, euphemize, censor, or omit"));
    assert!(!guidance.contains("do not mechanically translate every filler"));
}

#[test]
fn filler_glossary_pins_japanese_tone_words_for_chinese() {
    let terms =
        QwenMTDomainHint::filler_terms(SourceLanguage::Japanese, TargetLanguage::SimplifiedChinese);
    assert!(terms.contains(&QwenMTTerm::new("えっと", "那个")));
    assert!(terms.contains(&QwenMTTerm::new("うーん", "嗯")));
    assert!(terms.contains(&QwenMTTerm::new("あぁ", "啊")));
    assert!(terms.contains(&QwenMTTerm::new("まあ", "嘛")));
}

#[test]
fn filler_glossary_combines_languages_for_automatic_source() {
    let terms = QwenMTDomainHint::filler_terms(
        SourceLanguage::Automatic,
        TargetLanguage::SimplifiedChinese,
    );
    assert!(terms.contains(&QwenMTTerm::new("えっと", "那个")));
    assert!(terms.contains(&QwenMTTerm::new("um", "嗯")));
}

#[test]
fn request_encodes_the_filler_glossary_in_translation_options() {
    let terms =
        QwenMTDomainHint::filler_terms(SourceLanguage::Japanese, TargetLanguage::SimplifiedChinese);
    let json = QwenMTRequestEncoder::request(
        "えっと、うーん、あぁ。",
        SourceLanguage::Japanese,
        TargetLanguage::SimplifiedChinese,
        QwenMTModel::Flash,
        true,
        None,
        &terms,
        &[],
    )
    .unwrap();
    let encoded_terms = json["translation_options"]["terms"].as_array().unwrap();

    assert_eq!(encoded_terms.len(), terms.len());
    assert_eq!(encoded_terms[0]["source"], "えっと");
    assert_eq!(encoded_terms[0]["target"], "那个");
}

#[test]
fn request_omits_terms_when_none_are_provided() {
    let json = request("今日は晴れです。", SourceLanguage::Japanese);
    assert!(json["translation_options"].get("terms").is_none());
}

#[test]
fn request_includes_bounded_translation_memory_pairs() {
    let json = QwenMTRequestEncoder::request(
        "そうなんですね。",
        SourceLanguage::Japanese,
        TargetLanguage::SimplifiedChinese,
        QwenMTModel::Flash,
        false,
        None,
        &[],
        &[QwenMTMemoryPair {
            source: "今日は晴れです。".to_string(),
            target: "今天天气很好。".to_string(),
        }],
    )
    .unwrap();
    let memory = json["translation_options"]["tm_list"].as_array().unwrap();
    assert_eq!(memory[0]["source"], "今日は晴れです。");
    assert_eq!(memory[0]["target"], "今天天气很好。");
}

#[test]
fn asr_language_reports_resolve_to_explicit_qwen_mt_languages() {
    assert_eq!(
        SourceLanguage::from_detected(Some("ja-JP")),
        Some(SourceLanguage::Japanese)
    );
    assert_eq!(
        SourceLanguage::from_detected(Some("English")),
        Some(SourceLanguage::English)
    );
    assert_eq!(
        SourceLanguage::from_detected(Some("ko")),
        Some(SourceLanguage::Korean)
    );
    assert_eq!(
        SourceLanguage::from_detected(Some("zh-CN")),
        Some(SourceLanguage::Chinese)
    );
    assert_eq!(SourceLanguage::from_detected(Some("unknown")), None);
}

#[test]
fn response_decodes_and_trims_translated_content() {
    let translation = QwenMTResponseDecoder::decode(
        r#"{"choices":[{"message":{"role":"assistant","content":"  今天天气晴朗。  "}}]}"#,
    )
    .unwrap();
    assert_eq!(translation, "今天天气晴朗。");
}

#[test]
fn response_requires_translated_content() {
    assert!(matches!(
        QwenMTResponseDecoder::decode(r#"{"choices":[]}"#),
        Err(QwenMTProtocolError::MissingTranslation)
    ));
}

#[test]
fn response_rejects_explicit_unfinished_or_filtered_completions() {
    for reason in [
        json!("length"),
        json!("content_filter"),
        json!("tool_calls"),
        Value::Null,
    ] {
        let response = json!({"choices":[{"finish_reason":reason,"message":{"content":"Synthetic partial translation"}}]}).to_string();
        assert!(
            QwenMTResponseDecoder::decode(&response).is_err(),
            "{reason}"
        );
    }
}

#[test]
fn stream_rejects_explicit_unfinished_or_filtered_completions() {
    for reason in ["length", "content_filter", "tool_calls"] {
        let response = json!({"choices":[{"finish_reason":reason,"delta":{"content":"Synthetic partial translation"}}]}).to_string();
        assert!(
            QwenMTStreamDecoder::decode_chunk(&response).is_err(),
            "{reason}"
        );
    }
}

#[test]
fn stream_chunk_decodes_incremental_content() {
    let content = QwenMTStreamDecoder::decode_chunk(
        r#"{"choices":[{"delta":{"role":"assistant","content":"今天"}}]}"#,
    )
    .unwrap();
    let terminal = QwenMTStreamDecoder::decode_chunk(r#"{"choices":[]}"#).unwrap();
    assert_eq!(content.as_deref(), Some("今天"));
    assert_eq!(terminal, None);
}

#[test]
fn timeout_has_a_useful_error_message() {
    assert_eq!(
        QwenMTClientError::RequestTimedOut.to_string(),
        "Qwen-MT took too long to respond."
    );
}

#[test]
fn diagnostics_retain_status_without_response_content() {
    assert_eq!(
        QwenMTClientError::RequestFailed {
            status_code: 429,
            message: "sensitive response detail".into()
        }
        .diagnostic_label(),
        "QwenMTClientError.requestFailed(status=429)"
    );
}

#[test]
fn retry_policy_backs_off_only_for_transient_failures() {
    assert_eq!(
        QwenMTRetryPolicy::delay(&QwenMTClientError::RequestTimedOut, 1),
        Some(Duration::from_millis(600))
    );
    assert_eq!(
        QwenMTRetryPolicy::delay(
            &QwenMTClientError::RequestFailed {
                status_code: 429,
                message: "busy".into()
            },
            3
        ),
        Some(Duration::from_secs(8))
    );
    assert_eq!(
        QwenMTRetryPolicy::delay(
            &QwenMTClientError::RequestFailed {
                status_code: 503,
                message: "down".into()
            },
            8
        ),
        Some(Duration::from_secs(8))
    );
    assert_eq!(
        QwenMTRetryPolicy::delay(
            &QwenMTClientError::RequestFailed {
                status_code: 401,
                message: "bad key".into()
            },
            1
        ),
        None
    );
    assert_eq!(
        QwenMTRetryPolicy::delay(
            &QwenMTClientError::RequestFailed {
                status_code: 400,
                message: "bad request".into()
            },
            1
        ),
        None
    );
}

#[test]
fn recovery_classification_preserves_authentication_and_bounded_rate_limit_backoff() {
    use crate::core::diagnostics::latency::TranslationRecoveryReason;
    for error in [
        QwenMTClientError::RequestFailed {
            status_code: 429,
            message: "synthetic provider detail".into(),
        },
        QwenMTClientError::DeepL(super::super::deepl::DeepLError::Rejected(429)),
        QwenMTClientError::DeepLX(super::super::deeplx::DeepLXError::Rejected(429)),
        QwenMTClientError::OpenAICompatible(
            super::super::openai_compatible::OpenAICompatibleError::Rejected(429),
        ),
    ] {
        assert_eq!(
            error.recovery_reason(),
            Some(TranslationRecoveryReason::RateLimited)
        );
        assert!(!error.is_authentication_failure());
        assert_eq!(
            QwenMTRetryPolicy::delay(&error, 1),
            Some(Duration::from_secs(4))
        );
        assert_eq!(
            QwenMTRetryPolicy::delay(&error, 2),
            Some(Duration::from_secs(8))
        );
        assert_eq!(
            QwenMTRetryPolicy::delay(&error, usize::MAX),
            Some(Duration::from_secs(8))
        );
    }
    for status_code in [401, 403] {
        let error = QwenMTClientError::RequestFailed {
            status_code,
            message: "synthetic auth rejection".into(),
        };
        assert!(error.is_authentication_failure());
        assert_eq!(error.recovery_reason(), None);
        let error = QwenMTClientError::OpenAICompatible(
            super::super::openai_compatible::OpenAICompatibleError::Rejected(status_code),
        );
        assert!(error.is_authentication_failure());
        assert_eq!(error.recovery_reason(), None);
        assert_eq!(QwenMTRetryPolicy::delay(&error, 1), None);
    }
    for error in [
        QwenMTClientError::RequestTimedOut,
        QwenMTClientError::OpenAICompatible(
            super::super::openai_compatible::OpenAICompatibleError::Timeout,
        ),
        QwenMTClientError::OpenAICompatible(
            super::super::openai_compatible::OpenAICompatibleError::Rejected(503),
        ),
        QwenMTClientError::RequestFailed {
            status_code: 503,
            message: String::new(),
        },
    ] {
        assert_eq!(
            error.recovery_reason(),
            Some(TranslationRecoveryReason::TemporarilyUnavailable)
        );
    }
}

#[test]
fn missing_independent_translation_is_a_configuration_error_without_retry_or_auth_claims() {
    let error = QwenMTClientError::MissingTextTranslation;
    assert_eq!(error.recovery_reason(), None);
    assert!(!error.is_authentication_failure());
    assert_eq!(QwenMTRetryPolicy::delay(&error, 1), None);
    assert_eq!(
        error.diagnostic_label(),
        "TranslationClientError.missingTextTranslation"
    );
    assert!(!error.to_string().contains("Alibaba"));
}
