//! Configuration metadata and credential drafts validated by the shared runtime.
//! No provider, audio pipeline or native session is created here.

use super::{
    json, AlibabaRealtimeMode, LiveTranslationConfiguration, ProviderCredentials,
    ProviderCredentialsError, ProviderKind, ProviderPreferences, ServiceProfile, SourceLanguage,
    SpeechTranslationMode, TargetLanguage, TextTranslation, TranslationMode, Value, XAISpeechMode,
};

pub(super) fn resolve_credentials_draft(value: &Value) -> Result<Value, &'static str> {
    let provider: ProviderKind = serde_json::from_value(value["provider"].clone())
        .map_err(|_| "native_runtime_invalid_request")?;
    let mode: SpeechTranslationMode =
        serde_json::from_value(value["speechTranslationMode"].clone())
            .map_err(|_| "native_runtime_invalid_request")?;
    let draft: ProviderCredentials = serde_json::from_value(value["draft"].clone())
        .map_err(|_| "native_runtime_invalid_request")?;
    let saved: Option<ProviderCredentials> = value
        .get("saved")
        .filter(|saved| !saved.is_null())
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|_| "native_runtime_invalid_request")?;
    let mut profile = ServiceProfile::new("android-credential-draft", "Credentials", provider)
        .map_err(|_| "invalid_credentials_draft")?;
    profile
        .set_speech_translation_mode(mode)
        .map_err(|_| "invalid_credentials_draft")?;
    let credentials = draft
        .resolve_draft_for_profile(&profile, saved.as_ref())
        .map_err(|error| match error {
            ProviderCredentialsError::Missing(_) => "missing_credentials_draft",
            _ => "invalid_credentials_draft",
        })?;
    Ok(json!({"credentials": credentials}))
}

pub(super) fn normalize_text_endpoint(value: &Value) -> Result<Value, &'static str> {
    use mimi_runtime::core::protocols::{
        endpoint::HttpEndpointError, endpoint::HttpEndpointPolicy, translation::deepl,
        translation::deeplx, translation::openai_compatible,
    };
    let route: TextTranslation = serde_json::from_value(value["route"].clone())
        .map_err(|_| "native_runtime_invalid_request")?;
    let input = value["input"]
        .as_str()
        .ok_or("native_runtime_invalid_request")?;
    let policy = HttpEndpointPolicy::Android {
        allow_local_http: value["allowLocalHttp"]
            .as_bool()
            .ok_or("native_runtime_invalid_request")?,
    };
    let endpoint = match route {
        TextTranslation::DeepL => deepl::endpoint(input)
            .map_err(|_| "translation_key")?
            .to_owned(),
        TextTranslation::DeepLX | TextTranslation::OpenAICompatible | TextTranslation::ChatMock => {
            let parsed = if route == TextTranslation::DeepLX {
                deeplx::endpoint_with_policy(input, policy)
            } else {
                openai_compatible::endpoint_with_policy(input, policy)
            };
            parsed
                .map_err(|error| match error {
                    HttpEndpointError::Invalid => "translation_endpoint",
                    HttpEndpointError::HttpsRequired => "translation_https_required",
                })?
                .to_string()
        }
        _ => return Err("native_runtime_invalid_request"),
    };
    Ok(json!({"endpoint":endpoint}))
}

pub(super) fn text_endpoints_equivalent(value: &Value) -> Result<Value, &'static str> {
    let route: TextTranslation = serde_json::from_value(value["route"].clone())
        .map_err(|_| "native_runtime_invalid_request")?;
    let saved = value["savedEndpoint"]
        .as_str()
        .ok_or("native_runtime_invalid_request")?;
    let draft = value["draftEndpoint"]
        .as_str()
        .ok_or("native_runtime_invalid_request")?;
    // Delegate destination identity to the same protocol parser that
    // constructs production requests. Invalid drafts never retain auth.
    let endpoints = match route {
        TextTranslation::OpenAICompatible | TextTranslation::ChatMock => (
            mimi_runtime::core::protocols::translation::openai_compatible::endpoint(saved).ok(),
            mimi_runtime::core::protocols::translation::openai_compatible::endpoint(draft).ok(),
        ),
        TextTranslation::DeepLX => (
            mimi_runtime::core::protocols::translation::deeplx::endpoint(saved).ok(),
            mimi_runtime::core::protocols::translation::deeplx::endpoint(draft).ok(),
        ),
        _ => return Err("native_runtime_invalid_request"),
    };
    let equivalent = matches!(endpoints, (Some(saved), Some(draft)) if saved == draft);
    Ok(json!({"equivalent": equivalent}))
}

pub(super) fn capabilities_or_normalize(value: &Value) -> Result<Value, &'static str> {
    let provider: ProviderKind =
        serde_json::from_value(value["provider"].clone()).map_err(|_| "invalid_configuration")?;
    let route: TextTranslation =
        serde_json::from_value(value["route"].clone()).map_err(|_| "invalid_configuration")?;
    let target: TargetLanguage = match serde_json::from_value(value["target"].clone()) {
        Ok(language) => language,
        Err(_) if value["operation"] == "normalize" && value["target"].is_string() => {
            TargetLanguage::SimplifiedChinese
        }
        Err(_) => return Err("invalid_configuration"),
    };
    let mode: AlibabaRealtimeMode = value
        .get("alibabaRealtimeMode")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|_| "invalid_configuration")?
        .unwrap_or_default();
    if mode != AlibabaRealtimeMode::Disabled && provider != ProviderKind::AlibabaCloud {
        return Err("invalid_configuration");
    }
    let xai_mode: XAISpeechMode = value
        .get("xaiSpeechMode")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|_| "invalid_configuration")?
        .unwrap_or_default();
    if xai_mode != XAISpeechMode::Voice && provider != ProviderKind::XAIRealtime {
        return Err("invalid_configuration");
    }
    let speech_mode: SpeechTranslationMode = value
        .get("speechTranslationMode")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|_| "invalid_configuration")?
        .unwrap_or_default();
    if speech_mode != SpeechTranslationMode::Simultaneous
        && !provider.supports_speech_translation_mode()
    {
        return Err("invalid_configuration");
    }
    let capabilities_for_target = |target| {
        if speech_mode == SpeechTranslationMode::AsrMt {
            mimi_runtime::core::services::provider::realtime_asr_capabilities(
                provider, route, target,
            )
        } else if mode != AlibabaRealtimeMode::Disabled {
            mode.capabilities()
        } else if provider == ProviderKind::XAIRealtime && xai_mode == XAISpeechMode::AsrMt {
            mimi_runtime::core::services::provider::xai_asr_capabilities(route, target)
        } else if provider == ProviderKind::XAIRealtime {
            xai_mode.capabilities()
        } else {
            provider.capabilities_for_route(route, target)
        }
    };
    let capabilities = capabilities_for_target(target);
    if value["operation"] == "normalize" {
        let source: SourceLanguage = match serde_json::from_value(value["source"].clone()) {
            Ok(language) => language,
            Err(_) if value["source"].is_string() => SourceLanguage::Automatic,
            Err(_) => return Err("invalid_configuration"),
        };
        let first = capabilities.normalize(ProviderPreferences {
            source_language: source,
            target_language: target,
            translation_mode: TranslationMode::Turbo,
        });
        let normalized = capabilities_for_target(first.target_language).normalize(first);
        Ok(
            json!({"sourceLanguage":normalized.source_language,"targetLanguage":normalized.target_language}),
        )
    } else {
        let pairs: serde_json::Map<String, Value> = capabilities
            .source_languages
            .iter()
            .map(|source| {
                (
                    source.raw_value().into(),
                    json!(capabilities.clone().for_source(*source).target_languages),
                )
            })
            .collect();
        Ok(
            json!({"sourceLanguages":capabilities.source_languages,"targetLanguages":capabilities.target_languages,"sampleRateHz":capabilities.input_sample_rate_hz,"targetPairs":pairs}),
        )
    }
}

pub(super) fn endpoints(value: &Value) -> Result<Value, &'static str> {
    let configuration: LiveTranslationConfiguration =
        serde_json::from_value(value["configuration"].clone())
            .map_err(|_| "invalid_configuration")?;
    let (speech, text) = configuration
        .network_endpoints()
        .map_err(|_| "invalid_configuration")?;
    Ok(
        json!({"speechEndpoint":speech,"textEndpoint":text,"modelNames":configuration.model_names()}),
    )
}
