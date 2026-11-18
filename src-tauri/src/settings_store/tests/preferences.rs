//! Preferences behavior regressions.

use super::*;

#[test]
fn fresh_install_stays_empty_after_saving_preferences_and_restarting() {
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = fresh_store(directory.path(), &fake);
    assert_eq!(store.profile_catalog().unwrap(), (String::new(), vec![]));
    store
        .save_preferences_for_active_profile(|prefs| prefs.font_size = 19.0)
        .unwrap();
    drop(store);
    let restarted = fresh_store(directory.path(), &fake);
    assert_eq!(
        restarted.profile_catalog().unwrap(),
        (String::new(), vec![])
    );
    assert_eq!(restarted.preferences().font_size, 19.0);
    assert!(ProfileCatalog::default().validated().is_ok());
    assert!(ProfileCatalog {
        active_profile_id: "missing".into(),
        ..ProfileCatalog::default()
    }
    .validated()
    .is_err());
}

#[test]
fn failed_first_profile_write_restores_the_empty_catalog_and_preferences() {
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let mut store = fresh_store(directory.path(), &fake);
    let original = store.preferences();
    store.catalog_path = directory.path().join("blocked-catalog");
    std::fs::create_dir(&store.catalog_path).unwrap();
    assert!(store
        .create_profile(ProviderKind::GoogleGeminiLive, "First service")
        .is_err());
    assert_eq!(store.profile_catalog().unwrap(), (String::new(), vec![]));
    assert_eq!(store.preferences(), original);
    drop(store);
    assert_eq!(
        fresh_store(directory.path(), &fake)
            .profile_catalog()
            .unwrap(),
        (String::new(), vec![])
    );
}

#[test]
fn incomplete_profile_selection_leaves_current_preferences_and_catalog_untouched() {
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = SettingsStore::at_path(directory.path().into(), Box::new(fake));
    store
        .save_api_key(DEFAULT_ALIBABA_PROFILE_ID, "synthetic-current")
        .unwrap();
    let next = store
        .create_profile(ProviderKind::OpenAIRealtime, "Unconfigured")
        .unwrap();
    store
        .save_preferences_for_active_profile(|prefs| {
            prefs.source_language = SourceLanguage::Japanese
        })
        .unwrap();
    let current_configuration = store.configuration().unwrap();
    let previous = store.preferences();
    let active = store.active_profile().unwrap();
    let catalog = std::fs::read(&store.catalog_path).unwrap();
    let persisted = std::fs::read(&store.prefs_path).unwrap();

    assert!(store.configuration_for_profile_selection(&next).is_err());
    assert_eq!(store.configuration().unwrap(), current_configuration);
    assert_eq!(store.preferences(), previous);
    assert_eq!(store.active_profile().unwrap(), active);
    assert_eq!(std::fs::read(&store.catalog_path).unwrap(), catalog);
    assert_eq!(std::fs::read(&store.prefs_path).unwrap(), persisted);
}

#[test]
fn dock_visibility_defaults_on_and_preserves_saved_choices_without_credentials() {
    let legacy: Preferences = serde_json::from_str(r#"{"ui_language":"ja"}"#).unwrap();
    assert!(legacy.show_in_dock);
    let hidden: Preferences =
        serde_json::from_str(r#"{"ui_language":"ja","show_in_dock":false}"#).unwrap();
    assert!(!hidden.show_in_dock);
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
    assert!(store.preferences().show_in_dock);
    store
        .save_preferences_for_active_profile(|prefs| prefs.show_in_dock = false)
        .unwrap();
    let profile = store
        .create_profile(ProviderKind::OpenAIRealtime, "Synthetic")
        .unwrap();
    store.select_profile(&profile.id).unwrap();
    let reloaded = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
    assert!(!reloaded.preferences().show_in_dock);
    reloaded
        .save_preferences(|prefs| prefs.show_in_dock = true)
        .unwrap();
    let final_store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
    assert!(final_store.preferences().show_in_dock);
    assert!(fake.state.lock().unwrap().loads.is_empty());
}

#[test]
fn application_target_defaults_and_requires_fresh_recording_opt_in() {
    use crate::core::system_audio_target::SystemAudioTarget;
    let mut preferences: Preferences = serde_json::from_str("{}").unwrap();
    assert_eq!(preferences.system_audio_target, SystemAudioTarget::System);
    preferences.record_session_audio = true;
    let target = SystemAudioTarget::Application {
        id: "com.example.player".into(),
        name: "Player".into(),
    };
    preferences.apply_system_audio_target(target.clone());
    assert!(!preferences.record_session_audio);
    preferences.record_session_audio = true;
    preferences.apply_system_audio_target(target.clone());
    assert!(preferences.record_session_audio);
    let restored: Preferences =
        serde_json::from_value(serde_json::to_value(&preferences).unwrap()).unwrap();
    assert_eq!(restored.system_audio_target, target);
    preferences.apply_system_audio_target(SystemAudioTarget::System);
    assert!(!preferences.record_session_audio);
}

#[test]
fn playback_stream_selection_persists_only_identity_and_revokes_recording_consent() {
    use crate::core::system_audio_target::SystemAudioTarget;
    for input in [AudioInput::System, AudioInput::Microphone, AudioInput::Both] {
        let mut preferences = Preferences {
            audio_input: input,
            record_session_audio: true,
            ..Default::default()
        };
        let target = SystemAudioTarget::Application {
            id: "opaque-stream-lifetime".into(),
            name: "App · stream".into(),
        };
        preferences.apply_system_audio_target(target.clone());
        assert!(!preferences.record_session_audio);
        assert_eq!(preferences.audio_input, input);
        let value = serde_json::to_value(&preferences).unwrap();
        assert_eq!(value["system_audio_target"]["kind"], "application");
        let restored: Preferences = serde_json::from_value(value).unwrap();
        assert_eq!(restored.system_audio_target, target);
        preferences.record_session_audio = true;
        preferences.apply_system_audio_target(target);
        assert!(
            preferences.record_session_audio,
            "repeating an unchanged selection is a no-op"
        );
    }
}

#[test]
fn changing_audio_input_requires_a_new_recording_opt_in() {
    for previous in [AudioInput::System, AudioInput::Microphone, AudioInput::Both] {
        for next in [AudioInput::System, AudioInput::Microphone, AudioInput::Both] {
            if previous == next {
                continue;
            }
            for request in [None, Some(false), Some(true)] {
                let mut preferences = super::Preferences {
                    audio_input: previous,
                    record_session_audio: true,
                    ..Default::default()
                };
                preferences.apply_audio_preferences(Some(next), request);
                assert_eq!(preferences.audio_input, next);
                assert!(!preferences.record_session_audio);
                preferences.apply_audio_preferences(None, Some(true));
                assert!(preferences.record_session_audio);
                preferences.apply_audio_preferences(Some(next), None);
                assert!(preferences.record_session_audio);
            }
        }
    }
}

#[test]
fn loading_preferences_preserves_explicit_inputs_and_their_recording_opt_in() {
    for blocked_catalog in [false, true] {
        for input in [AudioInput::System, AudioInput::Microphone, AudioInput::Both] {
            let directory = tempfile::tempdir().unwrap();
            let previous = Preferences {
                audio_input: input,
                record_session_audio: true,
                font_size: 19.0,
                overlay_frame: Some(OverlayFrame {
                    x: 20.0,
                    y: 30.0,
                    width: 700.0,
                    height: 350.0,
                }),
                ..Preferences::default()
            };
            std::fs::write(
                directory.path().join("preferences.json"),
                serde_json::to_vec(&previous).unwrap(),
            )
            .unwrap();
            if blocked_catalog {
                std::fs::write(directory.path().join(PROFILE_CATALOG_FILE), b"invalid").unwrap();
            }
            let store = SettingsStore::at_path(
                directory.path().into(),
                Box::new(FakeSecretStore::default()),
            );
            let loaded = store.preferences();
            assert_eq!(loaded.audio_input, input);
            assert!(loaded.record_session_audio);
            assert_eq!(loaded.font_size, previous.font_size);
            assert_eq!(loaded.overlay_frame, previous.overlay_frame);
            let persisted: Preferences = serde_json::from_slice(
                &std::fs::read(directory.path().join("preferences.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(persisted.audio_input, input);
            assert!(persisted.record_session_audio);
        }
    }
}

#[test]
fn windows_source_defaults_to_system_and_preserves_stable_id() {
    let legacy: super::Preferences = serde_json::from_str("{}").unwrap();
    assert!(legacy.windows_audio_source.is_empty());
    let preferences = super::Preferences {
        windows_audio_source: "wasapi:stable-render-id".into(),
        ..Default::default()
    };
    let restored: super::Preferences =
        serde_json::from_str(&serde_json::to_string(&preferences).unwrap()).unwrap();
    assert_eq!(
        restored.windows_audio_source,
        preferences.windows_audio_source
    );
}

#[test]
fn windows_source_changes_require_fresh_recording_opt_in_and_persist_it() {
    for input in [AudioInput::System, AudioInput::Microphone, AudioInput::Both] {
        for recording in [None, Some(false), Some(true)] {
            let directory = tempfile::tempdir().unwrap();
            let store = SettingsStore::at_path(
                directory.path().into(),
                Box::new(FakeSecretStore::default()),
            );
            store
                .save_preferences_for_active_profile(|prefs| {
                    prefs.audio_input = input;
                    prefs.windows_audio_source = "wasapi:synthetic-first".into();
                    prefs.record_session_audio = true;
                })
                .unwrap();
            store
                .save_preferences_for_active_profile(|prefs| {
                    prefs.apply_audio_preferences(None, recording);
                    prefs.apply_windows_audio_source("wasapi:synthetic-second".into());
                })
                .unwrap();
            let changed = store.preferences();
            assert_eq!(changed.audio_input, input);
            assert_eq!(changed.windows_audio_source, "wasapi:synthetic-second");
            assert!(!changed.record_session_audio);
            let reloaded = SettingsStore::at_path(
                directory.path().into(),
                Box::new(FakeSecretStore::default()),
            );
            assert!(!reloaded.preferences().record_session_audio);
            reloaded
                .save_preferences_for_active_profile(|prefs| {
                    prefs.apply_audio_preferences(None, Some(true));
                })
                .unwrap();
            reloaded
                .save_preferences_for_active_profile(|prefs| {
                    prefs.apply_windows_audio_source("wasapi:synthetic-second".into());
                })
                .unwrap();
            assert!(reloaded.preferences().record_session_audio);
            reloaded
                .save_preferences_for_active_profile(|prefs| {
                    prefs.apply_windows_audio_source(String::new());
                })
                .unwrap();
            assert!(!reloaded.preferences().record_session_audio);
        }
    }
}

#[test]
fn explicit_ui_fixture_restarts_non_secret_preferences_only() {
    let directory = private_ui_fixture();
    let store = SettingsStore::load_ui_test_preferences(directory.clone()).unwrap();
    store
        .save_preferences(|preferences| {
            preferences.pulse_style = PulseStyle::Ribbon;
            preferences.pulse_animation = Some(false);
            preferences.subtitle_animation = Some(true);
            preferences.ui_language = Some("ja".into());
            preferences.subtitle_font_family = "Noto Sans CJK JP".into();
            preferences.windows_audio_source = "safe-fixture-endpoint".into();
        })
        .unwrap();
    store
        .create_profile(ProviderKind::OpenAIRealtime, "Synthetic fixture")
        .unwrap();
    let expected = store.preferences();
    drop(store);
    let restarted = SettingsStore::load_ui_test_preferences(directory.clone()).unwrap();
    assert_eq!(restarted.preferences(), expected);
    assert!(restarted.is_ui_test());
    assert_eq!(restarted.profile_catalog().unwrap().1.len(), 1);
    assert!(!directory.join(PROFILE_CATALOG_FILE).exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn explicit_ui_fixture_rejects_unbounded_or_invalid_preferences() {
    let directory = private_ui_fixture();
    let path = directory.join("preferences.json");
    std::fs::write(&path, vec![b' '; 65537]).unwrap();
    assert!(SettingsStore::load_ui_test_preferences(directory.clone()).is_err());
    std::fs::write(&path, b"invalid").unwrap();
    assert!(SettingsStore::load_ui_test_preferences(directory.clone()).is_err());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn startup_preserves_the_existing_identical_preferences_file() {
    let directory = tempfile::tempdir().unwrap();
    let store = SettingsStore::at_path(directory.path().into(), Box::<FakeSecretStore>::default());
    store.prepare_for_listening().unwrap();
    OpenOptions::new()
        .write(true)
        .open(&store.prefs_path)
        .unwrap()
        .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000))
        .unwrap();
    let before = std::fs::metadata(&store.prefs_path)
        .unwrap()
        .modified()
        .unwrap();

    store.prepare_for_listening().unwrap();

    let after = std::fs::metadata(&store.prefs_path)
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(before, after, "startup rewrote identical preferences");
}

#[test]
fn startup_recovers_missing_or_damaged_preferences_and_surfaces_failed_writes() {
    let directory = tempfile::tempdir().unwrap();
    let store = SettingsStore::at_path(directory.path().into(), Box::<FakeSecretStore>::default());
    for contents in [None, Some(b"{ broken".as_slice())] {
        if let Some(bytes) = contents {
            std::fs::write(&store.prefs_path, bytes).unwrap();
        }
        store.prepare_for_listening().unwrap();
        let persisted: Preferences =
            serde_json::from_slice(&std::fs::read(&store.prefs_path).unwrap()).unwrap();
        assert_eq!(persisted, store.preferences());
    }
    std::fs::remove_file(&store.prefs_path).unwrap();
    std::fs::create_dir(&store.prefs_path).unwrap();
    assert_eq!(
        store.prepare_for_listening().unwrap_err(),
        PREFERENCES_UNAVAILABLE
    );
}

#[test]
fn failed_preference_write_leaves_memory_unchanged() {
    let fake = FakeSecretStore::default();
    let directory = std::env::temp_dir().join(format!(
        "mimi-preferences-failure-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let store = SettingsStore::at_path(directory.clone(), Box::new(fake));
    let before = store.preferences();

    std::fs::remove_file(directory.join(PROFILE_CATALOG_FILE)).unwrap();
    std::fs::remove_dir(&directory).unwrap();
    std::fs::write(&directory, "not a directory").unwrap();

    assert_eq!(
        store
            .save_preferences(|prefs| prefs.font_size = 20.0)
            .unwrap_err(),
        PREFERENCES_UNAVAILABLE
    );
    assert_eq!(store.preferences(), before);

    let _ = std::fs::remove_file(directory);
}

#[test]
fn subtitle_font_size_keeps_saved_choices_and_persists_larger_fonts() {
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
    assert_eq!(store.preferences().font_size, 16.0);

    // Existing 14–20px choices survive the wider range without migration.
    // Both the ordinary and active-profile save routes retain larger sizes.
    for font_size in [14.0, 16.0, 18.5, 20.0, 24.0, 32.0, 48.0] {
        store
            .save_preferences(|prefs| prefs.font_size = font_size)
            .unwrap();
        let restarted = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        assert_eq!(restarted.preferences().font_size, font_size);
        restarted
            .save_preferences_for_active_profile(|prefs| prefs.font_size = font_size)
            .unwrap();
        let reloaded = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        assert_eq!(reloaded.preferences().font_size, font_size);
    }
}

#[test]
fn subtitle_font_size_normalization_is_durable_for_invalid_inputs() {
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
    for (input, expected) in [
        (-1.0, 14.0),
        (100.0, 48.0),
        (f64::NAN, 16.0),
        (f64::INFINITY, 16.0),
        (f64::NEG_INFINITY, 16.0),
    ] {
        store
            .save_preferences(|prefs| prefs.font_size = input)
            .unwrap();
        assert_eq!(store.preferences().font_size, expected);
        let saved: serde_json::Value = serde_json::from_slice(
            &std::fs::read(directory.path().join("preferences.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(saved["font_size"].as_f64(), Some(expected));
        let reloaded = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        assert_eq!(reloaded.preferences().font_size, expected);
    }
}

#[test]
fn subtitle_font_size_load_normalizes_only_out_of_range_values() {
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    for (saved, expected) in [
        (14.0, 14.0),
        (16.0, 16.0),
        (20.0, 20.0),
        (32.0, 32.0),
        (48.0, 48.0),
        (-1.0, 14.0),
        (100.0, 48.0),
    ] {
        std::fs::write(
            directory.path().join("preferences.json"),
            serde_json::to_vec(&serde_json::json!({ "font_size": saved })).unwrap(),
        )
        .unwrap();
        let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        assert_eq!(store.preferences().font_size, expected);
    }
}

#[test]
fn intermediate_subtitle_preference_round_trips_both_choices() {
    for enabled in [false, true] {
        let prefs = Preferences {
            show_intermediate_subtitles: enabled,
            ..Preferences::default()
        };
        let restored: Preferences =
            serde_json::from_str(&serde_json::to_string(&prefs).unwrap()).unwrap();
        assert_eq!(restored.show_intermediate_subtitles, enabled);
    }
}

#[test]
fn pulse_styles_survive_disk_reload_and_unrelated_updates_without_secret_access() {
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    for style in [PulseStyle::Syllable, PulseStyle::Ribbon] {
        let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        store
            .save_preferences(|prefs| {
                prefs.pulse_animation = Some(false);
                prefs.subtitle_animation = Some(true);
            })
            .unwrap();
        store
            .save_preferences_for_active_profile(|prefs| prefs.pulse_style = style)
            .unwrap();
        store
            .save_preferences_for_active_profile(|prefs| prefs.font_size = 19.0)
            .unwrap();
        let restored = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        let prefs = restored.preferences();
        assert_eq!(prefs.pulse_style, style);
        assert_eq!(prefs.font_size, 19.0);
        assert_eq!(prefs.pulse_animation, Some(false));
        assert_eq!(prefs.subtitle_animation, Some(true));
    }
    assert!(fake.state.lock().unwrap().loads.is_empty());
}

#[test]
fn subtitle_font_family_persists_and_resets_without_secret_access() {
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
    let original_catalog = store.profile_catalog().unwrap();
    for (input, expected) in [
        ("  思源黑体  ", "思源黑体"),
        (
            "A Font Removed From This Computer",
            "A Font Removed From This Computer",
        ),
        ("", ""),
    ] {
        store
            .save_preferences_for_active_profile(|prefs| {
                prefs.subtitle_font_family = input.into();
            })
            .unwrap();
        store
            .save_preferences_for_active_profile(|prefs| prefs.font_size = 19.0)
            .unwrap();
        let reloaded = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        assert_eq!(reloaded.preferences().subtitle_font_family, expected);
        assert_eq!(reloaded.preferences().font_size, 19.0);
        assert_eq!(reloaded.profile_catalog().unwrap(), original_catalog);
    }
    let state = fake.state.lock().unwrap();
    assert!(state.loads.is_empty());
    assert!(state.values.is_empty());
}

#[test]
fn subtitle_font_invalid_save_rolls_back_all_preferences_and_disk() {
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
    store
        .save_preferences(|prefs| prefs.subtitle_font_family = "Noto Sans".into())
        .unwrap();
    let before = store.preferences();
    let bytes = std::fs::read(directory.path().join("preferences.json")).unwrap();
    for invalid in ["Font\nName".to_owned(), "字".repeat(257)] {
        assert_eq!(
            store.save_preferences_for_active_profile(|prefs| {
                prefs.font_size = 20.0;
                prefs.subtitle_font_family = invalid;
            }),
            Err(crate::core::subtitle_font::INVALID_FAMILY_NAME.into())
        );
        assert_eq!(store.preferences(), before);
        assert_eq!(
            std::fs::read(directory.path().join("preferences.json")).unwrap(),
            bytes
        );
    }
    assert!(fake.state.lock().unwrap().loads.is_empty());
}

#[test]
fn subtitle_font_load_sanitizes_invalid_names_without_resetting_other_preferences() {
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    for (input, expected) in [
        ("  Noto Sans  ".to_owned(), "Noto Sans"),
        ("Font\u{7f}".to_owned(), ""),
        ("字".repeat(257), ""),
    ] {
        std::fs::write(
            directory.path().join("preferences.json"),
            serde_json::to_vec(&serde_json::json!({
                "subtitle_font_family": input,
                "font_size": 19,
                "show_in_dock": false
            }))
            .unwrap(),
        )
        .unwrap();
        let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        assert_eq!(store.preferences().subtitle_font_family, expected);
        assert_eq!(store.preferences().font_size, 19.0);
        assert!(!store.preferences().show_in_dock);
    }
    assert!(fake.state.lock().unwrap().loads.is_empty());
}

#[test]
fn subtitle_background_opacity_persists_and_is_bounded_on_save_and_load() {
    let directory = std::env::temp_dir().join(format!(
        "mimi-background-opacity-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let fake = FakeSecretStore::default();
    let store = SettingsStore::at_path(directory.clone(), Box::new(fake.clone()));
    for (input, expected) in [(0, 0), (35, 35), (100, 100), (255, 100)] {
        store
            .save_preferences(|prefs| prefs.subtitle_background_opacity = input)
            .unwrap();
        let reloaded = SettingsStore::at_path(directory.clone(), Box::new(fake.clone()));
        assert_eq!(reloaded.preferences().subtitle_background_opacity, expected);
    }
    std::fs::write(
        directory.join("preferences.json"),
        br#"{"subtitle_background_opacity":255}"#,
    )
    .unwrap();
    let reloaded = SettingsStore::at_path(directory.clone(), Box::new(fake));
    assert_eq!(reloaded.preferences().subtitle_background_opacity, 100);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn visible_subtitle_count_defaults_off_and_persists_clamped() {
    // Legacy preferences without the new fields keep the switch off.
    let legacy: Preferences = serde_json::from_str(r#"{"ui_language":"ja"}"#).unwrap();
    assert!(!legacy.custom_subtitle_count_enabled);
    assert_eq!(
        legacy.visible_subtitle_count,
        DEFAULT_VISIBLE_SUBTITLE_COUNT
    );

    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
    store
        .save_preferences_for_active_profile(|prefs| {
            prefs.custom_subtitle_count_enabled = true;
            prefs.visible_subtitle_count = 7;
        })
        .unwrap();
    let reloaded = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
    assert!(reloaded.preferences().custom_subtitle_count_enabled);
    assert_eq!(reloaded.preferences().visible_subtitle_count, 7);
    let payload = crate::commands::SettingsSnapshotPayload::try_from_store(&reloaded).unwrap();
    assert!(payload.custom_subtitle_count_enabled);
    assert_eq!(payload.visible_subtitle_count, 7);

    store
        .save_preferences_for_active_profile(|prefs| {
            prefs.custom_subtitle_count_enabled = false;
        })
        .unwrap();
    let all = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
    assert!(!all.preferences().custom_subtitle_count_enabled);
    assert_eq!(all.preferences().visible_subtitle_count, 7);

    // Out-of-range stored choices are clamped into 1..=10 on save.
    for (stored, expected) in [(0, 1), (11, 10), (255, 10)] {
        store
            .save_preferences_for_active_profile(|prefs| {
                prefs.visible_subtitle_count = stored;
            })
            .unwrap();
        assert_eq!(store.preferences().visible_subtitle_count, expected);
    }
    assert_eq!(clamp_visible_subtitle_count(0), 1);
    assert_eq!(clamp_visible_subtitle_count(10), 10);
}

#[test]
fn visible_subtitle_count_clamps_loaded_preferences_without_resetting_other_choices() {
    let directory = tempfile::tempdir().unwrap();
    for (stored, expected) in [(0, 1), (11, 10), (255, 10)] {
        let prefs = serde_json::json!({
            "custom_subtitle_count_enabled": true,
            "visible_subtitle_count": stored,
            "ui_language": "ja",
            "show_subtitle_timestamps": true
        });
        std::fs::write(
            directory.path().join("preferences.json"),
            serde_json::to_vec(&prefs).unwrap(),
        )
        .unwrap();
        let store = SettingsStore::at_path(
            directory.path().into(),
            Box::new(FakeSecretStore::default()),
        );
        let loaded = store.preferences();
        assert!(loaded.custom_subtitle_count_enabled);
        assert_eq!(loaded.visible_subtitle_count, expected);
        assert_eq!(loaded.ui_language.as_deref(), Some("ja"));
        assert!(loaded.show_subtitle_timestamps);
    }
}

#[test]
fn invalid_proxy_save_rolls_back_memory_and_disk_without_storing_authentication() {
    use crate::core::network_proxy::ProxyMode;
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
    store
        .save_preferences(|prefs| prefs.font_size = 19.0)
        .unwrap();
    let before = store.preferences();
    let result = store.save_preferences(|prefs| {
        prefs.font_size = 20.0;
        prefs.network_proxy = ProxyConfig {
            mode: ProxyMode::Custom,
            url: Some("http://synthetic-user:synthetic-password@127.0.0.1:8888".into()),
        };
    });
    assert!(result.is_err());
    assert_eq!(store.preferences(), before);
    let reloaded = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
    assert_eq!(reloaded.preferences(), before);
    assert!(fake.state.lock().unwrap().values.is_empty());
}

#[test]
fn loading_normalizes_preferences_for_the_active_provider() {
    let fake = FakeSecretStore::default();
    let directory = std::env::temp_dir().join(format!(
        "mimi-preferences-normalize-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&directory).unwrap();

    let openai = ServiceProfile::new("openai", "OpenAI", ProviderKind::OpenAIRealtime).unwrap();
    let catalog = ProfileCatalog {
        schema_version: PROFILE_CATALOG_SCHEMA_VERSION,
        active_profile_id: openai.id.clone(),
        profiles: vec![ServiceProfile::alibaba_default(), openai.clone()],
    };
    std::fs::write(
        directory.join(PROFILE_CATALOG_FILE),
        serde_json::to_vec_pretty(&catalog).unwrap(),
    )
    .unwrap();
    let stale = Preferences {
        source_language: SourceLanguage::Japanese,
        target_language: TargetLanguage::Original,
        translation_mode: TranslationMode::HighQuality,
        ..Preferences::default()
    };
    std::fs::write(
        directory.join("preferences.json"),
        serde_json::to_vec_pretty(&stale).unwrap(),
    )
    .unwrap();
    fake.put(
        PROFILE_KEYCHAIN_SERVICE,
        &credential_account(&openai),
        "sk-openai-test",
    );

    let store = SettingsStore::at_path(directory.clone(), Box::new(fake));
    let normalized = store.preferences();
    assert_eq!(normalized.source_language, SourceLanguage::Automatic);
    assert_eq!(
        normalized.target_language,
        TargetLanguage::SimplifiedChinese
    );
    assert_eq!(normalized.translation_mode, TranslationMode::Turbo);
    assert_eq!(
        store.configuration().unwrap().provider,
        ProviderKind::OpenAIRealtime
    );

    let persisted: Preferences =
        serde_json::from_slice(&std::fs::read(directory.join("preferences.json")).unwrap())
            .unwrap();
    assert_eq!(persisted, normalized);
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn failed_profile_selection_rolls_preferences_back() {
    let fake = FakeSecretStore::default();
    let directory = std::env::temp_dir().join(format!(
        "mimi-profile-selection-rollback-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let store = SettingsStore::at_path(directory.clone(), Box::new(fake));
    let openai = store
        .create_profile(ProviderKind::OpenAIRealtime, "OpenAI")
        .unwrap();
    store
        .save_preferences(|prefs| {
            prefs.source_language = SourceLanguage::Japanese;
            prefs.target_language = TargetLanguage::Original;
            prefs.translation_mode = TranslationMode::HighQuality;
        })
        .unwrap();
    let before = store.preferences();

    let catalog_path = directory.join(PROFILE_CATALOG_FILE);
    std::fs::remove_file(&catalog_path).unwrap();
    std::fs::create_dir(&catalog_path).unwrap();

    assert_eq!(
        store.select_profile(&openai.id).unwrap_err(),
        PROFILE_CATALOG_UNAVAILABLE
    );
    assert_eq!(store.preferences(), before);
    assert_eq!(
        store.active_profile().unwrap().id,
        DEFAULT_ALIBABA_PROFILE_ID
    );
    let persisted: Preferences =
        serde_json::from_slice(&std::fs::read(directory.join("preferences.json")).unwrap())
            .unwrap();
    assert_eq!(persisted, before);

    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn ordinary_ui_test_does_not_read_or_write_existing_user_config() {
    let directory = private_ui_fixture();
    let preferences = directory.join("preferences.json");
    let catalog = directory.join(PROFILE_CATALOG_FILE);
    std::fs::write(&preferences, br#"{"pulse_style":"ribbon","font_size":20}"#).unwrap();
    std::fs::write(&catalog, b"existing fixture catalog").unwrap();
    let store = SettingsStore::load(directory.clone(), true, DEVELOPMENT_APPLICATION_IDENTIFIER);
    assert_eq!(store.preferences(), Preferences::default());
    assert_eq!(store.profile_catalog().unwrap(), (String::new(), vec![]));
    store
        .save_preferences(|preferences| preferences.pulse_style = PulseStyle::Syllable)
        .unwrap();
    assert_eq!(
        std::fs::read(&preferences).unwrap(),
        br#"{"pulse_style":"ribbon","font_size":20}"#
    );
    assert_eq!(
        std::fs::read(&catalog).unwrap(),
        b"existing fixture catalog"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[cfg(unix)]
#[test]
fn explicit_ui_fixture_rejects_symlinks_and_shared_directories() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let directory = private_ui_fixture();
    let path = directory.join("preferences.json");
    let target = directory.join("fixture.json");
    std::fs::write(&target, b"{}").unwrap();
    symlink(&target, &path).unwrap();
    assert!(SettingsStore::load_ui_test_preferences(directory.clone()).is_err());
    std::fs::remove_file(&path).unwrap();
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(SettingsStore::load_ui_test_preferences(directory.clone()).is_err());
    std::fs::remove_dir_all(directory).unwrap();
}
