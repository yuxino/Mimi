//! The same synthetic wire examples also run against the Android adapters.
use super::{deepl, deeplx, gemini_live::GeminiLiveRequestEncoder, openai_compatible};
use crate::core::{
    credentials::TextTranslationCredentials,
    models::{SourceLanguage, TargetLanguage},
};
use serde_json::Value;

fn contract() -> Value {
    let value: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../shared/translation-contracts.json"
    )))
    .unwrap();
    assert_eq!(value["schemaVersion"], 1);
    for name in [
        "requests",
        "detectedSourceRequests",
        "endpoints",
        "responses",
        "models",
        "credentials",
        "optionalAuthorization",
        "liveSetups",
        "liveTranscriptSequences",
    ] {
        let cases = value[name].as_array().unwrap();
        assert!(!cases.is_empty(), "empty {name} contract");
        for case in cases {
            assert!(case["id"].is_string());
            assert!(case.get("expected").is_some());
            if let Some(provider) = case.get("provider") {
                assert!(matches!(
                    provider.as_str(),
                    Some("deepL" | "deepLX" | "openaiCompatible" | "googleGeminiLive")
                ));
            }
        }
    }
    value
}

#[test]
fn shared_request_contracts() {
    let fixtures = contract();
    for case in fixtures["requests"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let text = case["text"].as_str().unwrap();
        let source: SourceLanguage = serde_json::from_value(case["source"].clone()).unwrap();
        let target: TargetLanguage = serde_json::from_value(case["target"].clone()).unwrap();
        let actual = match case["provider"].as_str().unwrap() {
            "deepL" => deepl::request(text, source, target).map_err(|_| ()),
            "deepLX" => deeplx::request(text, source, target).map_err(|_| ()),
            "openaiCompatible" => {
                openai_compatible::request(text, source, target, case["model"].as_str().unwrap())
                    .map_err(|_| ())
            }
            _ => panic!("unknown provider in {id}"),
        };
        if case["expected"].is_null() {
            assert!(actual.is_err(), "{id}");
        } else {
            assert_eq!(actual.unwrap(), case["expected"], "{id}");
        }
    }
}

#[test]
fn generic_requests_cover_every_configurable_source_and_target() {
    let fixtures = contract();
    let requests: Vec<_> = fixtures["requests"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| case["provider"] == "openaiCompatible" && !case["expected"].is_null())
        .collect();
    for source in SourceLanguage::ALL {
        assert!(
            requests
                .iter()
                .any(|case| case["source"] == source.raw_value()),
            "missing generic source {}",
            source.raw_value()
        );
    }
    for target in TargetLanguage::ALL {
        assert_eq!(
            requests
                .iter()
                .any(|case| case["target"] == target.raw_value()),
            target != TargetLanguage::Original,
            "generic target {}",
            target.raw_value()
        );
    }
}

#[test]
fn shared_detected_source_request_contracts() {
    let fixtures = contract();
    for case in fixtures["detectedSourceRequests"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let text = case["text"].as_str().unwrap();
        let source: SourceLanguage = serde_json::from_value(case["source"].clone()).unwrap();
        let detected = SourceLanguage::from_detected(case["reported"].as_str());
        let target: TargetLanguage = serde_json::from_value(case["target"].clone()).unwrap();
        let actual =
            match case["provider"].as_str().unwrap() {
                "deepL" => deepl::request_with_detected_source(text, source, detected, target)
                    .map_err(|_| ()),
                "deepLX" => deeplx::request_with_detected_source(text, source, detected, target)
                    .map_err(|_| ()),
                _ => panic!("unknown detected-source provider in {id}"),
            };
        if case["expected"].is_null() {
            assert!(actual.is_err(), "{id}");
        } else {
            assert_eq!(actual.unwrap(), case["expected"], "{id}");
        }
    }
}

#[test]
fn shared_endpoint_contracts() {
    let fixtures = contract();
    for case in fixtures["endpoints"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let input = case["input"].as_str().unwrap();
        let actual = match case["provider"].as_str().unwrap() {
            "deepL" => deepl::endpoint(input).map(str::to_owned).map_err(|_| ()),
            "deepLX" => deeplx::endpoint(input)
                .map(|url| url.to_string())
                .map_err(|_| ()),
            "openaiCompatible" => openai_compatible::endpoint(input)
                .map(|url| url.to_string())
                .map_err(|_| ()),
            _ => panic!("unknown provider in {id}"),
        };
        if let Some(expected) = case["expected"].as_str() {
            assert_eq!(actual.unwrap(), expected, "{id}");
        } else {
            assert!(actual.is_err(), "{id}");
        }
    }
}

#[test]
fn shared_response_contracts() {
    let fixtures = contract();
    for case in fixtures["responses"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let body = case["body"].as_str().unwrap().as_bytes();
        let actual = match case["provider"].as_str().unwrap() {
            "deepL" => deepl::decode(body).map_err(|_| ()),
            "deepLX" => deeplx::decode(body).map_err(|_| ()),
            "openaiCompatible" => openai_compatible::decode(body).map_err(|_| ()),
            _ => panic!("unknown provider in {id}"),
        };
        if let Some(expected) = case["expected"].as_str() {
            assert_eq!(actual.unwrap(), expected, "{id}");
        } else {
            assert!(actual.is_err(), "{id}");
        }
    }
}

#[test]
fn shared_model_contracts() {
    let fixtures = contract();
    for case in fixtures["models"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let actual = openai_compatible::validate_model(case["input"].as_str().unwrap());
        if let Some(expected) = case["expected"].as_str() {
            assert_eq!(actual.unwrap(), expected, "{id}");
        } else {
            assert!(actual.is_err(), "{id}");
        }
    }
}

fn credentials_accepted(provider: &str, key: &str) -> bool {
    let credentials = match provider {
        "deepL" => TextTranslationCredentials::DeepL {
            api_key: key.into(),
        },
        "deepLX" => TextTranslationCredentials::DeepLX {
            endpoint: "https://example.test/v1".into(),
            token: key.into(),
        },
        "openaiCompatible" => TextTranslationCredentials::OpenAICompatible {
            endpoint: "https://example.test/v1".into(),
            model: "synthetic-model".into(),
            api_key: key.into(),
        },
        _ => panic!("unknown credential provider"),
    };
    let Ok(validated) = credentials.validated() else {
        return false;
    };
    match validated {
        TextTranslationCredentials::DeepL { api_key } => deepl::endpoint(&api_key).is_ok(),
        _ => true,
    }
}

#[test]
fn shared_saved_credential_contracts_and_limits() {
    let fixtures = contract();
    for case in fixtures["credentials"].as_array().unwrap() {
        assert_eq!(
            credentials_accepted(
                case["provider"].as_str().unwrap(),
                case["apiKey"].as_str().unwrap()
            ),
            case["expected"].as_bool().unwrap(),
            "{}",
            case["id"].as_str().unwrap()
        );
    }
    let limit = fixtures["configurationLimits"]["credentialUnicodeScalars"]
        .as_u64()
        .unwrap() as usize;
    for provider in ["deepL", "deepLX", "openaiCompatible"] {
        assert!(
            credentials_accepted(provider, &"s".repeat(limit)),
            "{provider}"
        );
        assert!(
            !credentials_accepted(provider, &"s".repeat(limit + 1)),
            "{provider}"
        );
        if provider != "deepL" {
            assert!(
                credentials_accepted(provider, &"🐱".repeat(limit)),
                "{provider}"
            );
            assert!(
                !credentials_accepted(provider, &"🐱".repeat(limit + 1)),
                "{provider}"
            );
        }
    }
    let model_limit = fixtures["configurationLimits"]["modelUtf8Bytes"]
        .as_u64()
        .unwrap() as usize;
    assert!(openai_compatible::validate_model(&"m".repeat(model_limit)).is_ok());
    assert!(openai_compatible::validate_model(&"m".repeat(model_limit + 1)).is_err());
}

#[test]
fn shared_live_setup_contracts() {
    for case in contract()["liveSetups"].as_array().unwrap() {
        assert_eq!(case["provider"], "googleGeminiLive");
        let target: TargetLanguage = serde_json::from_value(case["target"].clone()).unwrap();
        assert_eq!(
            GeminiLiveRequestEncoder::setup(target).unwrap(),
            case["expected"],
            "{}",
            case["id"]
        );
    }
}

#[test]
fn shared_speech_language_setups_and_catalogs() {
    use super::{
        azure_openai_realtime::AzureOpenAIRealtimeRequestEncoder,
        openai_realtime::OpenAIRealtimeRequestEncoder, xai_realtime::XAIRealtimeRequestEncoder,
    };
    use crate::core::provider::ProviderKind;
    for case in contract()["speechLanguageSetups"].as_array().unwrap() {
        let target = serde_json::from_value(case["target"].clone()).unwrap();
        let source = serde_json::from_value(case["source"].clone()).unwrap();
        let actual = match case["provider"].as_str().unwrap() {
            "openAIRealtime" => OpenAIRealtimeRequestEncoder::session_update(target, None).map(|value| serde_json::json!({"targetCode": value["session"]["audio"]["output"]["language"]})).map_err(|_| ()),
            "azureOpenAIRealtime" => AzureOpenAIRealtimeRequestEncoder::session_update(target, "synthetic-transcription", None).map(|value| serde_json::json!({"targetCode": value["session"]["audio"]["output"]["language"]})).map_err(|_| ()),
            "googleGeminiLive" => GeminiLiveRequestEncoder::setup(target).map(|value| serde_json::json!({"targetCode": value["setup"]["generationConfig"]["translationConfig"]["targetLanguageCode"]})).map_err(|_| ()),
            "xAIRealtime" => XAIRealtimeRequestEncoder::session_update(source, target, None).map(|value| {
                let mut actual = serde_json::json!({"sourceHint": value["session"]["audio"]["input"]["transcription"]["language_hint"]});
                if case["expected"].get("instructions").is_some() {
                    actual["instructions"] = value["session"]["instructions"].clone();
                }
                actual
            }).map_err(|_| ()),
            _ => panic!("unknown provider"),
        };
        if case["expected"].is_null() {
            assert!(actual.is_err(), "{}", case["id"]);
        } else {
            assert_eq!(actual.unwrap(), case["expected"], "{}", case["id"]);
        }
    }
    for case in contract()["speechLanguageCatalogs"].as_array().unwrap() {
        let provider: ProviderKind = serde_json::from_value(case["provider"].clone()).unwrap();
        let capabilities = provider.capabilities();
        assert_eq!(
            serde_json::to_value(capabilities.source_languages).unwrap(),
            case["expected"]["sourceLanguages"],
            "{}",
            case["id"]
        );
        assert_eq!(
            serde_json::to_value(capabilities.target_languages).unwrap(),
            case["expected"]["targetLanguages"],
            "{}",
            case["id"]
        );
    }
}

#[test]
fn shared_text_language_catalogs_cover_every_supported_wire_code() {
    use super::deepl_languages;
    use std::collections::BTreeSet;
    let fixtures = contract();
    for (provider, sources, targets) in [
        (
            "deepL",
            deepl_languages::DEEPL_SOURCE_CODES,
            deepl_languages::DEEPL_TARGET_CODES,
        ),
        (
            "deepLX",
            deepl_languages::DEEPLX_SOURCE_CODES,
            deepl_languages::DEEPLX_TARGET_CODES,
        ),
    ] {
        let catalog = &fixtures["textLanguageCatalogs"][provider];
        assert_eq!(
            catalog["sourceLanguages"],
            serde_json::json!(sources),
            "{provider}"
        );
        assert_eq!(
            catalog["targetLanguages"],
            serde_json::json!(targets),
            "{provider}"
        );
        let successful = fixtures["requests"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|case| case["provider"] == provider && !case["expected"].is_null())
            .collect::<Vec<_>>();
        let actual_sources: BTreeSet<_> = successful
            .iter()
            .map(|case| case["source"].as_str().unwrap())
            .filter(|code| *code != "auto")
            .collect();
        let actual_targets: BTreeSet<_> = successful
            .iter()
            .map(|case| case["target"].as_str().unwrap())
            .collect();
        assert_eq!(
            actual_sources,
            sources.iter().copied().collect(),
            "{provider} source fixture coverage"
        );
        assert_eq!(
            actual_targets,
            targets.iter().copied().collect(),
            "{provider} target fixture coverage"
        );
    }
}
