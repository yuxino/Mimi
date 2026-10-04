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
