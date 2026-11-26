use super::*;

#[test]
fn signed_provider_status_diagnostics_retain_only_canonical_numeric_codes() {
    let marker = "synthetic-secret https://private.invalid /private/caption";
    for (prefix, code) in [
        ("baidu_provider_", "BAIDU_PROVIDER_ERROR"),
        ("tencent_provider_", "TENCENT_PROVIDER_ERROR"),
    ] {
        for status in [i64::MIN, -1, 0, 6_004, 31_003, i64::MAX] {
            let label = format!("{prefix}{status}");
            let failure = SafeFailure::from_provider_error(&label, marker);
            assert_eq!(failure, SafeFailure::from_error(&label));
            assert_eq!(failure.phase, "websocket_connection");
            assert_eq!(failure.category, "request_rejected");
            assert_eq!(failure.code, code);
            let mut journal = DiagnosticJournal::default();
            journal.record(
                DiagnosticEvent::Failure {
                    audio_source: None,
                    classification: failure,
                },
                20,
            );
            let report = render(DiagnosticFacts {
                last_error: Some((failure, 0)),
                journal: journal.snapshot(),
                ..Default::default()
            });
            let value: serde_json::Value = serde_json::from_str(&report).unwrap();
            assert_eq!(
                value["last_error"]["classification"]["provider_status_code"],
                status
            );
            assert_eq!(
                value["journal"]["recent_events"][0]["classification"]["provider_status_code"],
                status
            );
            for excluded in [
                "synthetic-secret",
                "private.invalid",
                "/private/caption",
                prefix,
            ] {
                assert!(!report.contains(excluded));
            }
        }
    }
}

#[test]
fn signed_provider_status_diagnostics_reject_noncanonical_and_overflow_values() {
    for prefix in ["baidu_provider_", "tencent_provider_"] {
        for suffix in [
            "",
            "+1",
            "01",
            "-01",
            "-0",
            "9223372036854775808",
            "-9223372036854775809",
            "6004.private",
            "6004\n",
            " 6004",
            "６００４",
            "private-token",
        ] {
            let label = format!("{prefix}{suffix}");
            for failure in [
                SafeFailure::from_error(&label),
                SafeFailure::from_provider_error(&label, "private-message"),
            ] {
                assert_eq!(failure.code, "OTHER", "{label:?}");
                assert_eq!(failure.provider_status_code, None);
                assert!(!serde_json::to_string(&failure).unwrap().contains("private"));
            }
        }
    }
}

#[test]
fn baidu_protocol_diagnostics_recognize_only_client_owned_labels() {
    let label = "baidu_protocol_error";
    let message = "Baidu realtime translation returned an invalid response.";
    let failure = SafeFailure::from_provider_error(label, "synthetic-private-detail");
    assert_eq!(failure, SafeFailure::from_error(label));
    assert_eq!(failure, SafeFailure::from_error(message));
    assert_eq!(
        failure,
        SafeFailure::from_provider_error("unknown", message)
    );
    assert_eq!(failure.phase, "websocket_connection");
    assert_eq!(failure.category, "protocol");
    assert_eq!(failure.code, "BAIDU_PROTOCOL_ERROR");
    assert_eq!(failure.provider_status_code, None);
    for unsafe_value in [format!("{label}.private"), format!("{message} private")] {
        assert_eq!(SafeFailure::from_error(&unsafe_value).code, "OTHER");
    }
}

#[test]
fn tencent_rejection_display_diagnostics_never_invent_ambiguous_status() {
    use crate::clients::realtime::tencent_cloud_client::TencentCloudClientError;
    for (status, category, code) in [
        (6001, "configuration", "TENCENT_CONFIGURATION_REJECTED"),
        (6003, "configuration", "TENCENT_SERVICE_ACTIVATION_REQUIRED"),
        (6004, "quota", "TENCENT_QUOTA_EXHAUSTED"),
        (6005, "quota", "TENCENT_QUOTA_EXHAUSTED"),
        (6006, "rate_limit", "TENCENT_CAPACITY_EXCEEDED"),
        (9999, "request_rejected", "TENCENT_PROVIDER_REJECTED"),
    ] {
        let message = TencentCloudClientError::ProviderRejected { code: status }.to_string();
        let failure = SafeFailure::from_error(&message);
        assert_eq!(
            failure,
            SafeFailure::from_provider_error(&message, "private detail")
        );
        assert_eq!(failure.category, category);
        assert_eq!(failure.code, code);
        assert_eq!(failure.provider_status_code, None);
        assert_eq!(
            SafeFailure::from_error(&format!("{message} private")).code,
            "OTHER"
        );
    }
}

#[test]
fn deeplx_diagnostics_keep_fixed_display_categories_and_typed_rejection_status() {
    use crate::core::protocols::translation::deeplx::DeepLXError;
    for (error, category, code) in [
        (
            DeepLXError::Endpoint,
            "configuration",
            "DEEPLX_ENDPOINT_INVALID",
        ),
        (DeepLXError::Timeout, "timeout", "DEEPLX_TIMEOUT"),
        (
            DeepLXError::Connection,
            "transport",
            "DEEPLX_CONNECTION_FAILED",
        ),
        (DeepLXError::Response, "protocol", "DEEPLX_INVALID_RESPONSE"),
        (
            DeepLXError::TooLarge,
            "size_limit",
            "DEEPLX_RESPONSE_TOO_LARGE",
        ),
    ] {
        let expected = SafeFailure::from_error(&error.to_string());
        assert_eq!(expected.phase, "text_translation");
        assert_eq!(expected.category, category);
        assert_eq!(expected.code, code);
        assert_eq!(expected.provider_status_code, None);
        assert_eq!(expected, SafeFailure::from_error(&error.diagnostic_label()));
        assert_eq!(
            expected,
            SafeFailure::from_provider_error("unknown", &error.to_string())
        );
        assert_eq!(
            expected,
            SafeFailure::from_provider_error(&error.diagnostic_label(), "synthetic-private-detail")
        );
        for unsafe_value in [
            format!("{} private", error),
            format!("{} private", error.diagnostic_label()),
        ] {
            assert_eq!(SafeFailure::from_error(&unsafe_value).code, "OTHER");
        }
    }
    for status in [0, 302, 401, 403, 429, 500, u16::MAX] {
        let error = DeepLXError::Rejected(status);
        for failure in [
            SafeFailure::from_error(&error.to_string()),
            SafeFailure::from_error(&error.diagnostic_label()),
            SafeFailure::from_provider_error(&error.diagnostic_label(), "synthetic-private-detail"),
        ] {
            let value = serde_json::to_value(failure).unwrap();
            assert_eq!(failure.phase, "text_translation");
            assert_eq!(failure.category, "request_rejected");
            assert_eq!(failure.code, "DEEPLX_REQUEST_REJECTED");
            assert_eq!(value["provider_status_code"], status);
            assert!(!value.to_string().contains("synthetic-private-detail"));
        }
    }
    for status in ["-1", "+401", "0401", "65536", "401.private", "401\n"] {
        for label in [format!("deeplx.rejected(code={status})"), format!("DeepLX rejected the request (code {status}). Check the endpoint and optional Bearer token with your server administrator.")] {
            assert_eq!(SafeFailure::from_error(&label).code, "OTHER", "{label:?}");
        }
    }
}

#[test]
fn built_in_setup_failures_keep_only_fixed_diagnostic_labels() {
    for label in [
        "openai_realtime_provider_error",
        "xai_realtime_provider_error",
        "gemini_live_provider_error",
        "baidu_realtime_provider_error",
        "audio3_provider_error",
        "gemini_live_service_unavailable",
    ] {
        let failure = SafeFailure::from_provider_error(label, "synthetic-private-detail");
        assert_eq!(failure, SafeFailure::from_error(label));
        assert_ne!(failure.code, "OTHER");
        assert_eq!(
            SafeFailure::from_error(&format!("{label}: synthetic-private-detail")).code,
            "OTHER"
        );
        assert!(!serde_json::to_string(&failure)
            .unwrap()
            .contains("synthetic-private-detail"));
    }
}

#[test]
fn unsupported_recognition_language_is_safe_configuration_diagnostic() {
    let failure =
        SafeFailure::from_error("audio3_error.setup.unsupported_language.UNSUPPORTED_LANGUAGE");
    let value = serde_json::to_value(failure).unwrap();
    assert_eq!(value["phase"], "asr_setup");
    assert_eq!(value["category"], "configuration");
    assert_eq!(value["code"], "UNSUPPORTED_LANGUAGE");
}

#[test]
fn recording_permission_and_user_stop_have_distinct_content_free_diagnostics() {
    for message in [
        "System audio capture permission was denied.",
        "capture.permission_denied",
    ] {
        let failure = SafeFailure::from_error(message);
        assert_eq!(failure.category, "permission");
        assert_eq!(failure.code, "SYSTEM_AUDIO_PERMISSION_DENIED");
    }
    for message in [
        "System audio capture was stopped by the user.",
        "capture.user_stopped",
    ] {
        let failure = SafeFailure::from_error(message);
        assert_eq!(failure.category, "user_stopped");
        assert_eq!(failure.code, "CAPTURE_USER_STOPPED");
    }
    assert_eq!(
        SafeFailure::from_error("private native error text").code,
        "OTHER"
    );
}

#[test]
fn microphone_diagnostics_use_fixed_labels_without_device_names() {
    let facts = DiagnosticFacts {
        output_selection: OutputSelection::DefaultMicrophone,
        ..Default::default()
    };
    let report = render(facts);
    assert!(report.contains("default_microphone"));
    let dual = render(DiagnosticFacts {
        output_selection: OutputSelection::SystemAndMicrophone,
        ..Default::default()
    });
    assert!(dual.contains("system_and_microphone"));
    for (message, code) in [
        (
            "Microphone capture permission was denied.",
            "MICROPHONE_PERMISSION_DENIED",
        ),
        (
            "No default microphone is available.",
            "MICROPHONE_UNAVAILABLE",
        ),
        (
            "Microphone capture could not be started.",
            "MICROPHONE_START_FAILED",
        ),
    ] {
        assert_eq!(SafeFailure::from_error(message).code, code);
    }
    assert_eq!(
        SafeFailure::from_error("Private microphone name: failed").code,
        "OTHER"
    );
}

#[test]
fn issue_link_has_a_fixed_public_destination_and_only_safe_facts() {
    let issue = issue_link(DiagnosticFacts {
        last_error: Some((
            SafeFailure::from_error(
                "private-token /Users/private https://private.invalid original-text",
            ),
            7,
        )),
        ..Default::default()
    });
    let url = url::Url::parse(&issue.url).unwrap();
    assert_eq!(url.host_str(), Some("github.com"));
    assert_eq!(url.path(), "/yuxino/mimi/issues/new");
    assert!(issue.url.len() <= 6000);
    let body = url.query_pairs().find(|(key, _)| key == "body").unwrap().1;
    assert!(body.contains(&issue.report));
    for secret in [
        "private-token",
        "/Users/private",
        "private.invalid",
        "original-text",
    ] {
        assert!(!body.contains(secret));
    }
    assert!(body.contains("before submitting"));
    assert!(!issue.requires_paste);
}

#[test]
fn windows_output_categories_are_values_not_private_names() {
    for (selection, availability) in [
        (OutputSelection::SystemDefault, Availability::Available),
        (OutputSelection::ManualOutput, Availability::Unavailable),
    ] {
        let report = render(DiagnosticFacts {
            output_selection: selection,
            output_availability: availability,
            ..Default::default()
        });
        assert!(report.contains("output_selection"));
    }
}

#[test]
fn unknown_and_oversized_errors_cannot_leak_into_reports() {
    let marker =
        "synthetic-secret /Users/private audio-words https://private.invalid/api?token=secret";
    for error in [
        marker.to_string(),
        marker.repeat(10000),
        "audio3_error.recognition.timeout.private-token".into(),
        "audio3_error.recognition.timeout.CLIENT_ERROR.private-text".into(),
    ] {
        let report = render(DiagnosticFacts {
            last_error: Some((SafeFailure::from_error(&error), 0)),
            ..Default::default()
        });
        for excluded in [
            "synthetic-secret",
            "/Users/private",
            "audio-words",
            "https://private",
            "private-token",
            "private-text",
        ] {
            assert!(!report.contains(excluded));
        }
        assert!(report.len() < 4096);
    }
}

#[test]
fn missing_state_is_unknown_and_timeout_does_not_assert_wrong_output() {
    let report = render(DiagnosticFacts::default());
    let json: serde_json::Value = serde_json::from_str(&report).unwrap();
    assert_eq!(json["capture"]["observation"], serde_json::Value::Null);
    assert_eq!(json["os"]["version"], "unknown");
    // Preparing diagnostics must not trigger another Keychain read or
    // mistake a saved credential/healthy transport for authentication.
    assert_eq!(json["credentials"]["state"], "unknown");
    assert_eq!(json["credentials"]["authentication"], "unknown");
    let error = SafeFailure::from_error("audio3_error.recognition.timeout.CLIENT_ERROR");
    assert_eq!(error.phase, "asr_recognition");
    assert_eq!(error.category, "timeout");
    assert_ne!(error.category, "device_unavailable");
}

#[test]
fn credential_storage_categories_use_fixed_content_free_codes() {
    for (label, code) in [
        (
            "credential_store_unavailable",
            "CREDENTIAL_STORE_UNAVAILABLE",
        ),
        (
            "credential_service_unavailable",
            "CREDENTIAL_SERVICE_UNAVAILABLE",
        ),
        (
            "credential_store_access_denied",
            "CREDENTIAL_STORE_ACCESS_DENIED",
        ),
    ] {
        let failure = SafeFailure::from_error(label);
        assert_eq!(failure.phase, "setup");
        assert_eq!(failure.category, "credential_storage");
        assert_eq!(failure.code, code);
    }
    assert_eq!(
        SafeFailure::from_error("credential_service_unavailable: private detail").code,
        "OTHER"
    );
}

#[test]
fn concurrent_reports_keep_schema_bounded_and_independent() {
    std::thread::scope(|scope| {
        for index in 0..16 {
            scope.spawn(move || {
                for _ in 0..50 {
                    let report = render(DiagnosticFacts {
                        elapsed_ms: index,
                        last_error: Some((
                            SafeFailure::from_error(
                                "audio3_error.setup.authentication.INVALID_API_KEY",
                            ),
                            1,
                        )),
                        ..Default::default()
                    });
                    let json: serde_json::Value = serde_json::from_str(&report).unwrap();
                    assert_eq!(json["schema"], "mimi.support.v2");
                    assert_eq!(
                        json["last_error"]["classification"]["category"],
                        "authentication"
                    );
                    assert_eq!(json["snapshot"]["elapsed_since_app_start_ms"], index);
                    assert!(report.len() < 4096);
                }
            });
        }
    });
}

#[test]
fn mt_failure_labels_and_provider_codes_remain_fixed_and_private() {
    let marker = "synthetic-secret /Users/private caption-words https://private.invalid";
    for (label, category, code) in [
        (
            "translation_rate_limited",
            "rate_limit",
            "TRANSLATION_RATE_LIMITED",
        ),
        (
            "translation_temporarily_unavailable",
            "service_error",
            "TRANSLATION_TEMPORARILY_UNAVAILABLE",
        ),
        (
            "translation_backlog_overflow",
            "backlog",
            "TRANSLATION_BACKLOG_OVERFLOW",
        ),
    ] {
        let failure = SafeFailure::from_provider_error(label, marker);
        assert_eq!(failure.phase, "text_translation");
        assert_eq!(failure.category, category);
        assert_eq!(failure.code, code);
        assert_eq!(SafeFailure::from_error(label), failure);
        assert_eq!(
            SafeFailure::from_error(&format!("{label}: {marker}")).code,
            "OTHER"
        );
        let mut journal = DiagnosticJournal::default();
        journal.record(
            DiagnosticEvent::Failure {
                audio_source: None,
                classification: failure,
            },
            20,
        );
        let report = render(DiagnosticFacts {
            journal: journal.snapshot(),
            ..Default::default()
        });
        for excluded in [
            "synthetic-secret",
            "/Users/private",
            "caption-words",
            "private.invalid",
        ] {
            assert!(!report.contains(excluded));
        }
    }
    assert_eq!(
        SafeFailure::from_provider_error(marker, marker).code,
        "OTHER"
    );
}

#[test]
fn apple_speech_failures_preserve_phase_without_guessing_or_exposing_native_errors() {
    let marker = "synthetic-secret /Users/private caption-words https://private.invalid";
    for (label, phase, category) in [
        (
            "apple_speech_recognition_failed",
            "asr_recognition",
            "recognition",
        ),
        (
            "apple_speech_runtime_service_unavailable",
            "asr_recognition",
            "service_error",
        ),
        ("apple_speech_runtime_timeout", "asr_recognition", "timeout"),
        (
            "apple_speech_stream_closed",
            "asr_recognition",
            "recognition",
        ),
        ("apple_speech_result_backlog", "asr_recognition", "backlog"),
        (
            "apple_speech_invalid_result",
            "asr_recognition",
            "recognition",
        ),
        ("apple_speech_audio_failed", "audio_send", "recognition"),
        ("apple_speech_finalize_timeout", "session_close", "timeout"),
        (
            "apple_speech_service_unavailable",
            "asr_setup",
            "service_error",
        ),
        ("apple_speech_setup_timeout", "asr_setup", "timeout"),
        ("apple_speech_start_failed", "asr_setup", "recognition"),
    ] {
        let failure = SafeFailure::from_provider_error(label, marker);
        assert_eq!(failure.phase, phase);
        assert_eq!(failure.category, category);
        assert_eq!(failure.code, label.to_ascii_uppercase());
        assert_eq!(failure.provider_status_code, None);
        assert_eq!(SafeFailure::from_error(label), failure);
        for unsafe_label in [format!("{label}: {marker}"), format!("{label}_unknown")] {
            assert_eq!(SafeFailure::from_error(&unsafe_label).code, "OTHER");
            assert_eq!(
                SafeFailure::from_provider_error(&unsafe_label, marker).code,
                "OTHER"
            );
        }
        let mut journal = DiagnosticJournal::default();
        journal.record(
            DiagnosticEvent::Failure {
                audio_source: None,
                classification: failure,
            },
            707_700,
        );
        let report = render(DiagnosticFacts {
            last_error: Some((failure, 0)),
            journal: journal.snapshot(),
            ..Default::default()
        });
        let json: serde_json::Value = serde_json::from_str(&report).unwrap();
        assert_eq!(json["last_error"]["classification"]["category"], category);
        assert_eq!(
            json["journal"]["recent_events"][0]["classification"]["phase"],
            phase
        );
        assert!(json["unknown"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("root_cause")));
        for excluded in [
            "synthetic-secret",
            "/Users/private",
            "caption-words",
            "private.invalid",
        ] {
            assert!(!report.contains(excluded));
        }
    }
    assert_eq!(
        SafeFailure::from_provider_error(marker, marker).code,
        "OTHER"
    );
}

#[test]
fn baidu_unexpected_end_preserves_only_the_exact_safe_label() {
    let label = "baidu_unexpected_session_end";
    let message = "Baidu realtime translation ended the session unexpectedly.";
    let marker = "private-provider-detail";
    let failure = SafeFailure::from_provider_error(label, marker);
    assert_eq!(failure.phase, "websocket_connection");
    assert_eq!(failure.category, "service_error");
    assert_eq!(failure.code, "BAIDU_UNEXPECTED_SESSION_END");
    assert_eq!(SafeFailure::from_error(label), failure);
    assert_eq!(SafeFailure::from_error(message), failure);
    assert_eq!(
        SafeFailure::from_provider_error("unknown", message),
        failure
    );
    for unknown in [format!("{label}:{marker}"), format!("{message} {marker}")] {
        assert_eq!(SafeFailure::from_error(&unknown).code, "OTHER");
        assert_eq!(
            SafeFailure::from_provider_error(&unknown, marker).code,
            "OTHER"
        );
    }
    let mut journal = DiagnosticJournal::default();
    journal.record(
        DiagnosticEvent::Failure {
            audio_source: None,
            classification: failure,
        },
        20,
    );
    let report = render(DiagnosticFacts {
        last_error: Some((failure, 0)),
        journal: journal.snapshot(),
        ..Default::default()
    });
    assert!(report.contains("BAIDU_UNEXPECTED_SESSION_END"));
    assert!(!report.contains(marker));
}

#[test]
fn volcano_provider_status_survives_last_error_and_journal_without_raw_message() {
    let marker = "synthetic-secret /Users/private caption-words https://private.invalid";
    // Published AST error codes, plus numeric boundaries and an undocumented code.
    for status in [
        0,
        45000001,
        45000002,
        45000081,
        45000151,
        55000031,
        55000999,
        i32::MAX,
    ] {
        let label = format!("volcano_provider_error.{status}");
        let failure = SafeFailure::from_provider_error(&label, marker);
        assert_eq!(failure.phase, "websocket_connection");
        assert_eq!(failure.category, "service_error");
        assert_eq!(failure.code, "VOLCANO_PROVIDER_ERROR");
        assert_eq!(failure.provider_status_code, Some(i64::from(status)));
        assert_eq!(SafeFailure::from_error(&label), failure);
        let mut journal = DiagnosticJournal::default();
        journal.record(
            DiagnosticEvent::Failure {
                audio_source: None,
                classification: failure,
            },
            20,
        );
        let report = render(DiagnosticFacts {
            last_error: Some((failure, 0)),
            journal: journal.snapshot(),
            ..Default::default()
        });
        let value: serde_json::Value = serde_json::from_str(&report).unwrap();
        let last = &value["last_error"]["classification"];
        let recent = &value["journal"]["recent_events"][0]["classification"];
        assert_eq!(last["provider_status_code"], status);
        assert_eq!(recent["provider_status_code"], status);
        assert_eq!(last["code"], "VOLCANO_PROVIDER_ERROR");
        for excluded in [
            "synthetic-secret",
            "/Users/private",
            "caption-words",
            "private.invalid",
        ] {
            assert!(!report.contains(excluded));
        }
    }
}

#[test]
fn volcano_terminal_labels_are_exact_and_distinct_from_normal_finish() {
    for (label, message, phase, category, code) in [
        (
            "volcano_provider_error",
            "Volcano Engine rejected the translation session.",
            "websocket_connection",
            "service_error",
            "VOLCANO_PROVIDER_ERROR",
        ),
        (
            "volcano_unexpected_session_finished",
            "Volcano Engine ended the translation session unexpectedly.",
            "websocket_connection",
            "service_error",
            "VOLCANO_UNEXPECTED_SESSION_FINISHED",
        ),
        (
            "volcano_protocol_error",
            "Volcano Engine returned an invalid response.",
            "websocket_connection",
            "service_error",
            "VOLCANO_PROTOCOL_ERROR",
        ),
        (
            "volcano_close_timeout",
            "Volcano Engine did not finish closing in time.",
            "session_close",
            "timeout",
            "VOLCANO_CLOSE_TIMEOUT",
        ),
    ] {
        let failure = SafeFailure::from_provider_error(label, "private response body");
        assert_eq!(
            (failure.phase, failure.category, failure.code),
            (phase, category, code)
        );
        assert_eq!(SafeFailure::from_error(label), failure);
        assert_eq!(SafeFailure::from_error(message), failure);
        assert_eq!(
            SafeFailure::from_provider_error("unknown", message),
            failure
        );
        // Existing reports keep their old shape when no numeric code was observed.
        assert!(serde_json::to_value(failure)
            .unwrap()
            .get("provider_status_code")
            .is_none());
        for unknown in [format!("{label}: private"), format!("{message} private")] {
            assert_eq!(
                SafeFailure::from_provider_error(&unknown, "private").code,
                "OTHER"
            );
        }
    }
    assert_eq!(SafeFailure::from_error("session_finished").code, "OTHER");
    assert_eq!(
        SafeFailure::from_provider_error("volcano_session_close_failed", "private").code,
        "VOLCANO_SESSION_CLOSE_FAILED"
    );
    // Prefer the close-specific code even though its fixed message is shared with transport errors.
    assert_eq!(
        SafeFailure::from_provider_error(
            "volcano_session_close_failed",
            "The Volcano Engine connection failed."
        )
        .code,
        "VOLCANO_SESSION_CLOSE_FAILED"
    );
}

#[test]
fn volcano_malformed_status_codes_do_not_become_diagnostic_content() {
    for suffix in [
        "",
        "-1",
        "+1",
        "01",
        "2147483648",
        "99999999999999999999",
        "45000081.private",
        "45000081\n",
        " 45000081",
        "４５００００８１",
        "private-token",
    ] {
        let label = format!("volcano_provider_error.{suffix}");
        for failure in [
            SafeFailure::from_error(&label),
            SafeFailure::from_provider_error(&label, "private-message"),
        ] {
            assert_eq!(failure.code, "OTHER", "{suffix:?}");
            assert_eq!(failure.provider_status_code, None);
            let json = serde_json::to_string(&failure).unwrap();
            assert!(!json.contains("private"));
            assert!(!json.contains("provider_status_code"));
        }
    }
}

#[test]
fn openai_expiry_and_quota_diagnostics_keep_safe_reasons() {
    for (label, category, code) in [
        (
            "openai_realtime_session_expired",
            "timeout",
            "OPENAI_REALTIME_SESSION_EXPIRED",
        ),
        (
            "openai_realtime_quota_exhausted",
            "configuration",
            "OPENAI_REALTIME_QUOTA_EXHAUSTED",
        ),
    ] {
        let failure =
            SafeFailure::from_provider_error(label, "synthetic private provider response");
        assert_eq!(
            (failure.phase, failure.category, failure.code),
            ("websocket_connection", category, code)
        );
        assert_eq!(SafeFailure::from_error(label), failure);
        assert_eq!(
            SafeFailure::from_provider_error(&format!("{label}.private"), "private").code,
            "OTHER"
        );
    }
}

#[test]
fn alibaba_recovery_codes_preserve_safe_reasons_without_provider_messages() {
    for (label, code) in [
        (
            "alibaba_realtime_idle_timeout",
            "ALIBABA_REALTIME_IDLE_TIMEOUT",
        ),
        (
            "live_translate_unexpected_session_end",
            "LIVE_TRANSLATE_UNEXPECTED_SESSION_END",
        ),
        (
            "alibaba_realtime_service_unavailable",
            "ALIBABA_REALTIME_SERVICE_UNAVAILABLE",
        ),
        (
            "alibaba_realtime_provider_error",
            "ALIBABA_REALTIME_PROVIDER_ERROR",
        ),
        (
            "alibaba_realtime_response_failed",
            "ALIBABA_REALTIME_RESPONSE_FAILED",
        ),
    ] {
        let failure = SafeFailure::from_provider_error(label, "private response body");
        assert_eq!(failure.code, code);
        assert_eq!(SafeFailure::from_error(label), failure);
        assert!(!serde_json::to_string(&failure).unwrap().contains("private"));
        assert_eq!(
            SafeFailure::from_provider_error(&format!("{label}.private"), "private").code,
            "OTHER"
        );
    }
}

#[test]
fn alibaba_connection_errors_keep_the_observed_boundary_without_private_details() {
    use mimi_runtime::clients::realtime::live_translate_client::LiveTranslateClientError as Live;
    use mimi_runtime::clients::recognition::audio3_client::Audio3ASRClientError as Asr;

    let cases = [
        (
            Live::NotConnected.to_string(),
            "websocket_connection",
            "transport",
            "LIVE_TRANSLATE_NOT_CONNECTED",
        ),
        (
            Live::HealthCheckTimedOut.to_string(),
            "websocket_connection",
            "timeout",
            "LIVE_TRANSLATE_HEALTH_TIMEOUT",
        ),
        (
            Live::ConnectionTimedOut.to_string(),
            "websocket_connection",
            "timeout",
            "LIVE_TRANSLATE_CONNECTION_TIMEOUT",
        ),
        (
            Live::SessionSetupTimedOut.to_string(),
            "setup",
            "timeout",
            "LIVE_TRANSLATE_SETUP_TIMEOUT",
        ),
        (
            Live::SessionSetupRejected.to_string(),
            "setup",
            "request_rejected",
            "LIVE_TRANSLATE_SETUP_REJECTED",
        ),
        (
            Live::TransportFailure.to_string(),
            "websocket_connection",
            "transport",
            "LIVE_TRANSLATE_TRANSPORT_ERROR",
        ),
        (
            Asr::NotConnected.to_string(),
            "websocket_connection",
            "transport",
            "ASR_NOT_CONNECTED",
        ),
        (
            Asr::HealthCheckTimedOut.to_string(),
            "websocket_connection",
            "timeout",
            "ASR_HEALTH_TIMEOUT",
        ),
        (
            Asr::ConnectionTimedOut.to_string(),
            "websocket_connection",
            "timeout",
            "ASR_CONNECTION_TIMEOUT",
        ),
        (
            Asr::TransportFailure.to_string(),
            "websocket_connection",
            "transport",
            "ASR_TRANSPORT_ERROR",
        ),
    ];
    for (message, phase, category, code) in cases {
        let failure = SafeFailure::from_error(&message);
        assert_eq!(
            (failure.phase, failure.category, failure.code),
            (phase, category, code)
        );
        assert_eq!(
            SafeFailure::from_provider_error("unknown", &message),
            failure
        );
        assert_eq!(failure.provider_status_code, None);
        for unsafe_message in [
            format!("{message} private-detail"),
            format!("private {message}"),
            format!("{message}\n"),
        ] {
            assert_eq!(SafeFailure::from_error(&unsafe_message).code, "OTHER");
        }
    }
}

#[test]
fn recovered_health_failure_remains_in_the_bounded_support_report() {
    use mimi_runtime::clients::realtime::live_translate_client::LiveTranslateClientError;
    for audio_source in [
        None,
        Some(crate::core::audio::audio_input::AudioSource::System),
        Some(crate::core::audio::audio_input::AudioSource::Microphone),
    ] {
        let failure =
            SafeFailure::from_error(&LiveTranslateClientError::HealthCheckTimedOut.to_string());
        let mut journal = DiagnosticJournal::default();
        journal.record(
            DiagnosticEvent::Failure {
                audio_source,
                classification: failure,
            },
            10,
        );
        journal.record(
            DiagnosticEvent::Recovery {
                action: RecoveryAction::Retrying,
            },
            11,
        );
        journal.record(
            DiagnosticEvent::Recovery {
                action: RecoveryAction::Recovered,
            },
            20,
        );
        let report: serde_json::Value = serde_json::from_str(&render(DiagnosticFacts {
            last_error: Some((failure, 10)),
            recovery: Some((RecoveryAction::Recovered, 0)),
            journal: journal.snapshot(),
            ..Default::default()
        }))
        .unwrap();
        assert_eq!(
            report["journal"]["recent_events"][0]["classification"]["code"],
            "LIVE_TRANSLATE_HEALTH_TIMEOUT"
        );
        assert_eq!(
            report["last_error"]["classification"]["code"],
            "LIVE_TRANSLATE_HEALTH_TIMEOUT"
        );
        assert_eq!(report["recent_recovery"]["action"], "recovered");
        assert_eq!(
            report["journal"]["recent_events"][0]["audio_source"],
            serde_json::to_value(audio_source).unwrap()
        );
        if audio_source.is_none() {
            assert!(report["journal"]["recent_events"][0]
                .get("audio_source")
                .is_none());
        }
        assert!(report["unknown"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("root_cause")));
    }
}

#[test]
fn alibaba_route_is_distinct_from_the_legacy_translation_mode() {
    for (route, expected) in [
        (
            Some(AlibabaRealtimeMode::LiveTranslate),
            serde_json::json!("liveTranslate"),
        ),
        (
            Some(AlibabaRealtimeMode::Disabled),
            serde_json::json!("disabled"),
        ),
        (None, serde_json::Value::Null),
    ] {
        let report: serde_json::Value = serde_json::from_str(&render(DiagnosticFacts {
            provider: Some(ProviderKind::AlibabaCloud),
            mode: TranslationMode::Turbo,
            alibaba_realtime_mode: route,
            ..Default::default()
        }))
        .unwrap();
        assert_eq!(report["mode"], "turbo");
        assert_eq!(report["alibaba_realtime_mode"], expected);
    }
}

#[test]
fn volcano_fixed_connection_errors_separate_setup_health_and_transport() {
    for (message, code) in [
        (
            "The Volcano Engine connection failed.",
            "VOLCANO_TRANSPORT_ERROR",
        ),
        (
            "The Volcano Engine transport failed.",
            "VOLCANO_TRANSPORT_ERROR",
        ),
        (
            "The Volcano Engine translation session is not connected.",
            "VOLCANO_NOT_CONNECTED",
        ),
        (
            "The Volcano Engine connection stopped responding.",
            "VOLCANO_HEALTH_TIMEOUT",
        ),
        (
            "The Volcano Engine connection could not be established in time.",
            "VOLCANO_CONNECTION_TIMEOUT",
        ),
        (
            "Volcano Engine did not confirm the session configuration in time.",
            "VOLCANO_SETUP_TIMEOUT",
        ),
        (
            "Volcano Engine rejected the session configuration.",
            "VOLCANO_SETUP_REJECTED",
        ),
    ] {
        let failure = SafeFailure::from_error(message);
        assert_eq!(failure.code, code);
        assert_eq!(failure.provider_status_code, None);
        assert_eq!(
            SafeFailure::from_error(&format!("{message} private-detail")).code,
            "OTHER"
        );
    }
}

#[test]
fn journal_is_bounded_monotonic_and_draft_counts_do_not_evict_state_changes() {
    let mut journal = DiagnosticJournal::default();
    journal.observe_status(DiagnosticStatus::Connecting, 8);
    journal.observe_status(DiagnosticStatus::Connecting, 9);
    for index in 1..100 {
        journal.record(
            DiagnosticEvent::Recovery {
                action: RecoveryAction::Retrying,
            },
            if index % 2 == 0 { 2 } else { index },
        );
    }
    for _ in 0..2_000 {
        journal.observe_text(TextEventKind::SourceDraft);
    }
    journal.observe_text(TextEventKind::ConfirmedPair);
    journal.counts.translation_draft_events = u64::MAX;
    journal.observe_text(TextEventKind::TranslationDraft);
    let snapshot = journal.snapshot();
    assert_eq!(snapshot.events_observed, 100);
    assert_eq!(snapshot.events_omitted, 100 - RECENT_EVENT_LIMIT as u64);
    assert_eq!(snapshot.recent_events.len(), RECENT_EVENT_LIMIT);
    assert_eq!(snapshot.recent_events.last().unwrap().sequence, 100);
    assert!(snapshot
        .recent_events
        .windows(2)
        .all(|pair| pair[0].sequence < pair[1].sequence
            && pair[0].elapsed_since_app_start_ms <= pair[1].elapsed_since_app_start_ms));
    assert_eq!(snapshot.text_event_counts.source_draft_events, 2_000);
    assert_eq!(snapshot.text_event_counts.confirmed_pair_events, 1);
    assert_eq!(
        snapshot.text_event_counts.translation_draft_events,
        u64::MAX
    );
}

#[test]
fn full_journal_report_fits_the_byte_bound_and_uses_the_fixed_paste_fallback() {
    let recovery = TranslationRecovery {
        reason:
            crate::core::diagnostics::latency::TranslationRecoveryReason::TemporarilyUnavailable,
        retry_after_ms: u64::MAX,
        retry_scheduled: false,
    };
    let mut journal = DiagnosticJournal::default();
    for _ in 0..RECENT_EVENT_LIMIT {
        journal.record(DiagnosticEvent::TranslationBackoff { recovery }, u64::MAX);
    }
    let issue = issue_link(DiagnosticFacts {
        elapsed_ms: u64::MAX,
        api_latency_ms: Some(u64::MAX),
        translation_latency_ms: Some(u64::MAX),
        translation_latency_kind: Some(TranslationLatencyKind::Request),
        translation_recovery: Some(recovery),
        journal: journal.snapshot(),
        ..Default::default()
    });
    assert!(issue.report.len() <= REPORT_BYTE_LIMIT);
    let report: serde_json::Value = serde_json::from_str(&issue.report).unwrap();
    assert_eq!(
        report["journal"]["recent_events"].as_array().unwrap().len(),
        RECENT_EVENT_LIMIT
    );
    assert_eq!(
        report["service"]["translation_recovery"]["retryScheduled"],
        false
    );
    assert_eq!(report["service"]["translation_duration_kind"], "request");
    assert!(issue.requires_paste);
    assert!(issue.url.len() <= 6000);
    let url = url::Url::parse(&issue.url).unwrap();
    assert_eq!(url.host_str(), Some("github.com"));
    assert_eq!(url.path(), "/yuxino/mimi/issues/new");
    let body = url.query_pairs().find(|(key, _)| key == "body").unwrap().1;
    assert!(body.contains("Paste the diagnostic snapshot"));
    assert!(!body.contains("translation_backoff"));
}
