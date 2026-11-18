//! Native application and tray menu resources, localization and event handling.

use crate::commands::{self, AppState};
use crate::core::models::SubtitleDisplayMode;
use crate::windows;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::menu::{
    CheckMenuItem, CheckMenuItemBuilder, MenuBuilder, MenuItem, MenuItemBuilder, Submenu,
    SubmenuBuilder,
};
use tauri::tray::TrayIconBuilder;
use tauri::{Listener, Manager};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeMenuLanguage {
    Chinese,
    TraditionalChinese,
    English,
    Japanese,
    German,
    French,
    Korean,
    Thai,
    Russian,
}

#[derive(Debug, Clone, Copy)]
struct NativeMenuLabels {
    start_subtitles: &'static str,
    stop_subtitles: &'static str,
    toggle_devtools: &'static str,
    settings: &'static str,
    quit: &'static str,
    subtitle_display: &'static str,
    display_modes: [&'static str; 3],
    #[cfg(any(target_os = "macos", test))]
    show_in_dock: &'static str,
}

#[derive(Clone)]
struct NativeTrayMenuItems {
    session_action: MenuItem<tauri::Wry>,
    toggle_devtools: Option<MenuItem<tauri::Wry>>,
    settings: MenuItem<tauri::Wry>,
    quit: MenuItem<tauri::Wry>,
    subtitle_display: Submenu<tauri::Wry>,
    display_modes: [CheckMenuItem<tauri::Wry>; 3],
    #[cfg(target_os = "macos")]
    show_in_dock: CheckMenuItem<tauri::Wry>,
}

const APPLICATION_QUIT_MENU_ID: &str = "mimi-app-quit";
const APPLICATION_SETTINGS_MENU_ID: &str = "mimi-app-settings";
#[cfg(any(target_os = "macos", test))]
const APPLICATION_QUIT_ACCELERATOR: &str = "CmdOrCtrl+Q";
#[cfg(any(target_os = "macos", test))]
const APPLICATION_SETTINGS_ACCELERATOR: &str = "CmdOrCtrl+,";

#[cfg(target_os = "macos")]
struct NativeApplicationMenuItems {
    settings: MenuItem<tauri::Wry>,
    quit: MenuItem<tauri::Wry>,
}

fn is_settings_menu_event(id: &str) -> bool {
    matches!(id, "settings" | APPLICATION_SETTINGS_MENU_ID)
}

fn is_normal_quit_menu_event(id: &str) -> bool {
    matches!(id, "quit" | APPLICATION_QUIT_MENU_ID)
}

#[cfg(any(target_os = "macos", test))]
fn default_application_quit_position(
    item_count: usize,
    last_predefined_text: Option<&str>,
) -> Option<usize> {
    // Tauri's default app submenu ends with muda's English predefined Quit.
    // Validate that item before removing it, rather than replacing a Services,
    // Hide, or application-defined entry after an upstream menu change.
    let text = last_predefined_text?;
    if text != "Quit" && !text.starts_with("Quit ") {
        return None;
    }
    item_count.checked_sub(1)
}

#[cfg(target_os = "macos")]
pub(crate) fn setup_application_menu(app: &tauri::AppHandle) -> tauri::Result<()> {
    fn unexpected_default_menu() -> tauri::Error {
        std::io::Error::other("The default application menu could not be installed.").into()
    }

    // Keep Tauri's complete default menu, including native Services and the
    // Edit menu's standard copy/paste responders. Replace predefined Quit,
    // which bypasses our async finalization, and add Settings before Services
    // so it remains reachable while the subtitle controls are hidden.
    let menu = app.menu().ok_or_else(unexpected_default_menu)?;
    let root_items = menu.items()?;
    let application_menu = root_items
        .first()
        .and_then(|item| item.as_submenu())
        .ok_or_else(unexpected_default_menu)?;
    let items = application_menu.items()?;
    let last_predefined_text = items
        .last()
        .and_then(|item| item.as_predefined_menuitem())
        .map(|item| item.text())
        .transpose()?;
    let position = default_application_quit_position(items.len(), last_predefined_text.as_deref())
        .ok_or_else(unexpected_default_menu)?;
    if position < 2 {
        return Err(unexpected_default_menu());
    }

    let override_language = app
        .try_state::<AppState>()
        .and_then(|state| state.settings.preferences().ui_language);
    let system_language = system_language_code();
    let labels = native_menu_labels(effective_native_menu_language(
        override_language.as_deref(),
        system_language.as_deref(),
    ));
    let quit_item = MenuItemBuilder::with_id(APPLICATION_QUIT_MENU_ID, labels.quit)
        .accelerator(APPLICATION_QUIT_ACCELERATOR)
        .build(app)?;
    let settings_item = MenuItemBuilder::with_id(APPLICATION_SETTINGS_MENU_ID, labels.settings)
        .accelerator(APPLICATION_SETTINGS_ACCELERATOR)
        .build(app)?;
    application_menu.remove_at(position)?;
    application_menu.insert(&quit_item, position)?;
    application_menu.insert(&settings_item, 2)?;
    application_menu.insert(&tauri::menu::PredefinedMenuItem::separator(app)?, 3)?;
    app.manage(NativeApplicationMenuItems {
        settings: settings_item,
        quit: quit_item,
    });
    Ok(())
}

fn effective_native_menu_language(
    override_language: Option<&str>,
    system_language: Option<&str>,
) -> NativeMenuLanguage {
    let language = match override_language {
        Some("zh") | Some("zh-TW") | Some("en") | Some("ja") | Some("de") | Some("fr")
        | Some("ko") | Some("th") | Some("ru") => override_language,
        _ => system_language,
    }
    .unwrap_or("en")
    .to_ascii_lowercase()
    .replace('_', "-");
    if language.starts_with("zh") {
        let subtags: Vec<_> = language.split('-').skip(1).collect();
        let traditional = if subtags.contains(&"hant") {
            true
        } else if subtags.contains(&"hans") {
            false
        } else {
            subtags
                .iter()
                .any(|subtag| matches!(*subtag, "tw" | "hk" | "mo"))
        };
        if traditional {
            NativeMenuLanguage::TraditionalChinese
        } else {
            NativeMenuLanguage::Chinese
        }
    } else if language.starts_with("ja") {
        NativeMenuLanguage::Japanese
    } else if language.starts_with("de") {
        NativeMenuLanguage::German
    } else if language.starts_with("fr") {
        NativeMenuLanguage::French
    } else if language.starts_with("ko") {
        NativeMenuLanguage::Korean
    } else if language.starts_with("th") {
        NativeMenuLanguage::Thai
    } else if language.starts_with("ru") {
        NativeMenuLanguage::Russian
    } else {
        NativeMenuLanguage::English
    }
}

fn native_menu_labels(language: NativeMenuLanguage) -> NativeMenuLabels {
    match language {
        NativeMenuLanguage::Chinese => NativeMenuLabels {
            start_subtitles: "开始字幕",
            stop_subtitles: "停止字幕",
            toggle_devtools: "打开调试工具",
            settings: "设置…",
            quit: "退出 Mimi",
            subtitle_display: "字幕显示",
            display_modes: ["仅译文", "原文与译文", "仅原文"],
            #[cfg(any(target_os = "macos", test))]
            show_in_dock: "在 Dock 中显示",
        },
        NativeMenuLanguage::TraditionalChinese => NativeMenuLabels {
            start_subtitles: "開始字幕",
            stop_subtitles: "停止字幕",
            toggle_devtools: "開啟開發者工具",
            settings: "設定…",
            quit: "結束 Mimi",
            subtitle_display: "字幕顯示",
            display_modes: ["僅譯文", "原文與譯文", "僅原文"],
            #[cfg(any(target_os = "macos", test))]
            show_in_dock: "在 Dock 中顯示",
        },
        NativeMenuLanguage::Japanese => NativeMenuLabels {
            start_subtitles: "字幕を開始",
            stop_subtitles: "字幕を停止",
            toggle_devtools: "開発者ツールを開く",
            settings: "設定…",
            quit: "Mimiを終了",
            subtitle_display: "字幕表示",
            display_modes: ["翻訳のみ", "原文と翻訳", "原文のみ"],
            #[cfg(any(target_os = "macos", test))]
            show_in_dock: "Dock に表示",
        },
        NativeMenuLanguage::German => NativeMenuLabels {
            start_subtitles: "Untertitel starten",
            stop_subtitles: "Untertitel stoppen",
            toggle_devtools: "Entwicklerwerkzeuge öffnen",
            settings: "Einstellungen…",
            quit: "Mimi beenden",
            subtitle_display: "Untertitelanzeige",
            display_modes: [
                "Nur Übersetzung",
                "Original und Übersetzung",
                "Nur Original",
            ],
            #[cfg(any(target_os = "macos", test))]
            show_in_dock: "Im Dock anzeigen",
        },
        NativeMenuLanguage::French => NativeMenuLabels {
            start_subtitles: "Démarrer les sous-titres",
            stop_subtitles: "Arrêter les sous-titres",
            toggle_devtools: "Ouvrir les outils de développement",
            settings: "Réglages…",
            quit: "Quitter Mimi",
            subtitle_display: "Affichage des sous-titres",
            display_modes: [
                "Traduction seule",
                "Original et traduction",
                "Original seul",
            ],
            #[cfg(any(target_os = "macos", test))]
            show_in_dock: "Afficher dans le Dock",
        },
        NativeMenuLanguage::Korean => NativeMenuLabels {
            start_subtitles: "자막 시작",
            stop_subtitles: "자막 중지",
            toggle_devtools: "개발자 도구 열기",
            settings: "설정…",
            quit: "Mimi 종료",
            subtitle_display: "자막 표시",
            display_modes: ["번역만", "원문과 번역", "원문만"],
            #[cfg(any(target_os = "macos", test))]
            show_in_dock: "Dock에 표시",
        },
        NativeMenuLanguage::Thai => NativeMenuLabels {
            start_subtitles: "เริ่มคำบรรยาย",
            stop_subtitles: "หยุดคำบรรยาย",
            toggle_devtools: "เปิดเครื่องมือสำหรับนักพัฒนา",
            settings: "การตั้งค่า…",
            quit: "ออกจาก Mimi",
            subtitle_display: "การแสดงคำบรรยาย",
            display_modes: ["คำแปลเท่านั้น", "ต้นฉบับและคำแปล", "ต้นฉบับเท่านั้น"],
            #[cfg(any(target_os = "macos", test))]
            show_in_dock: "แสดงใน Dock",
        },
        NativeMenuLanguage::Russian => NativeMenuLabels {
            start_subtitles: "Запустить субтитры",
            stop_subtitles: "Остановить субтитры",
            toggle_devtools: "Открыть инструменты разработчика",
            settings: "Настройки…",
            quit: "Выйти из Mimi",
            subtitle_display: "Отображение субтитров",
            display_modes: ["Только перевод", "Оригинал и перевод", "Только оригинал"],
            #[cfg(any(target_os = "macos", test))]
            show_in_dock: "Показывать в Dock",
        },
        NativeMenuLanguage::English => NativeMenuLabels {
            start_subtitles: "Start Subtitles",
            stop_subtitles: "Stop Subtitles",
            toggle_devtools: "Open DevTools",
            settings: "Settings…",
            quit: "Quit Mimi",
            subtitle_display: "Subtitle Display",
            display_modes: [
                "Translation Only",
                "Original and Translation",
                "Original Only",
            ],
            #[cfg(any(target_os = "macos", test))]
            show_in_dock: "Show in Dock",
        },
    }
}

fn native_session_action_label(labels: NativeMenuLabels, is_active: bool) -> &'static str {
    if is_active {
        labels.stop_subtitles
    } else {
        labels.start_subtitles
    }
}

fn native_tray_session_is_active(status: &str) -> bool {
    matches!(status, "connecting" | "listening" | "stopping")
}

/// Reads the operating-system language code when the preference follows the system.
#[cfg(target_os = "macos")]
fn system_language_code() -> Option<String> {
    use objc2::msg_send;
    use objc2::runtime::AnyClass;
    use objc2_foundation::{NSArray, NSString};
    unsafe {
        let locale_class = AnyClass::get(c"NSLocale")?;
        // The formatting locale's base language drops scripts such as Hant
        // and can differ from the user's preferred interface language.
        let languages: *mut NSArray<NSString> = msg_send![locale_class, preferredLanguages];
        languages
            .as_ref()?
            .firstObject()
            .map(|language| language.to_string())
    }
}

#[cfg(target_os = "windows")]
fn system_language_code() -> Option<String> {
    use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;

    // Windows defines LOCALE_NAME_MAX_LENGTH as 85 UTF-16 code units,
    // including the trailing null terminator.
    let mut buffer = [0_u16; 85];
    let length =
        unsafe { GetUserDefaultLocaleName(buffer.as_mut_ptr(), i32::try_from(buffer.len()).ok()?) };
    if length <= 1 {
        return None;
    }
    String::from_utf16(&buffer[..usize::try_from(length - 1).ok()?]).ok()
}

#[cfg(target_os = "linux")]
fn system_language_code() -> Option<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty())
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn system_language_code() -> Option<String> {
    None
}

/// Tray icon with the same compact native menu for both mouse buttons.
/// Copy follows the saved UI override, falling back to the system language.
fn tray_icon_bytes(is_windows: bool) -> &'static [u8] {
    if is_windows {
        // Other desktops do not implement macOS template-image recolouring. Use
        // the branded full-colour icon so it remains visible on dark taskbars.
        include_bytes!("../../icons/32x32.png")
    } else {
        include_bytes!("../../icons/tray-template.png")
    }
}

pub(crate) fn setup_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let (override_language, session_is_active) = app
        .try_state::<AppState>()
        .map(|state| {
            (
                state.settings.preferences().ui_language,
                native_tray_session_is_active(&state.session.status_kind()),
            )
        })
        .unwrap_or((None, false));
    let system_language = system_language_code();
    let labels = native_menu_labels(effective_native_menu_language(
        override_language.as_deref(),
        system_language.as_deref(),
    ));

    let session_action = MenuItemBuilder::with_id(
        "live-subtitles",
        native_session_action_label(labels, session_is_active),
    )
    .build(app)?;

    let current_mode = app
        .try_state::<AppState>()
        .map(|state| state.settings.preferences().subtitle_display_mode)
        .unwrap_or_default();
    let translation_item =
        CheckMenuItemBuilder::with_id("display-translation", labels.display_modes[0])
            .checked(current_mode == SubtitleDisplayMode::Translation)
            .build(app)?;
    let bilingual_item =
        CheckMenuItemBuilder::with_id("display-bilingual", labels.display_modes[1])
            .checked(current_mode == SubtitleDisplayMode::Bilingual)
            .build(app)?;
    let original_item = CheckMenuItemBuilder::with_id("display-original", labels.display_modes[2])
        .checked(current_mode == SubtitleDisplayMode::Original)
        .build(app)?;
    let display_modes = [translation_item, bilingual_item, original_item];
    let subtitle_display = SubmenuBuilder::new(app, labels.subtitle_display)
        .item(&display_modes[0])
        .item(&display_modes[1])
        .item(&display_modes[2])
        .build()?;
    let mut menu_builder = MenuBuilder::new(app)
        .item(&session_action)
        .item(&subtitle_display)
        .separator();

    // Dev builds expose a manual "open inspector" action instead of opening
    // the WebView devtools automatically, so the user decides when to look.
    let toggle_devtools = (cfg!(any(debug_assertions, feature = "devtools"))
        && windows::is_dev_build())
    .then(|| MenuItemBuilder::with_id("toggle-devtools", labels.toggle_devtools).build(app))
    .transpose()?;
    if let Some(item) = &toggle_devtools {
        menu_builder = menu_builder.item(item);
    }

    #[cfg(target_os = "macos")]
    let show_in_dock = {
        let checked = app
            .try_state::<AppState>()
            .is_some_and(|state| state.settings.preferences().show_in_dock);
        let item = CheckMenuItemBuilder::with_id("show-in-dock", labels.show_in_dock)
            .checked(checked)
            .build(app)?;
        menu_builder = menu_builder.item(&item);
        item
    };

    let settings_item = MenuItemBuilder::with_id("settings", labels.settings).build(app)?;
    let quit_item = MenuItemBuilder::with_id("quit", labels.quit).build(app)?;

    let menu = menu_builder.item(&settings_item).item(&quit_item).build()?;
    // Menu-bar icon: a monochrome waveform template (like the original app's
    // `ear.badge.waveform` SF Symbol). Template icons are rendered by macOS at
    // the native menu-bar resolution — crisp at any size and adapting to the
    // light/dark menu bar — unlike the character squircle, whose fine detail
    // turned into a blurry blob at ~18pt.
    let icon = tauri::image::Image::from_bytes(tray_icon_bytes(!cfg!(target_os = "macos")))
        .ok()
        .or_else(|| app.default_window_icon().cloned())
        .expect("the tray icon is bundled");

    let _tray = TrayIconBuilder::with_id("mimi-tray")
        .icon(icon)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip(windows::dev_title("Mimi"))
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(move |app, event| {
            let Some(state) = app.try_state::<AppState>() else {
                return;
            };
            match event.id().as_ref() {
                "live-subtitles" => {
                    let session = Arc::clone(&state.session);
                    tauri::async_runtime::spawn(async move {
                        if session.is_active() {
                            session.stop().await;
                        } else {
                            let _ = session.start().await;
                        }
                    });
                }
                "display-translation" | "display-bilingual" | "display-original" => {
                    let mode = match event.id().as_ref() {
                        "display-bilingual" => SubtitleDisplayMode::Bilingual,
                        "display-original" => SubtitleDisplayMode::Original,
                        _ => SubtitleDisplayMode::Translation,
                    };
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        if commands::settings::set_subtitle_display_mode(&app, Some(mode))
                            .await
                            .is_err()
                        {
                            refresh_native_tray_language(&app);
                            tracing::warn!(
                                "subtitle display setting failed label=settings_unavailable"
                            );
                        }
                    });
                }
                "toggle-devtools" => {
                    // Manual inspector toggle (dev builds only). Open the
                    // overlay's WebView devtools so the user decides when to
                    // inspect, instead of auto-opening at startup.
                    #[cfg(any(debug_assertions, feature = "devtools"))]
                    {
                        if let Some(window) = app.get_webview_window("overlay") {
                            window.open_devtools();
                        }
                    }
                }
                #[cfg(target_os = "macos")]
                "show-in-dock" => {
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        if commands::settings::toggle_dock_visibility(app.clone())
                            .await
                            .is_err()
                        {
                            // Restore the checkmark from the saved preference
                            // when native application or persistence fails.
                            refresh_native_tray_language(&app);
                            tracing::warn!(
                                "Dock visibility setting failed label=settings_unavailable"
                            );
                        }
                    });
                }
                id if is_settings_menu_event(id) => {
                    // The shared command also dismisses the always-on-top
                    // control panel so neither surface obscures Settings.
                    let _ = commands::app::app_show_settings(app.clone(), None);
                }
                id if is_normal_quit_menu_event(id) => {
                    let session = Arc::clone(&state.session);
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        if commands::app::quit_application(app.clone(), session)
                            .await
                            .is_err()
                        {
                            // archive_state exposes the existing, content-free
                            // historySaveError; the export page explains why
                            // quitting failed and keeps its retry reachable.
                            let _ = commands::app::app_show_settings(
                                app,
                                Some(commands::SettingsNavigationTarget::Export),
                            );
                        }
                    });
                }
                _ => {}
            }
        })
        .build(app)?;

    app.manage(NativeTrayMenuItems {
        session_action,
        toggle_devtools,
        settings: settings_item,
        quit: quit_item,
        subtitle_display,
        display_modes,
        #[cfg(target_os = "macos")]
        show_in_dock,
    });

    // Session broadcasts already cover every start/stop path (native menu,
    // settings, shortcut, recovery). Update the native action
    // only when activity changes instead of polling all menu state.
    let app_for_session = app.clone();
    let session_was_active = AtomicBool::new(session_is_active);
    app.listen("session-state", move |_| {
        let Some(state) = app_for_session.try_state::<AppState>() else {
            return;
        };
        let is_active = native_tray_session_is_active(&state.session.status_kind());
        if session_was_active.swap(is_active, Ordering::Relaxed) != is_active {
            refresh_native_tray_session_action(&app_for_session, is_active);
        }
    });

    Ok(())
}

pub(crate) fn refresh_native_tray_language(app: &tauri::AppHandle) {
    let (override_language, session_is_active) = app
        .try_state::<AppState>()
        .map(|state| {
            (
                state.settings.preferences().ui_language,
                native_tray_session_is_active(&state.session.status_kind()),
            )
        })
        .unwrap_or((None, false));
    let system_language = system_language_code();
    let labels = native_menu_labels(effective_native_menu_language(
        override_language.as_deref(),
        system_language.as_deref(),
    ));
    #[cfg(target_os = "macos")]
    if let Some(items) = app.try_state::<NativeApplicationMenuItems>() {
        let _ = items.settings.set_text(labels.settings);
        let _ = items.quit.set_text(labels.quit);
    }
    let Some(items) = app.try_state::<NativeTrayMenuItems>() else {
        return;
    };

    let _ = items
        .session_action
        .set_text(native_session_action_label(labels, session_is_active));
    if let Some(item) = &items.toggle_devtools {
        let _ = item.set_text(labels.toggle_devtools);
    }
    let _ = items.settings.set_text(labels.settings);
    let _ = items.quit.set_text(labels.quit);
    let _ = items.subtitle_display.set_text(labels.subtitle_display);
    #[cfg(target_os = "macos")]
    {
        let _ = items.show_in_dock.set_text(labels.show_in_dock);
        let checked = app
            .try_state::<AppState>()
            .is_some_and(|state| state.settings.preferences().show_in_dock);
        let _ = items.show_in_dock.set_checked(checked);
    }
    let current_mode = app
        .try_state::<AppState>()
        .map(|state| state.settings.preferences().subtitle_display_mode)
        .unwrap_or_default();
    for ((item, label), mode) in items.display_modes.iter().zip(labels.display_modes).zip([
        SubtitleDisplayMode::Translation,
        SubtitleDisplayMode::Bilingual,
        SubtitleDisplayMode::Original,
    ]) {
        let _ = item.set_text(label);
        let _ = item.set_checked(mode == current_mode);
    }
}

fn refresh_native_tray_session_action(app: &tauri::AppHandle, is_active: bool) {
    let override_language = app
        .try_state::<AppState>()
        .and_then(|state| state.settings.preferences().ui_language);
    let system_language = system_language_code();
    let labels = native_menu_labels(effective_native_menu_language(
        override_language.as_deref(),
        system_language.as_deref(),
    ));
    if let Some(items) = app.try_state::<NativeTrayMenuItems>() {
        let _ = items
            .session_action
            .set_text(native_session_action_label(labels, is_active));
    }
}

#[cfg(test)]
mod tests;
