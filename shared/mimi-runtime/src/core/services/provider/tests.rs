use super::capabilities::{independent_catalog, source_languages, target_languages};
use super::*;
use crate::core::models::{SourceLanguage, TargetLanguage, TranslationMode};
use crate::core::protocols::{
    realtime::baidu_translate, realtime::gemini_live, realtime::openai_realtime,
    realtime::tencent_cloud, realtime::xai_realtime, recognition::audio3,
};
use crate::core::services::network_proxy::ProxyConfig;
use serde_json::json;

#[test]
fn realtime_modes_default_to_simultaneous_and_preserve_independent_selection() {
    for provider in [
        ProviderKind::OpenAIRealtime,
        ProviderKind::GoogleGeminiLive,
        ProviderKind::AzureOpenAIRealtime,
    ] {
        let mut profile = ServiceProfile::new("route", "Route", provider).unwrap();
        assert_eq!(
            profile.speech_translation_mode,
            SpeechTranslationMode::Simultaneous
        );
        let mut legacy = serde_json::to_value(&profile).unwrap();
        legacy
            .as_object_mut()
            .unwrap()
            .remove("speechTranslationMode");
        assert_eq!(
            serde_json::from_value::<ServiceProfile>(legacy)
                .unwrap()
                .speech_translation_mode,
            SpeechTranslationMode::Simultaneous
        );
        profile
            .set_speech_translation_mode(SpeechTranslationMode::AsrMt)
            .unwrap();
        assert_eq!(
            profile.text_translation(),
            TextTranslation::OpenAICompatible
        );
        assert!(profile.is_standalone_asr());
        assert!(profile
            .capabilities(TargetLanguage::Original)
            .target_languages
            .contains(&TargetLanguage::Original));
        profile.text_translation = Some(TextTranslation::DeepL);
        profile
            .set_speech_translation_mode(SpeechTranslationMode::Simultaneous)
            .unwrap();
        assert_eq!(profile.text_translation(), TextTranslation::FollowService);
        assert_eq!(
            profile.validated().unwrap().text_translation,
            Some(TextTranslation::DeepL)
        );
        profile
            .set_speech_translation_mode(SpeechTranslationMode::AsrMt)
            .unwrap();
        assert_eq!(profile.text_translation(), TextTranslation::DeepL);
    }
    assert!(
        ServiceProfile::new("other", "Other", ProviderKind::VolcanoEngine)
            .unwrap()
            .set_speech_translation_mode(SpeechTranslationMode::AsrMt)
            .is_err()
    );
}

#[test]
fn xai_profiles_preserve_legacy_voice_and_opt_into_streaming_for_new_profiles() {
    let legacy: ServiceProfile =
        serde_json::from_str(r#"{"id":"legacy-xai","name":"xAI","provider":"xAIRealtime"}"#)
            .unwrap();
    let legacy = legacy.validated().unwrap();
    assert_eq!(legacy.xai_speech_mode, XAISpeechMode::Voice);
    assert!(!legacy
        .capabilities(TargetLanguage::Original)
        .target_languages
        .contains(&TargetLanguage::Original));
    let mut profile = ServiceProfile::new("new-xai", "xAI", ProviderKind::XAIRealtime).unwrap();
    assert!(profile.uses_xai_asr());
    assert_eq!(
        profile
            .capabilities(TargetLanguage::Original)
            .input_sample_rate_hz,
        16_000
    );
    assert!(profile
        .capabilities(TargetLanguage::Original)
        .target_languages
        .contains(&TargetLanguage::Original));
    profile
        .set_xai_speech(XAISpeechMode::AsrMt, " custom-text-model ")
        .unwrap();
    let restored: ServiceProfile =
        serde_json::from_value(serde_json::to_value(profile.validated().unwrap()).unwrap())
            .unwrap();
    assert_eq!(restored.xai_text_model, "custom-text-model");
    assert!(profile
        .set_xai_speech(XAISpeechMode::AsrMt, "bad\nmodel")
        .is_err());
    let mut other = ServiceProfile::alibaba_default();
    assert!(other
        .set_xai_speech(XAISpeechMode::AsrMt, DEFAULT_XAI_TEXT_MODEL)
        .is_err());
    assert!(other
        .set_xai_speech(XAISpeechMode::Voice, "other-model")
        .is_err());
}

#[test]
fn workspace_ids_match_the_realtime_endpoint_dns_label() {
    for valid in ["a", "123456", "test-workspace"] {
        assert!(alibaba_workspace_id_is_valid(valid));
    }
    assert!(alibaba_workspace_id_is_valid(&"a".repeat(63)));
    for invalid in ["", "-start", "end-", "under_score", "space id", "path/id"] {
        assert!(!alibaba_workspace_id_is_valid(invalid));
    }
    assert!(!alibaba_workspace_id_is_valid(&"a".repeat(64)));
}

#[test]
fn experimental_realtime_metadata_defaults_and_keeps_independent_route() {
    let mut profile: ServiceProfile =
        serde_json::from_str(r#"{"id":"legacy","name":"Alibaba","provider":"alibabaCloud"}"#)
            .unwrap();
    assert_eq!(profile.alibaba_realtime_mode, AlibabaRealtimeMode::Disabled);
    assert!(profile.alibaba_workspace_id.is_empty());
    profile
        .set_alibaba_realtime(AlibabaRealtimeMode::LiveTranslate, "")
        .unwrap();
    assert_eq!(profile.validated().unwrap(), profile);
    profile.text_translation = Some(TextTranslation::DeepLX);
    profile
        .set_alibaba_realtime(AlibabaRealtimeMode::LiveTranslate, " test-workspace ")
        .unwrap();
    let restored = profile.validated().unwrap();
    assert_eq!(restored.alibaba_workspace_id, "test-workspace");
    assert_eq!(restored.text_translation(), TextTranslation::FollowService);
    assert_eq!(restored.effective_provider(), ProviderKind::AlibabaCloud);
    assert_eq!(
        restored
            .capabilities(TargetLanguage::English)
            .source_languages,
        vec![SourceLanguage::Automatic]
    );
    assert_eq!(restored.text_translation, Some(TextTranslation::DeepLX));
    profile
        .set_alibaba_realtime(AlibabaRealtimeMode::Disabled, "test-workspace")
        .unwrap();
    assert_eq!(profile.text_translation(), TextTranslation::DeepLX);
    let serialized = serde_json::to_string(&restored).unwrap();
    assert_eq!(
        serde_json::from_str::<ServiceProfile>(&serialized).unwrap(),
        restored
    );
    let mut other = ServiceProfile::new("other", "Other", ProviderKind::OpenAIRealtime).unwrap();
    assert!(other
        .set_alibaba_realtime(AlibabaRealtimeMode::LiveTranslate, "test-workspace")
        .is_err());
}

#[test]
fn removed_realtime_experiment_metadata_falls_back_without_losing_saved_settings() {
    let profile: ServiceProfile = serde_json::from_str(
        r#"{"id":"experiment","name":"Alibaba","provider":"alibabaCloud","alibabaRealtimeMode":"omni","alibabaWorkspaceId":"test-workspace","textTranslation":"deepLX"}"#,
    )
    .unwrap();
    let profile = profile.validated().unwrap();
    assert_eq!(profile.alibaba_realtime_mode, AlibabaRealtimeMode::Disabled);
    assert_eq!(profile.alibaba_workspace_id, "test-workspace");
    assert_eq!(profile.text_translation(), TextTranslation::DeepLX);
    assert_eq!(profile.effective_provider(), ProviderKind::DeepLX);
    let value = serde_json::to_value(profile).unwrap();
    assert_eq!(value["alibabaRealtimeMode"], "disabled");
}

#[test]
fn qwen_model_metadata_defaults_for_legacy_profiles_and_round_trips() {
    use crate::core::protocols::translation::qwen_mt::QwenMTModel;
    let legacy = r#"{"id":"legacy","name":"Alibaba","provider":"alibabaCloud"}"#;
    let mut profile: ServiceProfile = serde_json::from_str(legacy).unwrap();
    assert_eq!(profile.qwen_mt_model, QwenMTModel::Lite);
    for model in [QwenMTModel::Lite, QwenMTModel::Flash, QwenMTModel::Plus] {
        profile.qwen_mt_model = model;
        let validated = profile.validated().unwrap();
        let restored: ServiceProfile =
            serde_json::from_str(&serde_json::to_string(&validated).unwrap()).unwrap();
        assert_eq!(restored.qwen_mt_model, model);
    }
    assert!(serde_json::from_str::<ServiceProfile>(
        &legacy.replace("\"provider\"", "\"qwenMtModel\":\"unknown\",\"provider\"")
    )
    .is_err());
}

#[test]
fn language_presets_survive_validation_but_reject_incompatible_activation() {
    let baidu = ServiceProfile::new("baidu", "Baidu", ProviderKind::BaiduTranslate).unwrap();
    assert!(baidu
        .validate_language_preset(ProfileLanguagePreset {
            source_language: SourceLanguage::Chinese,
            target_language: TargetLanguage::SimplifiedChinese,
        })
        .is_err());
    let mut profile = ServiceProfile::new("apple", "Apple", ProviderKind::AppleSpeech).unwrap();
    let pair = ProfileLanguagePreset {
        source_language: SourceLanguage::Japanese,
        target_language: TargetLanguage::SimplifiedChinese,
    };
    profile.text_translation = Some(TextTranslation::Apple);
    profile.validate_language_preset(pair).unwrap();
    profile.language_preset = Some(pair);
    profile.text_translation = None;
    let restored = profile.validated().unwrap();
    assert_eq!(restored.language_preset, Some(pair));
    assert_eq!(
        restored.validate_language_preset(pair).unwrap_err(),
        "profile_language_preset_unsupported"
    );
    let legacy = serde_json::to_value(ServiceProfile::alibaba_default()).unwrap();
    assert!(legacy.get("languagePreset").is_none());
    assert!(serde_json::from_value::<ServiceProfile>(legacy)
        .unwrap()
        .language_preset
        .is_none());
    assert!(serde_json::from_str::<ProfileLanguagePresetPatch>("{}").is_err());
    assert!(
        serde_json::from_str::<ProfileLanguagePresetPatch>(r#"{"preset": null}"#)
            .unwrap()
            .preset
            .is_none()
    );
    assert!(serde_json::from_str::<ProfileLanguagePresetPatch>(
        r#"{"preset": {"sourceLanguage": "bad", "targetLanguage": "zh"}}"#
    )
    .is_err());
}

#[test]
fn explicit_target_switches_work_for_translation_only_services() {
    for provider in [
        ProviderKind::GoogleGeminiLive,
        ProviderKind::OpenAIRealtime,
        ProviderKind::AzureOpenAIRealtime,
    ] {
        let profile = ServiceProfile::new("target-test", "Target", provider).unwrap();
        let prefs = ProviderPreferences {
            source_language: SourceLanguage::Automatic,
            target_language: TargetLanguage::SimplifiedChinese,
            translation_mode: TranslationMode::Turbo,
        };
        let selected = profile
            .preferences_after_target_switch(prefs, TargetLanguage::German)
            .unwrap();
        assert_eq!(selected.source_language, SourceLanguage::Automatic);
        assert_eq!(selected.target_language, TargetLanguage::German);
        assert_eq!(
            profile.preferences_after_target_switch(prefs, TargetLanguage::Original),
            None
        );
    }
}

#[test]
fn explicit_target_switches_reject_invalid_directions_without_substitution() {
    for (provider, source, accepted, rejected) in [
        (
            ProviderKind::TencentCloud,
            SourceLanguage::Russian,
            TargetLanguage::SimplifiedChinese,
            TargetLanguage::Japanese,
        ),
        (
            ProviderKind::TencentCloud,
            SourceLanguage::ChineseEnglishMixed,
            TargetLanguage::ChineseEnglishMixed,
            TargetLanguage::French,
        ),
        (
            ProviderKind::VolcanoEngine,
            SourceLanguage::French,
            TargetLanguage::English,
            TargetLanguage::Japanese,
        ),
        (
            ProviderKind::XAIRealtime,
            SourceLanguage::French,
            TargetLanguage::English,
            TargetLanguage::French,
        ),
    ] {
        let mut profile = ServiceProfile::new("target-test", "Target", provider).unwrap();
        // Keep this matrix on the historical integrated Voice route.
        if provider == ProviderKind::XAIRealtime {
            profile
                .set_xai_speech(XAISpeechMode::Voice, DEFAULT_XAI_TEXT_MODEL)
                .unwrap();
        }
        let prefs = ProviderPreferences {
            source_language: source,
            target_language: TargetLanguage::SimplifiedChinese,
            translation_mode: TranslationMode::Turbo,
        };
        assert_eq!(
            profile
                .preferences_after_target_switch(prefs, accepted)
                .unwrap(),
            ProviderPreferences {
                target_language: accepted,
                ..prefs
            }
        );
        assert_eq!(
            profile.preferences_after_target_switch(prefs, rejected),
            None
        );
    }
    let profile = ServiceProfile::alibaba_default();
    let prefs = ProviderPreferences {
        source_language: SourceLanguage::English,
        target_language: TargetLanguage::Original,
        translation_mode: TranslationMode::Turbo,
    };
    assert_eq!(
        profile
            .preferences_after_target_switch(prefs, TargetLanguage::English)
            .unwrap()
            .target_language,
        TargetLanguage::English
    );
    assert_eq!(
        profile.preferences_after_target_switch(prefs, TargetLanguage::Original),
        Some(prefs)
    );
}

#[test]
fn apple_text_route_preserves_recognition_identity_and_explicit_translated_sources() {
    assert_eq!(
        serde_json::to_string(&TextTranslation::Apple).unwrap(),
        "\"apple\""
    );
    for provider in [
        ProviderKind::AlibabaCloud,
        ProviderKind::DeepLX,
        ProviderKind::CustomOpenAIASR,
        ProviderKind::AppleSpeech,
    ] {
        let mut profile = ServiceProfile::new("local", "Local", provider).unwrap();
        profile.text_translation = Some(TextTranslation::Apple);
        assert!(profile.validated().is_ok());
        assert!(!profile
            .capabilities(TargetLanguage::English)
            .source_languages
            .contains(&SourceLanguage::Automatic));
        assert!(profile
            .capabilities(TargetLanguage::English)
            .target_languages
            .contains(&TargetLanguage::TraditionalChinese));
        if provider != ProviderKind::AppleSpeech {
            assert!(profile
                .capabilities(TargetLanguage::Original)
                .source_languages
                .contains(&SourceLanguage::Automatic));
        }
    }
}

#[test]
fn apple_source_catalog_respects_the_selected_text_encoder() {
    for route in [TextTranslation::DeepL, TextTranslation::DeepLX] {
        let translated =
            ProviderKind::AppleSpeech.capabilities_for_route(route, TargetLanguage::English);
        let (sources, targets) = independent_catalog(route);
        assert_eq!(translated.source_languages, source_languages(sources));
        assert_eq!(
            translated.target_languages,
            [vec![TargetLanguage::Original], target_languages(targets)].concat()
        );
        assert!(translated
            .source_languages
            .contains(&SourceLanguage::French));
        assert!(!translated
            .source_languages
            .contains(&SourceLanguage::Automatic));
        assert!(!translated
            .source_languages
            .contains(&SourceLanguage::Asturian));
        let original =
            ProviderKind::AppleSpeech.capabilities_for_route(route, TargetLanguage::Original);
        assert!(original
            .source_languages
            .contains(&SourceLanguage::Asturian));
        assert!(!original
            .source_languages
            .contains(&SourceLanguage::Automatic));
    }
    for route in [TextTranslation::OpenAICompatible, TextTranslation::ChatMock] {
        let translated =
            ProviderKind::AppleSpeech.capabilities_for_route(route, TargetLanguage::English);
        assert!(translated
            .source_languages
            .contains(&SourceLanguage::French));
        assert!(translated
            .source_languages
            .contains(&SourceLanguage::Asturian));
        assert!(!translated
            .source_languages
            .contains(&SourceLanguage::Automatic));
    }
}

#[test]
fn profile_proxy_fields_read_legacy_and_normalize_without_losing_preferences() {
    use crate::core::services::network_proxy::ProxyMode;
    let legacy: ServiceProfile =
        serde_json::from_str(r#"{"id":"legacy","name":"Legacy","provider":"alibabaCloud"}"#)
            .unwrap();
    assert_eq!(legacy.speech_network_proxy, None);
    assert_eq!(legacy.text_network_proxy, None);
    let mut profile = legacy;
    profile.speech_network_proxy = Some(ProxyConfig {
        mode: ProxyMode::Custom,
        url: Some(" socks5h://127.0.0.1 ".into()),
    });
    profile.text_network_proxy = Some(ProxyConfig {
        mode: ProxyMode::Direct,
        url: Some("stale-address".into()),
    });
    let validated = profile.validated().unwrap();
    assert_eq!(
        validated
            .speech_network_proxy
            .as_ref()
            .unwrap()
            .url
            .as_deref(),
        Some("socks5h://127.0.0.1:1080")
    );
    assert_eq!(validated.text_network_proxy.as_ref().unwrap().url, None);
    let saved: ServiceProfile =
        serde_json::from_str(&serde_json::to_string(&validated).unwrap()).unwrap();
    assert_eq!(saved, validated);
    profile.speech_network_proxy.as_mut().unwrap().url =
        Some("http://user:private-value@localhost:7890".into());
    let error = profile.validated().unwrap_err();
    assert!(!error.to_string().contains("private-value"));
    assert_eq!(
        error.to_string(),
        "network_proxy_authentication_unsupported"
    );
    assert!(!format!("{profile:?}").contains("private-value"));
}

#[test]
fn alibaba_route_catalogs_match_audio3_lite_intersection_and_original() {
    let profile = ServiceProfile::alibaba_default();
    let translated = profile.capabilities(TargetLanguage::French);
    assert_eq!(
        translated
            .source_languages
            .iter()
            .map(|source| source.raw_value())
            .collect::<Vec<_>>(),
        [
            "auto", "zh", "en", "ja", "ko", "vi", "th", "id", "ms", "tl", "hi", "ar", "fr", "de",
            "es", "pt", "ru", "it", "nl", "sv", "da", "fi", "pl", "cs", "hu"
        ]
    );
    assert_eq!(translated.target_languages[0], TargetLanguage::Original);
    assert_eq!(
        translated.target_languages[1..]
            .iter()
            .map(|target| target.raw_value())
            .collect::<Vec<_>>(),
        crate::core::protocols::translation::qwen_mt::QWEN_MT_LITE_LANGUAGE_CODES
    );
    assert_eq!(
        profile
            .capabilities(TargetLanguage::Original)
            .source_languages,
        [
            vec![SourceLanguage::Automatic],
            source_languages(audio3::LANGUAGE_CODES)
        ]
        .concat()
    );
    let normalized = profile.normalize_preferences(ProviderPreferences {
        source_language: SourceLanguage::Greek,
        target_language: TargetLanguage::French,
        translation_mode: TranslationMode::Turbo,
    });
    assert_eq!(normalized.source_language, SourceLanguage::Automatic);
    assert_eq!(normalized.target_language, TargetLanguage::French);
}

#[test]
fn independent_text_routes_and_other_providers_do_not_inherit_lite() {
    let mut profile = ServiceProfile::alibaba_default();
    for route in [TextTranslation::DeepL, TextTranslation::DeepLX] {
        profile.text_translation = Some(route);
        let (sources, targets) = independent_catalog(route);
        let original = profile.capabilities(TargetLanguage::Original);
        assert_eq!(
            original.source_languages,
            [
                vec![SourceLanguage::Automatic],
                source_languages(audio3::LANGUAGE_CODES)
            ]
            .concat()
        );
        assert_eq!(
            original.target_languages,
            [vec![TargetLanguage::Original], target_languages(targets)].concat()
        );
        let translated = profile.capabilities(TargetLanguage::French);
        let expected_sources = std::iter::once(SourceLanguage::Automatic)
            .chain(
                source_languages(audio3::LANGUAGE_CODES)
                    .into_iter()
                    .filter(|source| sources.contains(&source.raw_value())),
            )
            .collect::<Vec<_>>();
        assert_eq!(translated.source_languages, expected_sources);
        assert!(translated
            .source_languages
            .contains(&SourceLanguage::French));
        assert!(translated
            .target_languages
            .contains(&TargetLanguage::TraditionalChinese));
        assert!(!translated
            .source_languages
            .contains(&SourceLanguage::Cantonese));
    }
    let tencent = ProviderKind::TencentCloud.capabilities();
    assert!(!tencent.source_languages.contains(&SourceLanguage::French));
    assert!(!tencent.target_languages.contains(&TargetLanguage::French));
    assert!(ProviderKind::BaiduTranslate
        .capabilities()
        .source_languages
        .contains(&SourceLanguage::French));
    assert!(ProviderKind::AzureOpenAIRealtime
        .capabilities()
        .target_languages
        .contains(&TargetLanguage::French));
}

#[test]
fn chatmock_has_its_own_wire_identity_without_changing_generic_profiles() {
    for provider in [
        ProviderKind::AlibabaCloud,
        ProviderKind::DeepLX,
        ProviderKind::CustomDashScopeASR,
        ProviderKind::CustomOpenAIASR,
    ] {
        let mut generic = ServiceProfile::new("synthetic", "Synthetic", provider).unwrap();
        generic.text_translation = Some(TextTranslation::OpenAICompatible);
        let mut chatmock = generic.clone();
        chatmock.text_translation = Some(TextTranslation::ChatMock);
        assert_eq!(
            serde_json::to_value(&generic).unwrap()["textTranslation"],
            "openAICompatible"
        );
        assert_eq!(
            serde_json::to_value(&chatmock).unwrap()["textTranslation"],
            "chatMock"
        );
        let decoded: ServiceProfile =
            serde_json::from_value(serde_json::to_value(&chatmock).unwrap()).unwrap();
        assert_eq!(decoded.validated().unwrap(), chatmock);
        assert_eq!(chatmock.effective_provider(), generic.effective_provider());
        assert_eq!(
            chatmock.capabilities(TargetLanguage::SimplifiedChinese),
            generic.capabilities(TargetLanguage::SimplifiedChinese)
        );
    }
    let mut unsupported =
        ServiceProfile::new("other", "Other", ProviderKind::VolcanoEngine).unwrap();
    unsupported.text_translation = Some(TextTranslation::ChatMock);
    assert!(unsupported.validated().is_err());
}

#[test]
fn openai_compatible_route_stays_on_alibaba_and_encodes_the_represented_catalog() {
    for provider in [ProviderKind::AlibabaCloud, ProviderKind::DeepLX] {
        let mut profile = ServiceProfile::new("custom", "Custom translation", provider).unwrap();
        profile.text_translation = Some(TextTranslation::OpenAICompatible);
        assert_eq!(
            profile.validated().unwrap().effective_provider(),
            ProviderKind::AlibabaCloud
        );
        let body = serde_json::to_value(&profile).unwrap();
        assert_eq!(body["textTranslation"], "openAICompatible");
        assert_eq!(
            serde_json::from_value::<ServiceProfile>(body).unwrap(),
            profile
        );
        let caps = profile.capabilities(TargetLanguage::Original);
        assert_eq!(
            caps.source_languages,
            [
                vec![SourceLanguage::Automatic],
                source_languages(audio3::LANGUAGE_CODES)
            ]
            .concat()
        );
        assert_eq!(caps.target_languages, TargetLanguage::ALL);
        assert_eq!(caps.translation_modes, vec![TranslationMode::Turbo]);
    }
    let mut unsupported =
        ServiceProfile::new("volcano", "Volcano", ProviderKind::VolcanoEngine).unwrap();
    unsupported.text_translation = Some(TextTranslation::OpenAICompatible);
    assert_eq!(
        unsupported.validated(),
        Err(ServiceProfileError::UnsupportedTextTranslation)
    );
}

#[test]
fn custom_speech_declarations_are_optional_canonical_and_not_discovery() {
    let legacy: ServiceProfile = serde_json::from_value(json!({
        "id": "custom", "name": "Synthetic", "provider": "customDashScopeASR"
    }))
    .unwrap();
    assert_eq!(legacy.custom_speech_source_languages, None);
    assert_eq!(
        legacy
            .capabilities(TargetLanguage::Original)
            .source_languages,
        SourceLanguage::ALL
    );
    for provider in [
        ProviderKind::CustomDashScopeASR,
        ProviderKind::CustomOpenAIASR,
    ] {
        let mut profile = ServiceProfile::new("custom", "Synthetic", provider).unwrap();
        profile
            .set_custom_speech_source_languages(Some(vec![
                SourceLanguage::German,
                SourceLanguage::English,
                SourceLanguage::German,
            ]))
            .unwrap();
        assert_eq!(
            profile.custom_speech_source_languages,
            Some(vec![SourceLanguage::English, SourceLanguage::German])
        );
        assert_eq!(
            profile
                .capabilities(TargetLanguage::Original)
                .source_languages,
            vec![
                SourceLanguage::Automatic,
                SourceLanguage::English,
                SourceLanguage::German
            ]
        );
        let restored: ServiceProfile =
            serde_json::from_value(serde_json::to_value(&profile).unwrap()).unwrap();
        assert_eq!(restored.validated().unwrap(), profile);
        profile
            .set_custom_speech_source_languages(Some(vec![]))
            .unwrap();
        assert_eq!(
            profile
                .capabilities(TargetLanguage::Original)
                .source_languages,
            vec![SourceLanguage::Automatic]
        );
        profile.set_custom_speech_source_languages(None).unwrap();
        assert_eq!(
            profile
                .capabilities(TargetLanguage::Original)
                .source_languages,
            SourceLanguage::ALL
        );
    }
}

#[test]
fn custom_declarations_intersect_only_the_actual_text_route_constraints() {
    let mut profile =
        ServiceProfile::new("custom", "Synthetic", ProviderKind::CustomDashScopeASR).unwrap();
    profile
        .set_custom_speech_source_languages(Some(vec![
            SourceLanguage::French,
            SourceLanguage::German,
            SourceLanguage::Asturian,
        ]))
        .unwrap();
    for route in [TextTranslation::OpenAICompatible, TextTranslation::ChatMock] {
        profile.text_translation = Some(route);
        let prefs = ProviderPreferences {
            source_language: SourceLanguage::German,
            target_language: TargetLanguage::French,
            translation_mode: TranslationMode::Turbo,
        };
        assert_eq!(profile.normalize_preferences(prefs), prefs);
        assert_eq!(
            profile
                .capabilities(TargetLanguage::French)
                .source_languages,
            vec![
                SourceLanguage::Automatic,
                SourceLanguage::French,
                SourceLanguage::German,
                SourceLanguage::Asturian
            ]
        );
        assert_eq!(
            profile
                .capabilities(TargetLanguage::French)
                .target_languages,
            TargetLanguage::ALL
        );
    }
    for route in [TextTranslation::DeepL, TextTranslation::DeepLX] {
        profile.text_translation = Some(route);
        assert_eq!(
            profile
                .capabilities(TargetLanguage::English)
                .source_languages,
            vec![
                SourceLanguage::Automatic,
                SourceLanguage::German,
                SourceLanguage::French
            ]
        );
        assert_eq!(
            profile
                .capabilities(TargetLanguage::Original)
                .source_languages,
            vec![
                SourceLanguage::Automatic,
                SourceLanguage::French,
                SourceLanguage::German,
                SourceLanguage::Asturian
            ]
        );
    }
}

#[test]
fn custom_language_declaration_validation_rejects_builtins_auto_and_unbounded_lists() {
    let mut builtin = ServiceProfile::alibaba_default();
    assert_eq!(
        builtin.set_custom_speech_source_languages(None),
        Err(ServiceProfileError::InvalidCustomSpeechLanguages)
    );
    builtin.custom_speech_source_languages = Some(vec![SourceLanguage::English]);
    assert_eq!(
        builtin.validated(),
        Err(ServiceProfileError::InvalidCustomSpeechLanguages)
    );
    let mut custom =
        ServiceProfile::new("custom", "Synthetic", ProviderKind::CustomDashScopeASR).unwrap();
    for languages in [
        vec![SourceLanguage::Automatic],
        vec![SourceLanguage::English; SourceLanguage::ALL.len()],
    ] {
        assert_eq!(
            custom.set_custom_speech_source_languages(Some(languages)),
            Err(ServiceProfileError::InvalidCustomSpeechLanguages)
        );
        assert_eq!(custom.custom_speech_source_languages, None);
    }
    let all_explicit = SourceLanguage::ALL
        .into_iter()
        .filter(|language| *language != SourceLanguage::Automatic)
        .collect::<Vec<_>>();
    custom
        .set_custom_speech_source_languages(Some(all_explicit.clone()))
        .unwrap();
    assert_eq!(custom.custom_speech_source_languages, Some(all_explicit));
    assert!(serde_json::from_value::<CustomSpeechLanguagesPatch>(
        json!({"languages": ["not-a-language"]})
    )
    .is_err());
    assert!(serde_json::from_value::<CustomSpeechLanguagesPatch>(json!({})).is_err());
    assert!(serde_json::from_value::<CustomSpeechLanguagesPatch>(
        json!({"languages": null, "unexpected": true})
    )
    .is_err());
    assert_eq!(
        serde_json::from_value::<CustomSpeechLanguagesPatch>(json!({"languages": null}))
            .unwrap()
            .languages,
        None
    );
    assert_eq!(
        serde_json::from_value::<CustomSpeechLanguagesPatch>(json!({"languages": []}))
            .unwrap()
            .languages,
        Some(vec![])
    );
}

#[test]
fn historical_profiles_and_supported_text_routes_round_trip() {
    let legacy: ServiceProfile =
        serde_json::from_str(r#"{"id":"old","name":"Custom","provider":"deepLX"}"#).unwrap();
    assert_eq!(legacy.effective_provider(), ProviderKind::DeepLX);
    assert_eq!(legacy.validated().unwrap(), legacy);
    let mut ordinary = ServiceProfile::alibaba_default();
    assert_eq!(ordinary.text_translation(), TextTranslation::FollowService);
    ordinary.text_translation = Some(TextTranslation::DeepLX);
    let json = serde_json::to_string(&ordinary).unwrap();
    assert!(json.contains(r#""textTranslation":"deepLX""#));
    assert_eq!(
        serde_json::from_str::<ServiceProfile>(&json)
            .unwrap()
            .validated()
            .unwrap()
            .effective_provider(),
        ProviderKind::DeepLX
    );
    let mut unsupported =
        ServiceProfile::new("other", "Other", ProviderKind::VolcanoEngine).unwrap();
    unsupported.text_translation = Some(TextTranslation::DeepLX);
    assert_eq!(
        unsupported.validated().unwrap_err(),
        ServiceProfileError::UnsupportedTextTranslation
    );
}

#[test]
fn provider_wire_values_are_stable() {
    let cases = [
        (ProviderKind::AlibabaCloud, "alibabaCloud"),
        (ProviderKind::OpenAIRealtime, "openAIRealtime"),
        (ProviderKind::GoogleGeminiLive, "googleGeminiLive"),
        (ProviderKind::AzureOpenAIRealtime, "azureOpenAIRealtime"),
        (ProviderKind::VolcanoEngine, "volcanoEngine"),
        (ProviderKind::TencentCloud, "tencentCloud"),
        (ProviderKind::BaiduTranslate, "baiduTranslate"),
        (ProviderKind::XAIRealtime, "xAIRealtime"),
        (ProviderKind::CustomDashScopeASR, "customDashScopeASR"),
        (ProviderKind::CustomOpenAIASR, "customOpenAIASR"),
    ];
    for (provider, wire_value) in cases {
        assert_eq!(serde_json::to_value(provider).unwrap(), json!(wire_value));
        assert_eq!(
            serde_json::from_value::<ProviderKind>(json!(wire_value)).unwrap(),
            provider
        );
        assert_eq!(provider.wire_value(), wire_value);
    }
}

#[test]
fn custom_speech_keeps_protocol_identity_and_requires_an_independent_translation_route() {
    for (provider, sample_rate) in [
        (ProviderKind::CustomDashScopeASR, 16_000),
        (ProviderKind::CustomOpenAIASR, 24_000),
    ] {
        let mut profile = ServiceProfile::new("custom", "Custom speech", provider).unwrap();
        assert_eq!(profile.validated().unwrap().effective_provider(), provider);
        assert_eq!(
            profile
                .capabilities(TargetLanguage::English)
                .target_languages,
            vec![TargetLanguage::Original]
        );
        for translation in [
            TextTranslation::DeepL,
            TextTranslation::DeepLX,
            TextTranslation::OpenAICompatible,
            TextTranslation::ChatMock,
        ] {
            profile.text_translation = Some(translation);
            assert_eq!(profile.validated().unwrap().effective_provider(), provider);
            let caps = profile.capabilities(TargetLanguage::English);
            assert_eq!(caps.input_sample_rate_hz, sample_rate);
            if translation.uses_chat_completions() {
                assert_eq!(caps.source_languages, SourceLanguage::ALL);
                assert_eq!(caps.target_languages, TargetLanguage::ALL);
                continue;
            }
            let (sources, targets) = independent_catalog(translation);
            assert_eq!(
                caps.source_languages,
                [vec![SourceLanguage::Automatic], source_languages(sources)].concat()
            );
            assert_eq!(
                caps.target_languages,
                [vec![TargetLanguage::Original], target_languages(targets)].concat()
            );
            assert!(caps.source_languages.contains(&SourceLanguage::French));
            assert!(!caps.source_languages.contains(&SourceLanguage::Asturian));
        }
        profile.text_translation = Some(TextTranslation::FollowService);
        let normalized = profile.normalize_preferences(ProviderPreferences {
            source_language: SourceLanguage::French,
            target_language: TargetLanguage::English,
            translation_mode: TranslationMode::Turbo,
        });
        assert_eq!(normalized.target_language, TargetLanguage::Original);
        assert_eq!(normalized.source_language, SourceLanguage::French);
    }
}

#[test]
fn provider_capabilities_match_transport_constraints() {
    let alibaba = ProviderKind::AlibabaCloud.capabilities();
    assert_eq!(alibaba.source_languages.len(), 25);
    assert_eq!(alibaba.target_languages.len(), 32);
    assert_eq!(alibaba.translation_modes, vec![TranslationMode::Turbo]);
    assert_eq!(alibaba.input_sample_rate_hz, 16_000);
    for provider in [
        ProviderKind::OpenAIRealtime,
        ProviderKind::AzureOpenAIRealtime,
    ] {
        let caps = provider.capabilities();
        assert_eq!(caps.source_languages, vec![SourceLanguage::Automatic]);
        assert_eq!(
            caps.target_languages,
            target_languages(openai_realtime::TRANSLATION_LANGUAGE_CODES)
        );
        assert_eq!(caps.target_languages.len(), 13);
        assert_eq!(caps.input_sample_rate_hz, 24_000);
    }
    let gemini = ProviderKind::GoogleGeminiLive.capabilities();
    assert_eq!(
        gemini.target_languages,
        target_languages(gemini_live::LANGUAGE_CODES)
    );
    assert_eq!(gemini.target_languages.len(), 78);
    assert_eq!(gemini.source_languages, vec![SourceLanguage::Automatic]);
    assert_eq!(gemini.input_sample_rate_hz, 16_000);
    let xai = ProviderKind::XAIRealtime.capabilities();
    assert_eq!(
        xai.source_languages,
        [
            vec![SourceLanguage::Automatic],
            source_languages(xai_realtime::LANGUAGE_CODES)
        ]
        .concat()
    );
    assert_eq!(
        xai.target_languages,
        target_languages(xai_realtime::LANGUAGE_CODES)
    );
    assert_eq!(xai.target_languages.len(), 20);
    let volcano = ProviderKind::VolcanoEngine.capabilities();
    assert_eq!(
        volcano.source_languages,
        source_languages(crate::core::protocols::realtime::volcano_engine::SOURCE_LANGUAGE_CODES)
    );
    assert_eq!(
        volcano.target_languages,
        target_languages(crate::core::protocols::realtime::volcano_engine::TARGET_LANGUAGE_CODES)
    );
    assert_eq!(volcano.source_languages.len(), 23);
    assert_eq!(volcano.target_languages.len(), 21);
    let tencent = ProviderKind::TencentCloud.capabilities();
    assert_eq!(
        tencent.source_languages,
        source_languages(&["zh", "en", "zh_en", "ja", "ko", "yue", "id", "th", "ru"])
    );
    assert_eq!(
        tencent.target_languages,
        target_languages(&["zh", "en", "ja", "ko", "yue", "id", "th", "ru", "zh_en"])
    );
    let baidu = ProviderKind::BaiduTranslate.capabilities();
    let baidu_codes = baidu_translate::LANGUAGE_CODES
        .iter()
        .map(|(code, _)| *code)
        .collect::<Vec<_>>();
    assert_eq!(baidu.source_languages, source_languages(&baidu_codes));
    assert_eq!(baidu.target_languages, target_languages(&baidu_codes));
    assert_eq!(baidu.source_languages.len(), 45);
    for provider in [
        ProviderKind::GoogleGeminiLive,
        ProviderKind::AzureOpenAIRealtime,
        ProviderKind::VolcanoEngine,
        ProviderKind::TencentCloud,
        ProviderKind::BaiduTranslate,
        ProviderKind::XAIRealtime,
    ] {
        assert_eq!(
            provider.capabilities().translation_modes,
            vec![TranslationMode::Turbo]
        );
    }
}

#[test]
fn openai_normalization_uses_supported_fallbacks() {
    let normalized = ProviderKind::OpenAIRealtime
        .capabilities()
        .normalize(ProviderPreferences {
            source_language: SourceLanguage::Japanese,
            target_language: TargetLanguage::Original,
            translation_mode: TranslationMode::HighQuality,
        });
    assert_eq!(
        normalized,
        ProviderPreferences {
            source_language: SourceLanguage::Automatic,
            target_language: TargetLanguage::SimplifiedChinese,
            translation_mode: TranslationMode::Turbo,
        }
    );
}

#[test]
fn explicit_source_providers_do_not_fallback_to_the_target_language() {
    let normalized = ProviderKind::VolcanoEngine
        .capabilities()
        .normalize(ProviderPreferences {
            source_language: SourceLanguage::Automatic,
            target_language: TargetLanguage::Japanese,
            translation_mode: TranslationMode::HighQuality,
        });
    assert_eq!(normalized.source_language, SourceLanguage::Chinese);
    assert_eq!(normalized.target_language, TargetLanguage::Japanese);
    assert_eq!(normalized.translation_mode, TranslationMode::Turbo);
}

#[test]
fn explicit_source_providers_keep_chinese_translation_enabled() {
    for (provider, same_language_target) in [
        (
            ProviderKind::VolcanoEngine,
            TargetLanguage::SimplifiedChinese,
        ),
        (
            ProviderKind::TencentCloud,
            TargetLanguage::SimplifiedChinese,
        ),
        (ProviderKind::BaiduTranslate, TargetLanguage::English),
        (ProviderKind::DeepLX, TargetLanguage::SimplifiedChinese),
    ] {
        let capabilities = provider.capabilities();
        assert_eq!(
            capabilities.target_language_after_source_switch(
                SourceLanguage::Chinese,
                TargetLanguage::English
            ),
            TargetLanguage::English
        );
        assert_eq!(
            capabilities.target_language_after_source_switch(
                SourceLanguage::Chinese,
                TargetLanguage::SimplifiedChinese
            ),
            same_language_target
        );
    }
}

#[test]
fn tencent_normalization_preserves_every_legal_direction_and_rejects_other_directions() {
    let capabilities = ProviderKind::TencentCloud.capabilities();
    for source in &capabilities.source_languages {
        let expected = target_languages(tencent_cloud::supported_target_codes(source.raw_value()));
        assert_eq!(
            capabilities.clone().for_source(*source).target_languages,
            expected
        );
        for target in &capabilities.target_languages {
            let normalized = capabilities.normalize(ProviderPreferences {
                source_language: *source,
                target_language: *target,
                translation_mode: TranslationMode::Turbo,
            });
            assert_eq!(normalized.source_language, *source);
            assert!(expected.contains(&normalized.target_language));
            assert_eq!(
                normalized.target_language,
                if expected.contains(target) {
                    *target
                } else {
                    expected[0]
                }
            );
        }
    }
    assert_eq!(
        capabilities
            .target_language_after_source_switch(SourceLanguage::Russian, TargetLanguage::Japanese),
        TargetLanguage::SimplifiedChinese
    );
    assert_eq!(
        capabilities.target_language_after_source_switch(
            SourceLanguage::ChineseEnglishMixed,
            TargetLanguage::ChineseEnglishMixed
        ),
        TargetLanguage::ChineseEnglishMixed
    );
}

#[test]
fn volcano_normalization_preserves_legal_pairs_and_uses_chinese_or_english_for_other_sources() {
    let capabilities = ProviderKind::VolcanoEngine.capabilities();
    for source in &capabilities.source_languages {
        let expected = target_languages(
            crate::core::protocols::realtime::volcano_engine::supported_target_codes(
                source.raw_value(),
            ),
        );
        assert_eq!(
            capabilities.clone().for_source(*source).target_languages,
            expected
        );
        for target in &capabilities.target_languages {
            let normalized = capabilities.normalize(ProviderPreferences {
                source_language: *source,
                target_language: *target,
                translation_mode: TranslationMode::Turbo,
            });
            assert_eq!(normalized.source_language, *source);
            assert_eq!(
                normalized.target_language,
                if expected.contains(target) {
                    *target
                } else {
                    expected[0]
                }
            );
        }
    }
    assert_eq!(
        capabilities
            .target_language_after_source_switch(SourceLanguage::French, TargetLanguage::Japanese),
        TargetLanguage::SimplifiedChinese
    );
    assert_eq!(
        capabilities.target_language_after_source_switch(
            SourceLanguage::ChineseEnglishMixed,
            TargetLanguage::English
        ),
        TargetLanguage::ChineseEnglishMixed
    );
}

#[test]
fn alibaba_source_selection_preserves_an_explicit_translation_target() {
    let profile = ServiceProfile::alibaba_default();
    let capabilities = profile.capabilities(TargetLanguage::English);
    assert_eq!(
        capabilities
            .target_language_after_source_switch(SourceLanguage::Chinese, TargetLanguage::English),
        TargetLanguage::English
    );
}

#[test]
fn alibaba_source_selection_keeps_original_for_audio3_only_languages() {
    let profile = ServiceProfile::alibaba_default();
    let capabilities = profile.capabilities(TargetLanguage::Original);
    for source in [
        SourceLanguage::Norwegian,
        SourceLanguage::Romanian,
        SourceLanguage::Greek,
        SourceLanguage::Bulgarian,
        SourceLanguage::Croatian,
        SourceLanguage::Slovak,
    ] {
        let target =
            capabilities.target_language_after_source_switch(source, TargetLanguage::Original);
        assert_eq!(target, TargetLanguage::Original);
        let configuration =
            crate::core::services::configuration::LiveTranslationConfiguration::for_provider(
                profile.provider,
                "synthetic",
                source,
                target,
                TranslationMode::Turbo,
            );
        let validated = configuration.validated().unwrap();
        assert_eq!(validated.source_language, source);
        assert_eq!(validated.target_language, TargetLanguage::Original);
    }
}

#[test]
fn profiles_are_non_secret_and_trim_names() {
    let profile =
        ServiceProfile::new("openai-1", "  OpenAI Work  ", ProviderKind::OpenAIRealtime).unwrap();
    assert_eq!(profile.name, "OpenAI Work");
    let json = serde_json::to_value(&profile).unwrap();
    assert_eq!(json["provider"], "openAIRealtime");
    assert!(json.get("apiKey").is_none());
    assert!(json.get("credential").is_none());
}

#[test]
fn speech_recognition_names_are_optional_trimmed_custom_metadata() {
    for provider in [
        ProviderKind::CustomDashScopeASR,
        ProviderKind::CustomOpenAIASR,
    ] {
        let mut profile: ServiceProfile = serde_json::from_value(serde_json::json!({
            "id": "legacy", "name": "Legacy", "provider": provider
        }))
        .unwrap();
        assert!(profile.speech_recognition_name.is_none());
        profile
            .set_speech_recognition_name("  Whisper · 本地  ")
            .unwrap();
        let restored = profile.validated().unwrap();
        assert_eq!(restored, profile);
        assert_eq!(restored.name, "Legacy");
        assert_eq!(
            serde_json::to_value(&restored).unwrap()["speechRecognitionName"],
            "Whisper · 本地"
        );
        profile.set_speech_recognition_name("  ").unwrap();
        assert!(profile.speech_recognition_name.is_none());
        assert!(serde_json::to_value(&profile)
            .unwrap()
            .get("speechRecognitionName")
            .is_none());
    }
}

#[test]
fn speech_recognition_names_reject_built_in_providers_and_invalid_names() {
    let mut profile =
        ServiceProfile::new("custom", "Custom", ProviderKind::CustomOpenAIASR).unwrap();
    profile
        .set_speech_recognition_name(&"😀".repeat(64))
        .unwrap();
    let saved = profile.clone();
    for name in [
        "😀".repeat(65),
        "line\nbreak".into(),
        "control\u{0000}".into(),
    ] {
        assert_eq!(
            profile.set_speech_recognition_name(&name),
            Err(ServiceProfileError::InvalidSpeechRecognitionName)
        );
        assert_eq!(profile, saved);
    }
    profile.provider = ProviderKind::AlibabaCloud;
    assert_eq!(
        profile.set_speech_recognition_name("Alias"),
        Err(ServiceProfileError::UnsupportedSpeechRecognitionName)
    );
    assert_eq!(
        profile.validated(),
        Err(ServiceProfileError::UnsupportedSpeechRecognitionName)
    );
}

#[test]
fn text_translation_names_are_optional_trimmed_route_metadata() {
    let mut profile: ServiceProfile = serde_json::from_value(serde_json::json!({
        "id": "legacy", "name": "Legacy", "provider": "alibabaCloud"
    }))
    .unwrap();
    assert!(profile.text_translation_names.is_empty());
    profile
        .set_text_translation_name(TextTranslation::OpenAICompatible, "  My translator  ")
        .unwrap();
    profile
        .set_text_translation_name(TextTranslation::DeepLX, "Local translator")
        .unwrap();
    let restored = profile.validated().unwrap();
    assert_eq!(restored, profile);
    let json = serde_json::to_value(&restored).unwrap();
    assert_eq!(
        json["textTranslationNames"]["openAICompatible"],
        "My translator"
    );
    assert_eq!(json["textTranslationNames"]["deepLX"], "Local translator");
    profile
        .set_text_translation_name(TextTranslation::OpenAICompatible, "  ")
        .unwrap();
    assert!(!profile
        .text_translation_names
        .contains_key(&TextTranslation::OpenAICompatible));
    assert_eq!(profile.text_translation_names.len(), 1);
}

#[test]
fn text_translation_names_reject_unsupported_routes_and_invalid_names() {
    let mut profile = ServiceProfile::alibaba_default();
    assert_eq!(
        profile.set_text_translation_name(TextTranslation::FollowService, "Alias"),
        Err(ServiceProfileError::UnsupportedTextTranslation)
    );
    for name in [
        "x".repeat(65),
        "line\nbreak".into(),
        "control\u{0000}".into(),
    ] {
        assert_eq!(
            profile.set_text_translation_name(TextTranslation::OpenAICompatible, &name),
            Err(ServiceProfileError::InvalidTextTranslationName)
        );
    }
    profile
        .set_text_translation_name(TextTranslation::OpenAICompatible, &"名".repeat(64))
        .unwrap();
    profile.provider = ProviderKind::VolcanoEngine;
    assert_eq!(
        profile.validated(),
        Err(ServiceProfileError::UnsupportedTextTranslation)
    );
}

#[test]
fn default_profile_preserves_alibaba_compatibility() {
    let profile = ServiceProfile::default();
    assert_eq!(profile.id, DEFAULT_ALIBABA_PROFILE_ID);
    assert_eq!(profile.provider, ProviderKind::AlibabaCloud);
}

mod selected_qwen_model_tests {
    use super::*;
    use crate::core::protocols::translation::qwen_mt::{QwenMTModel, QwenMTRequestEncoder};

    #[test]
    fn selected_model_catalog_matches_encoder_and_shared_ui_subset() {
        let catalogs: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../provider-language-catalogs.json"
        )))
        .unwrap();
        for model in [QwenMTModel::Lite, QwenMTModel::Flash, QwenMTModel::Plus] {
            let mut profile =
                ServiceProfile::new("fixture", "Fixture", ProviderKind::AlibabaCloud).unwrap();
            profile.qwen_mt_model = model;
            let caps = profile.capabilities(TargetLanguage::English);
            assert_eq!(
                caps.target_languages.len(),
                if model == QwenMTModel::Lite { 32 } else { 81 }
            );
            assert_eq!(
                caps.source_languages.contains(&SourceLanguage::Norwegian),
                model != QwenMTModel::Lite
            );
            if model != QwenMTModel::Lite {
                let codes: Vec<_> = caps
                    .target_languages
                    .iter()
                    .filter(|target| **target != TargetLanguage::Original)
                    .map(|target| target.raw_value())
                    .collect();
                assert_eq!(
                    serde_json::to_value(codes).unwrap(),
                    catalogs["qwenMtFlash"]["targetLanguages"]
                );
            }
            for source in &caps.source_languages {
                for target in &caps.target_languages {
                    if *target == TargetLanguage::Original {
                        continue;
                    }
                    let request = QwenMTRequestEncoder::request(
                        "Synthetic fixture.",
                        *source,
                        *target,
                        model,
                        false,
                        None,
                        &[],
                        &[],
                    )
                    .unwrap();
                    if *target == TargetLanguage::Norwegian {
                        assert_eq!(
                            request["translation_options"]["target_lang"],
                            "Norwegian Bokmål"
                        );
                    }
                }
            }
        }
    }
}
