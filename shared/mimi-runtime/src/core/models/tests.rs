use super::*;

#[test]
fn subtitle_palette_has_stable_wire_values() {
    for (color, name) in [
        (SubtitleColor::White, "white"),
        (SubtitleColor::Teal, "teal"),
        (SubtitleColor::Yellow, "yellow"),
        (SubtitleColor::Green, "green"),
        (SubtitleColor::Pink, "pink"),
    ] {
        assert_eq!(serde_json::to_value(color).unwrap(), name);
        assert_eq!(
            serde_json::from_value::<SubtitleColor>(name.into()).unwrap(),
            color
        );
    }
}

#[test]
fn custom_subtitle_colors_are_validated_and_normalized() {
    let color: SubtitleColor = serde_json::from_str("\"#a1b2c3\"").unwrap();
    assert_eq!(color, SubtitleColor::Custom([0xa1, 0xb2, 0xc3]));
    assert_eq!(serde_json::to_value(color).unwrap(), "#A1B2C3");
    for invalid in [
        "",
        "red",
        "#fff",
        "#12345678",
        "#GG0011",
        "123456",
        "#é0011",
        "url(x)",
    ] {
        assert!(serde_json::from_value::<SubtitleColor>(invalid.into()).is_err());
    }
}

#[test]
fn display_modes_round_trip_and_cycle_in_presentation_order() {
    let mut mode = SubtitleDisplayMode::default();
    for value in ["translation", "bilingual", "original"] {
        assert_eq!(serde_json::to_value(mode).unwrap(), value);
        assert_eq!(
            serde_json::from_value::<SubtitleDisplayMode>(value.into()).unwrap(),
            mode
        );
        mode = mode.next();
    }
    assert_eq!(mode, SubtitleDisplayMode::default());
}

#[test]
fn complete_language_codes_round_trip_without_unknown_fallback() {
    assert_eq!(SourceLanguage::ALL.len(), 137);
    assert_eq!(TargetLanguage::ALL.len(), 137);
    let mut sources = std::collections::HashSet::new();
    for language in SourceLanguage::ALL {
        assert!(sources.insert(language.raw_value()));
        assert_eq!(
            serde_json::from_value::<SourceLanguage>(serde_json::to_value(language).unwrap())
                .unwrap(),
            language
        );
        if language != SourceLanguage::Automatic {
            assert_eq!(
                SourceLanguage::from_detected(Some(language.raw_value())),
                Some(language)
            );
        }
    }
    let mut targets = std::collections::HashSet::new();
    for language in TargetLanguage::ALL {
        assert!(targets.insert(language.raw_value()));
        assert_eq!(
            serde_json::from_value::<TargetLanguage>(serde_json::to_value(language).unwrap())
                .unwrap(),
            language
        );
    }
    for unknown in ["xx", "zh-hant", "fr-FR", "", "auto"] {
        assert!(serde_json::from_value::<TargetLanguage>(serde_json::json!(unknown)).is_err());
    }
    assert!(serde_json::from_value::<SourceLanguage>(serde_json::json!("xx")).is_err());
    assert_eq!(SourceLanguage::from_detected(Some("unrecognized")), None);
}

#[test]
fn same_language_bypass_requires_explicit_matching_code_and_script() {
    for source in SourceLanguage::ALL
        .into_iter()
        .filter(|source| *source != SourceLanguage::Automatic)
    {
        for target in TargetLanguage::ALL {
            assert_eq!(
                target.matches_reported_asr(Some(source.raw_value())),
                target.raw_value() == source.raw_value()
                    && target != TargetLanguage::ChineseEnglishMixed
            );
        }
    }
    assert!(!TargetLanguage::French.matches_reported_asr(None));
    assert!(!TargetLanguage::French.matches_reported_asr(Some("fr-FR")));
    assert!(!TargetLanguage::SimplifiedChinese.matches_reported_asr(Some("zh_tw")));
    assert!(!TargetLanguage::TraditionalChinese.matches_reported_asr(Some("zh")));
    assert!(TargetLanguage::TraditionalChinese.matches_reported_asr(Some("zh-Hant")));
    assert!(!TargetLanguage::TraditionalChinese.matches_reported_asr(Some("unknown")));
}

#[test]
fn region_and_script_codes_remain_distinct_in_detected_languages() {
    for code in ["ar-EG", "pt-BR", "pt-PT", "es-MX", "zh_en", "zh_tw"] {
        let source = SourceLanguage::ALL
            .into_iter()
            .find(|source| source.raw_value() == code)
            .unwrap();
        assert_eq!(
            SourceLanguage::from_detected(Some(&code.to_ascii_lowercase())),
            Some(source)
        );
    }
    assert_eq!(
        SourceLanguage::from_detected(Some("zh-Hant")),
        Some(SourceLanguage::TraditionalChinese)
    );
    assert_eq!(
        SourceLanguage::from_detected(Some("fr-FR")),
        Some(SourceLanguage::French)
    );
    let brazil = TargetLanguage::ALL
        .into_iter()
        .find(|target| target.raw_value() == "pt-BR")
        .unwrap();
    assert!(brazil.matches_reported_asr(Some("pt-br")));
    assert!(!brazil.matches_reported_asr(Some("pt-PT")));
    assert!(!brazil.matches_reported_asr(Some("pt")));
}

#[test]
fn target_languages_expose_service_codes_and_display_names() {
    assert!(!TargetLanguage::Original.translates_audio());
    assert_eq!(TargetLanguage::SimplifiedChinese.raw_value(), "zh");
    assert_eq!(TargetLanguage::English.qwen_mt_name(), "English");
}

#[test]
fn same_language_passthrough_requires_an_explicit_compatible_asr_report() {
    for (target, reported) in [
        (TargetLanguage::English, "en"),
        (TargetLanguage::Japanese, "JA"),
        (TargetLanguage::SimplifiedChinese, "zh"),
        (TargetLanguage::SimplifiedChinese, " zh-CN "),
        (TargetLanguage::SimplifiedChinese, "zh-Hans"),
    ] {
        assert!(target.matches_reported_asr(Some(reported)));
    }
    for target in [
        TargetLanguage::Original,
        TargetLanguage::English,
        TargetLanguage::Japanese,
        TargetLanguage::SimplifiedChinese,
    ] {
        for reported in [None, Some(""), Some("unknown"), Some("yue")] {
            assert!(!target.matches_reported_asr(reported));
        }
    }
    for reported in ["zh-TW", "zh-HK", "zh-Hant", "ja", "en", "Chinese"] {
        assert!(!TargetLanguage::SimplifiedChinese.matches_reported_asr(Some(reported)));
    }
    assert!(!TargetLanguage::English.matches_reported_asr(Some("ja")));
    assert!(!TargetLanguage::Japanese.matches_reported_asr(Some("en")));
    assert!(!TargetLanguage::Original.matches_reported_asr(Some("zh")));
}

#[test]
fn detected_languages_normalize_service_codes() {
    assert_eq!(
        DetectedLanguage::from_reported(Some("ja-JP")).unwrap().code,
        "ja"
    );
    assert_eq!(
        DetectedLanguage::from_reported(Some("yue")).unwrap().code,
        "yue"
    );
    assert_eq!(
        DetectedLanguage::from_reported(Some("unknown"))
            .unwrap()
            .code,
        "unknown"
    );
}

#[test]
fn session_status_active_flag_matches_lifecycle_contract() {
    assert!(!SessionStatus::Idle.is_active());
    assert!(SessionStatus::Connecting.is_active());
    assert!(SessionStatus::Listening.is_active());
    assert!(SessionStatus::Stopping.is_active());
    assert!(!SessionStatus::Error("boom".into()).is_active());
}

#[test]
fn subtitle_pair_equality_ignores_creation_time() {
    let a = SubtitlePair::new("s".into(), "t".into(), 1);
    let b = SubtitlePair::new("s".into(), "t".into(), 999);
    assert_eq!(a, b);
}
