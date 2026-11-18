use super::*;

#[test]
fn native_menu_language_preserves_chinese_scripts_and_regions() {
    for locale in [
        "zh-TW",
        "zh-HK",
        "zh-MO",
        "zh-Hant",
        "zh-Hant-CN",
        "ZH_hant_HK",
    ] {
        assert_eq!(
            effective_native_ui_language(Some("system"), Some(locale)),
            NativeUiLanguage::TraditionalChinese,
            "{locale}"
        );
    }
    for locale in [
        "zh",
        "zh-CN",
        "zh-SG",
        "zh-Hans",
        "zh-Hans-HK",
        "ZH_hans_CN",
    ] {
        assert_eq!(
            effective_native_ui_language(None, Some(locale)),
            NativeUiLanguage::Chinese,
            "{locale}"
        );
    }
    assert_eq!(
        effective_native_ui_language(Some("zh-TW"), Some("zh-CN")),
        NativeUiLanguage::TraditionalChinese
    );
    assert_eq!(
        effective_native_ui_language(Some("zh"), Some("zh-Hant")),
        NativeUiLanguage::Chinese
    );
    let labels = native_menu_labels(NativeUiLanguage::TraditionalChinese);
    assert_eq!(labels.start_subtitles, "開始字幕");
    assert_eq!(labels.stop_subtitles, "停止字幕");
    assert_eq!(labels.toggle_devtools, "開啟開發者工具");
    assert_eq!(labels.settings, "設定…");
    assert_eq!(labels.quit, "結束 Mimi");
    assert_eq!(labels.subtitle_display, "字幕顯示");
    assert_eq!(labels.display_modes, ["僅譯文", "原文與譯文", "僅原文"]);
    assert_eq!(labels.show_in_dock, "在 Dock 中顯示");
}

#[test]
fn added_interface_languages_localize_native_menus() {
    for (code, region, language, label) in [
        ("de", "de-DE", NativeUiLanguage::German, "Einstellungen…"),
        ("fr", "fr-CA", NativeUiLanguage::French, "Réglages…"),
        ("ko", "ko-KR", NativeUiLanguage::Korean, "설정…"),
        ("th", "TH_th", NativeUiLanguage::Thai, "การตั้งค่า…"),
        ("ru", "RU_ru", NativeUiLanguage::Russian, "Настройки…"),
    ] {
        assert_eq!(
            effective_native_ui_language(Some(code), Some("en-US")),
            language
        );
        assert_eq!(
            effective_native_ui_language(Some("system"), Some(region)),
            language
        );
        assert_eq!(native_menu_labels(language).settings, label);
        assert_ne!(
            native_menu_labels(language).quit,
            native_menu_labels(NativeUiLanguage::English).quit
        );
    }
}

#[test]
fn application_settings_and_tray_share_the_settings_route_in_all_languages() {
    assert!(is_settings_menu_event("settings"));
    assert!(is_settings_menu_event(APPLICATION_SETTINGS_MENU_ID));
    for id in [
        "quit",
        APPLICATION_QUIT_MENU_ID,
        "live-subtitles",
        "mimi-app-settings-extra",
    ] {
        assert!(!is_settings_menu_event(id));
    }
    assert_eq!(APPLICATION_SETTINGS_ACCELERATOR, "CmdOrCtrl+,");
    for (language, label) in [
        (NativeUiLanguage::Chinese, "设置…"),
        (NativeUiLanguage::English, "Settings…"),
        (NativeUiLanguage::Japanese, "設定…"),
    ] {
        assert_eq!(native_menu_labels(language).settings, label);
    }
}

#[test]
fn application_and_tray_quit_share_the_normal_finalization_route() {
    assert!(is_normal_quit_menu_event("quit"));
    assert!(is_normal_quit_menu_event(APPLICATION_QUIT_MENU_ID));
    for id in ["settings", "live-subtitles", "close", "mimi-app-quit-extra"] {
        assert!(!is_normal_quit_menu_event(id));
    }
    assert_eq!(APPLICATION_QUIT_ACCELERATOR, "CmdOrCtrl+Q");
    for (language, label) in [
        (NativeUiLanguage::Chinese, "退出 Mimi"),
        (NativeUiLanguage::English, "Quit Mimi"),
        (NativeUiLanguage::Japanese, "Mimiを終了"),
    ] {
        assert_eq!(native_menu_labels(language).quit, label);
    }
}

#[test]
fn application_quit_replacement_accepts_only_the_last_default_predefined_quit() {
    assert_eq!(
        default_application_quit_position(8, Some("Quit mimi")),
        Some(7)
    );
    assert_eq!(default_application_quit_position(1, Some("Quit")), Some(0));
    assert_eq!(default_application_quit_position(0, Some("Quit")), None);
    assert_eq!(default_application_quit_position(8, None), None);
    for text in ["Services", "Hide mimi", "Copy", "Quitter", "退出 mimi"] {
        assert_eq!(default_application_quit_position(8, Some(text)), None);
    }
}

#[test]
fn native_tray_session_action_follows_status_in_all_three_languages() {
    for (language, start, stop) in [
        (NativeUiLanguage::Chinese, "开始字幕", "停止字幕"),
        (
            NativeUiLanguage::English,
            "Start Subtitles",
            "Stop Subtitles",
        ),
        (NativeUiLanguage::Japanese, "字幕を開始", "字幕を停止"),
    ] {
        let labels = native_menu_labels(language);
        assert_eq!(native_session_action_label(labels, false), start);
        assert_eq!(native_session_action_label(labels, true), stop);
    }

    assert!(!native_tray_session_is_active("idle"));
    assert!(native_tray_session_is_active("connecting"));
    assert!(native_tray_session_is_active("listening"));
    assert!(native_tray_session_is_active("stopping"));
    assert!(!native_tray_session_is_active("error"));
}

#[test]
fn native_tray_language_override_precedes_three_language_system_fallback() {
    assert_eq!(
        effective_native_ui_language(Some("ja"), Some("zh-CN")),
        NativeUiLanguage::Japanese
    );
    assert_eq!(
        effective_native_ui_language(None, Some("zh-Hans")),
        NativeUiLanguage::Chinese
    );
    assert_eq!(
        effective_native_ui_language(Some("system"), Some("ja-JP")),
        NativeUiLanguage::Japanese
    );
    assert_eq!(
        effective_native_ui_language(None, Some("en-US")),
        NativeUiLanguage::English
    );
    assert_eq!(
        native_menu_labels(NativeUiLanguage::Japanese).settings,
        "設定…"
    );
}

#[test]
fn native_dock_visibility_labels_match_the_existing_frontend_copy() {
    for (language, frontend_copy) in [
        (
            NativeUiLanguage::Chinese,
            include_str!("../../../../src/lib/locales/zh.ts"),
        ),
        (
            NativeUiLanguage::English,
            include_str!("../../../../src/lib/locales/en.ts"),
        ),
        (
            NativeUiLanguage::Japanese,
            include_str!("../../../../src/lib/locales/ja.ts"),
        ),
    ] {
        let label = native_menu_labels(language).show_in_dock;
        assert!(frontend_copy.contains(&format!("showInDock: \"{label}\"")));
    }
}

#[test]
fn windows_tray_asset_contains_visible_colour_pixels() {
    let windows_icon = tauri::image::Image::from_bytes(tray_icon_bytes(true)).unwrap();
    let template_icon = tauri::image::Image::from_bytes(tray_icon_bytes(false)).unwrap();

    assert_eq!((windows_icon.width(), windows_icon.height()), (32, 32));
    assert_ne!(windows_icon.rgba(), template_icon.rgba());
    assert!(windows_icon
        .rgba()
        .as_chunks::<4>()
        .0
        .iter()
        .any(|pixel| { pixel[3] != 0 && (pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0) }));
}
