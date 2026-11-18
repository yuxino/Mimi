//! Settings command behavior and capability regressions.

use super::super::tests::{BootstrapCredentialStore, PartiallyUnavailableSecretStore};
use super::*;
use crate::apple_speech::AppleSpeechResourceStatus;
use crate::settings_store::CredentialState;
use std::sync::Arc;

#[tokio::test]
async fn initial_settings_load_preserves_empty_and_unavailable_credential_catalogs() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("service-profiles.json"),
        r#"{"schemaVersion":1,"activeProfileId":"","profiles":[]}"#,
    )
    .unwrap();
    let empty = SettingsStore::at_path(
        directory.path().into(),
        Box::new(PartiallyUnavailableSecretStore),
    );
    let snapshot = get_settings_snapshot(&empty, true, |_| {
        panic!("Empty capabilities must not publish")
    })
    .await
    .unwrap();
    assert!(snapshot.profiles.is_empty());
    assert!(snapshot.active_profile_id.is_empty());

    let unavailable = SettingsStore::in_memory(Box::new(PartiallyUnavailableSecretStore), false);
    let snapshot = get_settings_snapshot(&unavailable, true, |_| {
        panic!("Equivalent capabilities must not publish")
    })
    .await
    .unwrap();
    assert_eq!(snapshot.profiles.len(), 1);
    assert_eq!(
        snapshot.profiles[0].credential_state,
        CredentialState::Unavailable
    );
    assert!(snapshot.language_capabilities.is_some());
}

#[tokio::test]
async fn initial_settings_bootstrap_reads_each_credential_slot_once() {
    let directory = tempfile::tempdir().unwrap();
    let alibaba = ServiceProfile::alibaba_default();
    let mut custom = ServiceProfile::new(
        "bootstrap-custom",
        "Synthetic speech",
        ProviderKind::CustomOpenAIASR,
    )
    .unwrap();
    custom.text_translation = Some(TextTranslation::DeepL);
    std::fs::write(
        directory.path().join("service-profiles.json"),
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "activeProfileId": alibaba.id,
            "profiles": [&alibaba, &custom],
        }))
        .unwrap(),
    )
    .unwrap();
    let reads = Arc::new(std::sync::Mutex::new(Vec::new()));
    let store = SettingsStore::at_path(
        directory.path().into(),
        Box::new(BootstrapCredentialStore {
            reads: Arc::clone(&reads),
        }),
    );

    let snapshot = get_settings_snapshot(&store, false, |_| {
        panic!("Equivalent capabilities must not publish a settings change")
    })
    .await
    .unwrap();

    assert_eq!(snapshot.active_profile_id, alibaba.id);
    assert_eq!(snapshot.profiles.len(), 2);
    assert_eq!(
        snapshot.profiles[0].credential_state,
        CredentialState::Present
    );
    assert_eq!(snapshot.profiles[1].id, custom.id);
    assert_eq!(
        snapshot.profiles[1].text_translation,
        TextTranslation::DeepL
    );
    assert_eq!(
        snapshot.profiles[1].speech_credential_state,
        Some(CredentialState::Missing)
    );
    assert_eq!(
        snapshot.profiles[1].text_credential_state,
        Some(CredentialState::Unavailable)
    );
    assert_eq!(
        snapshot.profiles[1].credential_state,
        CredentialState::Unavailable
    );
    let mut accounts = reads.lock().unwrap().clone();
    accounts.sort_unstable();
    assert_eq!(
        accounts,
        [
            "provider-profile:alibaba-default:alibabaCloud:api-key",
            "provider-profile:bootstrap-custom:customOpenAIASR:api-key",
            "provider-profile:bootstrap-custom:customOpenAIASR:text-translation",
        ]
    );
}

#[tokio::test]
async fn initial_settings_load_rejects_an_unreadable_catalog_without_empty_success() {
    for content in [
        "{invalid catalog",
        r#"{"schemaVersion":999,"activeProfileId":"","profiles":[]}"#,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("service-profiles.json");
        std::fs::write(&path, content).unwrap();
        let store = SettingsStore::at_path(
            directory.path().into(),
            Box::new(PartiallyUnavailableSecretStore),
        );
        let result =
            get_settings_snapshot(&store, true, |_| panic!("Failed reads must not publish")).await;
        assert_eq!(
            result.unwrap_err(),
            "Service profile settings are unavailable."
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), content);
    }
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join("service-profiles.json")).unwrap();
    let store = SettingsStore::at_path(
        directory.path().into(),
        Box::new(PartiallyUnavailableSecretStore),
    );
    assert_eq!(
        get_settings_snapshot(&store, true, |_| panic!("Failed reads must not publish"))
            .await
            .unwrap_err(),
        "Service profile settings are unavailable."
    );
}

fn apple_bootstrap_snapshot(sources: &[SourceLanguage], revision: u64) -> SettingsSnapshotPayload {
    let store = SettingsStore::in_memory(Box::new(PartiallyUnavailableSecretStore), true);
    let profile = store
        .create_profile(ProviderKind::AppleSpeech, "Synthetic Apple profile")
        .unwrap();
    store.select_profile(&profile.id).unwrap();
    let mut snapshot = SettingsSnapshotPayload::try_from_store(&store).unwrap();
    let capabilities = snapshot.language_capabilities.as_mut().unwrap();
    capabilities.source_languages = sources.to_vec();
    capabilities.apple_speech_support_revision = Some(revision);
    snapshot
}

#[tokio::test]
async fn initial_settings_support_recovery_publishes_the_exact_returned_snapshot() {
    let mut recovered = apple_bootstrap_snapshot(&[SourceLanguage::English], 1);
    let mut previous = recovered.language_capabilities.clone();
    previous.as_mut().unwrap().source_languages.clear();
    previous.as_mut().unwrap().apple_speech_support_revision = Some(0);
    recovered.font_size = 21.0;
    let expected = serde_json::to_value(&recovered).unwrap();
    let published = std::cell::RefCell::new(Vec::new());
    let loaded = std::cell::Cell::new(false);
    let returned = load_and_publish_settings_snapshot(
        previous,
        async {
            loaded.set(true);
            Ok(recovered)
        },
        |payload| {
            assert!(loaded.get());
            published
                .borrow_mut()
                .push(serde_json::to_value(payload).unwrap());
        },
    )
    .await
    .unwrap();
    assert_eq!(serde_json::to_value(returned).unwrap(), expected);
    assert_eq!(*published.borrow(), vec![expected]);
}

#[tokio::test]
async fn initial_settings_resource_revision_change_is_published_without_a_language_list_change() {
    let mut recovered = apple_bootstrap_snapshot(&[SourceLanguage::English], 2);
    let mut previous = recovered.language_capabilities.clone();
    previous.as_mut().unwrap().apple_speech_support_revision = Some(1);
    let published = std::cell::Cell::new(0);
    recovered.font_size = 22.0;
    load_and_publish_settings_snapshot(previous, async { Ok(recovered) }, |_| {
        published.set(published.get() + 1);
    })
    .await
    .unwrap();
    assert_eq!(published.get(), 1);
}

#[tokio::test]
async fn initial_settings_equivalent_ready_or_failed_support_does_not_rebroadcast() {
    for sources in [Vec::new(), vec![SourceLanguage::English]] {
        let mut current = apple_bootstrap_snapshot(&sources, 1);
        let previous = current.language_capabilities.clone();
        current.font_size = 23.0;
        let returned = load_and_publish_settings_snapshot(previous, async { Ok(current) }, |_| {
            panic!("Equivalent capabilities must not trigger another resource query")
        })
        .await
        .unwrap();
        assert_eq!(returned.font_size, 23.0);
    }
}

#[tokio::test]
async fn initial_settings_failed_snapshot_does_not_publish_fallback_capabilities() {
    let result = load_and_publish_settings_snapshot(
        apple_bootstrap_snapshot(&[], 0).language_capabilities,
        async { Err("Service profile settings are unavailable.".into()) },
        |_| panic!("A failed catalog read must not publish a fallback snapshot"),
    )
    .await;
    assert_eq!(
        result.unwrap_err(),
        "Service profile settings are unavailable."
    );
}

#[tokio::test]
async fn initial_settings_recovery_reads_the_current_profile_after_the_loader_wait() {
    let store = SettingsStore::in_memory(Box::new(PartiallyUnavailableSecretStore), true);
    let previous = SettingsSnapshotPayload::try_from_store(&store)
        .unwrap()
        .language_capabilities;
    let published = std::cell::RefCell::new(Vec::new());
    let (release, released) = tokio::sync::oneshot::channel();
    let mut loading = Box::pin(load_and_publish_settings_snapshot(
        previous,
        async {
            released.await.unwrap();
            // UI-test loading must never inspect real native support.
            load_settings_snapshot(&store, true).await
        },
        |payload| {
            published
                .borrow_mut()
                .push(serde_json::to_value(payload).unwrap());
        },
    ));
    std::future::poll_fn(|cx| {
        assert!(std::future::Future::poll(loading.as_mut(), cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    assert!(published.borrow().is_empty());
    let profile = store
        .create_profile(
            ProviderKind::AppleSpeech,
            "Synthetic newly selected profile",
        )
        .unwrap();
    store.select_profile(&profile.id).unwrap();
    release.send(()).unwrap();
    let returned = loading.await.unwrap();
    assert_eq!(returned.active_profile_id, profile.id);
    assert_eq!(
        returned.language_capabilities.as_ref().unwrap().profile_id,
        profile.id
    );
    assert_eq!(
        *published.borrow(),
        vec![serde_json::to_value(returned).unwrap()]
    );
}

fn apple_language_support() -> crate::apple_speech_support::AppleSpeechSupport {
    use crate::apple_speech_support::{AppleSpeechLanguage, AppleSpeechSupport};
    AppleSpeechSupport {
        available: true,
        languages: vec![
            AppleSpeechLanguage {
                source_language: SourceLanguage::English,
                locale: "en-US".into(),
                status: AppleSpeechResourceStatus::Installed,
                installed: true,
                downloading: false,
            },
            AppleSpeechLanguage {
                source_language: SourceLanguage::Japanese,
                locale: "ja-JP".into(),
                status: AppleSpeechResourceStatus::Supported,
                installed: false,
                downloading: false,
            },
            AppleSpeechLanguage {
                source_language: SourceLanguage::French,
                locale: "fr-FR".into(),
                status: AppleSpeechResourceStatus::Supported,
                installed: false,
                downloading: false,
            },
        ],
    }
}

#[test]
fn apple_language_snapshot_only_offers_ready_sources_on_available_devices() {
    let profile = ServiceProfile::new("apple", "Apple", ProviderKind::AppleSpeech).unwrap();
    let mut support = apple_language_support();
    let payload = LanguageCapabilitiesPayload::from_profile_with_apple_support(
        &profile,
        TargetLanguage::Original,
        Some((&support, 1)),
    );
    let json = serde_json::to_value(payload).unwrap();
    assert_eq!(json["profileId"], "apple");
    assert_eq!(json["appleSpeechSupportRevision"], 1);
    assert_eq!(json["sourceLanguages"], serde_json::json!(["en"]));
    assert_eq!(support, apple_language_support());

    support.available = false;
    for support in [Some((&support, 1)), None] {
        let payload = LanguageCapabilitiesPayload::from_profile_with_apple_support(
            &profile,
            TargetLanguage::Original,
            support,
        );
        assert!(payload.source_languages.is_empty());
    }
}

#[test]
fn apple_language_snapshot_keeps_ready_sources_within_the_text_route() {
    let mut profile = ServiceProfile::new("apple", "Apple", ProviderKind::AppleSpeech).unwrap();
    let mut support = apple_language_support();
    support.languages[2].set_status(AppleSpeechResourceStatus::Installed);
    for route in [TextTranslation::DeepL, TextTranslation::DeepLX] {
        profile.text_translation = Some(route);
        let translated = LanguageCapabilitiesPayload::from_profile_with_apple_support(
            &profile,
            TargetLanguage::English,
            Some((&support, 1)),
        );
        assert_eq!(
            translated
                .source_languages
                .iter()
                .map(|source| source.raw_value())
                .collect::<std::collections::BTreeSet<_>>(),
            ["en", "fr"].into_iter().collect()
        );
        let original = LanguageCapabilitiesPayload::from_profile_with_apple_support(
            &profile,
            TargetLanguage::Original,
            Some((&support, 1)),
        );
        assert_eq!(
            original.source_languages,
            vec![SourceLanguage::English, SourceLanguage::French]
        );
    }
    profile.text_translation = Some(TextTranslation::OpenAICompatible);
    let translated = LanguageCapabilitiesPayload::from_profile_with_apple_support(
        &profile,
        TargetLanguage::English,
        Some((&support, 1)),
    );
    assert_eq!(
        translated.source_languages,
        vec![SourceLanguage::English, SourceLanguage::French]
    );
}

#[test]
fn apple_language_snapshot_adds_a_prepared_source_without_hiding_downloadable_resources() {
    let profile = ServiceProfile::new("apple", "Apple", ProviderKind::AppleSpeech).unwrap();
    let mut support = apple_language_support();
    let before = LanguageCapabilitiesPayload::from_profile_with_apple_support(
        &profile,
        TargetLanguage::Original,
        Some((&support, 1)),
    );
    assert_eq!(before.source_languages, vec![SourceLanguage::English]);
    support.languages[1].set_status(AppleSpeechResourceStatus::Installed);
    let after = LanguageCapabilitiesPayload::from_profile_with_apple_support(
        &profile,
        TargetLanguage::Original,
        Some((&support, 1)),
    );
    assert_eq!(
        after.source_languages,
        vec![SourceLanguage::English, SourceLanguage::Japanese]
    );
    // The resource preparation response still includes the unprepared
    // French locale; no fallback language is inserted into the picker.
    let resources = serde_json::to_value(&support).unwrap();
    assert_eq!(resources["languages"].as_array().unwrap().len(), 3);
    assert_eq!(resources["languages"][2]["sourceLanguage"], "fr");
    assert_eq!(resources["languages"][2]["installed"], false);
    support
        .languages
        .iter_mut()
        .for_each(|language| language.set_status(AppleSpeechResourceStatus::Supported));
    let empty = LanguageCapabilitiesPayload::from_profile_with_apple_support(
        &profile,
        TargetLanguage::Original,
        Some((&support, 1)),
    );
    assert!(empty.source_languages.is_empty());
}

#[test]
fn language_capability_snapshot_is_stamped_and_tracks_the_atomic_profile_route() {
    let store = SettingsStore::in_memory(Box::new(PartiallyUnavailableSecretStore), false);
    store
        .save_preferences_for_active_profile(|prefs| prefs.target_language = TargetLanguage::French)
        .unwrap();
    let snapshot = SettingsSnapshotPayload::try_from_store(&store).unwrap();
    let value = serde_json::to_value(snapshot).unwrap();
    let caps = &value["languageCapabilities"];
    assert_eq!(caps["profileId"], value["activeProfileId"]);
    assert_eq!(caps["targetLanguage"], value["targetLanguage"]);
    assert_eq!(caps["provider"], "alibabaCloud");
    assert_eq!(caps["qwenMtModel"], "lite");
    assert!(caps.get("appleSpeechSupportRevision").is_none());
    assert_eq!(caps["textTranslation"], "followService");
    assert_eq!(caps["sourceLanguages"].as_array().unwrap().len(), 25);
    assert_eq!(caps["targetLanguages"].as_array().unwrap().len(), 32);
    store
        .save_preferences_for_active_profile(|prefs| {
            prefs.target_language = TargetLanguage::Original;
            prefs.source_language = SourceLanguage::Greek;
        })
        .unwrap();
    let snapshot = SettingsSnapshotPayload::try_from_store(&store).unwrap();
    assert_eq!(
        snapshot
            .language_capabilities
            .unwrap()
            .source_languages
            .len(),
        31
    );
    let mut profile = ServiceProfile::alibaba_default();
    profile.text_translation = Some(TextTranslation::DeepL);
    let deep_l = LanguageCapabilitiesPayload::from_profile(&profile, TargetLanguage::Original);
    assert_eq!(deep_l.text_translation, TextTranslation::DeepL);
    assert_eq!(deep_l.source_languages.len(), 31);
    assert_eq!(
        deep_l.target_languages.len(),
        crate::core::protocols::translation::deepl_languages::DEEPL_TARGET_CODES.len() + 1
    );
    profile.text_translation = Some(TextTranslation::OpenAICompatible);
    let custom = LanguageCapabilitiesPayload::from_profile(&profile, TargetLanguage::Original);
    assert_eq!(
        custom
            .source_languages
            .iter()
            .map(|source| source.raw_value())
            .collect::<Vec<_>>(),
        std::iter::once("auto")
            .chain(
                crate::core::protocols::recognition::audio3::LANGUAGE_CODES
                    .iter()
                    .copied()
            )
            .collect::<Vec<_>>()
    );
    assert_eq!(custom.target_languages, TargetLanguage::ALL);
    let custom = serde_json::to_value(custom).unwrap();
    assert_eq!(custom["provider"], "alibabaCloud");
    assert_eq!(custom["textTranslation"], "openAICompatible");
    for secret_field in ["endpoint", "apiKey", "token", "model"] {
        assert!(custom.get(secret_field).is_none());
    }
}

#[test]
fn immersive_mode_toggle_inverts_the_current_preference() {
    assert!(toggled_immersive_mode(false));
    assert!(!toggled_immersive_mode(true));
}

#[test]
fn dock_preference_is_settings_scoped() {
    for show in [false, true] {
        let draft = SettingsDraft {
            show_in_dock: Some(show),
            ..SettingsDraft::default()
        };
        assert!(ensure_settings_draft_window_allowed("settings", &draft).is_ok());
        for label in ["overlay", "overlay-control", "tray-panel", "unknown"] {
            assert!(ensure_settings_draft_window_allowed(label, &draft).is_err());
        }
    }
    let presentation = SettingsDraft {
        subtitle_display_mode: Some(SubtitleDisplayMode::Bilingual),
        ..SettingsDraft::default()
    };
    assert!(ensure_settings_draft_window_allowed("overlay-control", &presentation).is_ok());
}

#[test]
fn intermediate_subtitle_toggle_is_allowed_during_an_active_session() {
    for enabled in [false, true] {
        let draft: SettingsDraft =
            serde_json::from_value(serde_json::json!({"showIntermediateSubtitles": enabled}))
                .unwrap();
        assert_eq!(draft.show_intermediate_subtitles, Some(enabled));
        assert!(ensure_settings_draft_allowed(&draft, true).is_ok());
        assert!(ensure_settings_draft_window_allowed("settings", &draft).is_ok());
        assert!(ensure_settings_draft_window_allowed("overlay-control", &draft).is_ok());
    }
}

#[test]
fn subtitle_timestamp_toggle_is_allowed_during_an_active_session() {
    for enabled in [false, true] {
        let draft: SettingsDraft =
            serde_json::from_value(serde_json::json!({"showSubtitleTimestamps": enabled})).unwrap();
        assert_eq!(draft.show_subtitle_timestamps, Some(enabled));
        assert!(ensure_settings_draft_allowed(&draft, true).is_ok());
        assert!(ensure_settings_draft_window_allowed("settings", &draft).is_ok());
    }
}

#[test]
fn subtitle_count_preferences_are_allowed_during_an_active_session() {
    for enabled in [false, true] {
        let draft: SettingsDraft =
            serde_json::from_value(serde_json::json!({"customSubtitleCountEnabled": enabled}))
                .unwrap();
        assert_eq!(draft.custom_subtitle_count_enabled, Some(enabled));
        assert!(ensure_settings_draft_allowed(&draft, true).is_ok());
        assert!(ensure_settings_draft_window_allowed("settings", &draft).is_ok());
    }
    for count in [1_u8, 10] {
        let draft: SettingsDraft =
            serde_json::from_value(serde_json::json!({"visibleSubtitleCount": count})).unwrap();
        assert_eq!(draft.visible_subtitle_count, Some(count));
        assert!(ensure_settings_draft_allowed(&draft, true).is_ok());
        assert!(ensure_settings_draft_window_allowed("settings", &draft).is_ok());
    }
    assert!(SettingsDraft::default()
        .custom_subtitle_count_enabled
        .is_none());
    assert!(SettingsDraft::default().visible_subtitle_count.is_none());
}

#[test]
fn subtitle_font_choice_is_optional_and_allowed_during_an_active_session() {
    assert!(SettingsDraft::default().subtitle_font_family.is_none());
    for family in ["", "Noto Sans CJK SC", "思源黑体"] {
        let draft: SettingsDraft =
            serde_json::from_value(serde_json::json!({"subtitleFontFamily": family})).unwrap();
        assert_eq!(draft.subtitle_font_family.as_deref(), Some(family));
        assert!(ensure_settings_draft_allowed(&draft, true).is_ok());
        assert!(ensure_settings_draft_window_allowed("settings", &draft).is_ok());
    }
    assert!(serde_json::from_value::<SettingsDraft>(
        serde_json::json!({"subtitleFontFamily": ["Font", "Fallback"]})
    )
    .is_err());
}

#[test]
fn dock_access_does_not_widen_settings_only_preferences() {
    for field in [
        serde_json::json!({"retainSessionHistory": false}),
        serde_json::json!({"recordSessionAudio": false}),
        serde_json::json!({"windowsAudioSource": ""}),
        serde_json::json!({"audioInput": "microphone"}),
        serde_json::json!({"networkProxy": {"mode": "direct"}}),
    ] {
        for show in [None, Some(false), Some(true)] {
            let mut draft: SettingsDraft = serde_json::from_value(field.clone()).unwrap();
            draft.show_in_dock = show;
            assert!(ensure_settings_draft_window_allowed("settings", &draft).is_ok());
            for label in ["tray-panel", "overlay", "overlay-control", "unknown"] {
                assert!(ensure_settings_draft_window_allowed(label, &draft).is_err());
            }
        }
    }
}

#[test]
fn settings_payload_is_camel_case_and_write_only() {
    let payload = SettingsSnapshotPayload {
        credential_storage: "keychain",
        profiles: vec![ServiceProfilePayload {
            qwen_mt_model: Default::default(),
            alibaba_realtime_mode: Default::default(),
            alibaba_workspace_id: String::new(),
            xai_speech_mode: Default::default(),
            speech_translation_mode: Default::default(),
            xai_text_model: "grok-4.3".into(),
            language_preset: None,
            speech_network_proxy: None,
            text_network_proxy: None,
            custom_speech_source_languages: None,
            id: "alibaba-default".into(),
            name: "Alibaba Cloud".into(),
            provider: ProviderKind::AlibabaCloud,
            speech_recognition_name: None,
            credential_state: CredentialState::Present,
            credential_storage: "keychain",
            speech_credential_state: None,
            text_credential_state: None,
            text_translation: crate::core::services::provider::TextTranslation::FollowService,
            text_translation_names: std::collections::BTreeMap::new(),
        }],
        active_profile_id: "alibaba-default".into(),
        language_capabilities: None,
        source_language: SourceLanguage::Japanese,
        target_language: TargetLanguage::SimplifiedChinese,
        translation_mode: TranslationMode::HighQuality,
        font_size: 18.0,
        subtitle_font_family: "Noto Sans CJK SC".into(),
        subtitle_background_opacity: 80,
        subtitle_color: SubtitleColor::White,
        microphone_subtitle_color: SubtitleColor::Yellow,
        subtitle_alignment: SubtitleAlignment::Center,
        subtitle_display_mode: SubtitleDisplayMode::Translation,
        show_intermediate_subtitles: true,
        show_subtitle_dividers: false,
        show_subtitle_timestamps: false,
        custom_subtitle_count_enabled: false,
        visible_subtitle_count: 3,
        pulse_animation: None,
        pulse_style: PulseStyle::Ribbon,
        subtitle_animation: None,
        subtitle_blends_with_background: false,
        is_overlay_locked: false,
        ui_language: None,
        retain_session_history: false,
        record_session_audio: false,
        audio_input: AudioInput::System,
        microphone_input_available: crate::core::audio::audio_input::MICROPHONE_INPUT_AVAILABLE,
        windows_audio_source: String::new(),
        system_audio_target: Default::default(),
        show_in_dock: false,
        network_proxy: ProxyConfig::default(),
    };
    let json = serde_json::to_value(&payload).unwrap();
    assert_eq!(json["activeProfileId"], "alibaba-default");
    assert_eq!(json["microphoneInputAvailable"], true);
    assert_eq!(json["credentialStorage"], "keychain");
    assert_eq!(json["pulseStyle"], "ribbon");
    assert_eq!(json["showInDock"], false);
    assert_eq!(json["audioInput"], "system");
    assert_eq!(json["networkProxy"]["mode"], "system");
    assert_eq!(json["profiles"][0]["provider"], "alibabaCloud");
    assert_eq!(json["profiles"][0]["credentialState"], "present");
    assert_eq!(json["profiles"][0]["textTranslation"], "followService");
    for secret_field in ["apiKey", "asrApiKey", "endpoint", "token"] {
        assert!(json["profiles"][0].get(secret_field).is_none());
    }
    assert_eq!(json["subtitleAlignment"], "center");
    assert_eq!(json["subtitleFontFamily"], "Noto Sans CJK SC");
    assert_eq!(json["subtitleBackgroundOpacity"], 80);
    assert_eq!(json["subtitleColor"], "white");
    assert_eq!(json["subtitleDisplayMode"], "translation");
    assert_eq!(json["showIntermediateSubtitles"], true);
    assert_eq!(json["showSubtitleDividers"], false);
    assert_eq!(json["showSubtitleTimestamps"], false);
    assert_eq!(json["customSubtitleCountEnabled"], false);
    assert_eq!(json["visibleSubtitleCount"], 3);
    assert_eq!(json["microphoneSubtitleColor"], "yellow");
    assert_eq!(json["subtitleBlendsWithBackground"], false);
    assert!(json.get("apiKey").is_none());
    assert!(json.get("hasAPIKey").is_none());
    assert!(!json.to_string().contains("secret"));
}

#[test]
fn archive_preferences_require_an_inactive_session_for_both_directions() {
    for enabled in [true, false] {
        for draft in [
            SettingsDraft {
                retain_session_history: Some(enabled),
                ..Default::default()
            },
            SettingsDraft {
                record_session_audio: Some(enabled),
                ..Default::default()
            },
        ] {
            assert!(ensure_settings_draft_allowed(&draft, true).is_err());
            assert!(ensure_settings_draft_allowed(&draft, false).is_ok());
        }
    }
}

#[test]
fn audio_source_changes_require_stop_even_when_resetting_to_system() {
    for source in ["wasapi:headphones", ""] {
        let draft = SettingsDraft {
            windows_audio_source: Some(source.into()),
            ..Default::default()
        };
        assert!(ensure_settings_draft_allowed(&draft, true).is_err());
        assert!(ensure_settings_draft_allowed(&draft, false).is_ok());
    }
}

#[test]
fn application_settings_draft_is_settings_only_and_enumeration_reaches_live_controls() {
    use crate::core::audio::system_audio_target::SystemAudioTarget;
    let draft = SettingsDraft {
        system_audio_target: Some(SystemAudioTarget::Application {
            id: "test.player".into(),
            name: "Player".into(),
        }),
        ..Default::default()
    };
    assert!(ensure_settings_draft_window_allowed("settings", &draft).is_ok());
    for label in ["overlay", "overlay-control", "tray-panel"] {
        assert!(ensure_settings_draft_window_allowed(label, &draft).is_err());
    }
    let permissions = include_str!("../../../permissions/app.toml");
    let entries: Vec<_> = permissions
        .split("[[permission]]")
        .filter(|entry| entry.contains("\"audio_applications\""))
        .collect();
    assert_eq!(entries.len(), 2);
    assert!(entries
        .iter()
        .all(|entry| entry.contains("identifier = \"app-settings\"")
            || entry.contains("identifier = \"app-overlay-control\"")));
}

#[test]
fn audio_input_changes_are_settings_only_and_require_stop() {
    for input in [AudioInput::System, AudioInput::Microphone, AudioInput::Both] {
        let draft = SettingsDraft {
            audio_input: Some(input),
            ..Default::default()
        };
        assert!(ensure_settings_draft_allowed(&draft, true).is_err());
        assert!(ensure_settings_draft_allowed(&draft, false).is_ok());
        assert!(ensure_settings_draft_window_allowed("settings", &draft).is_ok());
        for window in ["tray-panel", "overlay", "overlay-control"] {
            assert!(ensure_settings_draft_window_allowed(window, &draft).is_err());
        }
    }
    assert!(
        serde_json::from_value::<SettingsDraft>(serde_json::json!({"audioInput":"none"})).is_err()
    );
}

#[test]
fn every_proxy_mode_is_camel_case_and_requires_a_stopped_session() {
    for value in [
        serde_json::json!({"mode":"system"}),
        serde_json::json!({"mode":"direct"}),
        serde_json::json!({"mode":"custom","url":"http://127.0.0.1:8888"}),
    ] {
        let draft: SettingsDraft =
            serde_json::from_value(serde_json::json!({"networkProxy":value})).unwrap();
        assert!(draft.network_proxy.is_some());
        assert_eq!(
            ensure_settings_draft_allowed(&draft, true),
            Err("network_proxy_change_requires_stop".into())
        );
        assert!(ensure_settings_draft_allowed(&draft, false).is_ok());
    }
    assert!(SettingsDraft::default().network_proxy.is_none());
}

#[test]
fn custom_color_draft_accepts_rgb_and_rejects_invalid_values() {
    let draft: SettingsDraft = serde_json::from_str(r##"{"subtitleColor":"#a1b2c3"}"##).unwrap();
    assert_eq!(
        draft.subtitle_color,
        Some(SubtitleColor::Custom([0xa1, 0xb2, 0xc3]))
    );
    assert!(ensure_settings_draft_allowed(&draft, true).is_ok());
    assert!(serde_json::from_str::<SettingsDraft>(r##"{"subtitleColor":"#abc"}"##).is_err());
}

#[test]
fn active_session_rejects_pipeline_settings_but_allows_visual_settings() {
    let pipeline = SettingsDraft {
        source_language: Some(SourceLanguage::Japanese),
        ..SettingsDraft::default()
    };
    assert!(ensure_settings_draft_allowed(&pipeline, true).is_err());

    let visual = SettingsDraft {
        font_size: Some(19.0),
        subtitle_font_family: Some("Noto Sans CJK SC".into()),
        subtitle_background_opacity: Some(65),
        subtitle_color: Some(SubtitleColor::Custom([0x12, 0x34, 0x56])),
        subtitle_alignment: Some(SubtitleAlignment::Right),
        subtitle_display_mode: Some(SubtitleDisplayMode::Bilingual),
        show_subtitle_dividers: Some(true),
        pulse_animation: Some(true),
        pulse_style: Some(PulseStyle::Syllable),
        subtitle_animation: Some(true),
        subtitle_blends_with_background: Some(true),
        is_overlay_locked: Some(true),
        ui_language: Some("ja".into()),
        ..SettingsDraft::default()
    };
    assert!(ensure_settings_draft_allowed(&visual, true).is_ok());
}

#[test]
fn one_unavailable_credential_does_not_fail_the_snapshot() {
    let store = SettingsStore::in_memory(Box::new(PartiallyUnavailableSecretStore), false);
    store
        .create_profile(ProviderKind::OpenAIRealtime, "OpenAI")
        .unwrap();

    let payload = SettingsSnapshotPayload::try_from_store(&store).unwrap();
    assert_eq!(payload.profiles.len(), 2);
    assert_eq!(
        payload.profiles[0].credential_state,
        CredentialState::Unavailable
    );
    assert_eq!(
        payload.profiles[1].credential_state,
        CredentialState::Missing
    );
}
