//! mimi — live translated subtitles for anything playing on your device.
//! Tauri v2 shell wiring: plugins, tray, global shortcut, windows, and state.

pub(crate) use mimi_runtime::pipeline_log;

mod apple_speech;
mod apple_speech_support;
mod apple_translation;
mod apple_translation_support;
mod audio;
#[cfg(test)]
mod audio3_benchmark;
mod clients;
mod commands;
mod core;
mod desktop_shortcuts;
#[cfg(any(test, feature = "development-debugger"))]
mod development_audio;
#[cfg(not(any(test, feature = "development-debugger")))]
#[path = "development_audio_disabled.rs"]
mod development_audio;
#[cfg(any(test, feature = "development-debugger"))]
mod development_content;
#[cfg(not(any(test, feature = "development-debugger")))]
#[path = "development_content_disabled.rs"]
mod development_content;
#[cfg(any(test, feature = "development-debugger"))]
mod development_debugger;
mod fonts;
#[cfg(target_os = "linux")]
mod linux_startup;
#[cfg(any(target_os = "macos", test))]
mod mac_dock;
#[cfg(target_os = "macos")]
mod mac_native_quit;
mod session_export;
mod session_history;
mod session_manager;
mod settings_store;
mod windows;
#[cfg(target_os = "windows")]
mod windows_startup;

use commands::AppState;
use core::models::SubtitleDisplayMode;
use session_manager::SessionManager;
use settings_store::{SettingsStore, DEVELOPMENT_APPLICATION_IDENTIFIER};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::menu::{
    CheckMenuItem, CheckMenuItemBuilder, MenuBuilder, MenuItem, MenuItemBuilder, Submenu,
    SubmenuBuilder,
};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Listener, Manager, WindowEvent};

/// Runs the mimi Tauri application.
pub fn run() {
    // Xlib threading must be initialized before GTK opens its display.
    #[cfg(target_os = "linux")]
    linux_startup::initialize_x11_threads().expect("Linux window-system initialization failed");

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let context = tauri::generate_context!();
    #[cfg(all(target_os = "macos", feature = "development-debugger"))]
    if let Some(options) = audio::macos_smoke::options_from_args() {
        audio::macos_smoke::run(context, options);
        return;
    }
    #[cfg(target_os = "windows")]
    let startup_gate = windows_startup::StartupGate::acquire(context.config().identifier.as_str())
        .unwrap_or_else(|label| panic!("mimi startup gate failed: {label}"));
    #[cfg(target_os = "windows")]
    if startup_gate
        .handoff_if_running(context.config().identifier.as_str())
        .unwrap_or_else(|label| panic!("mimi early handoff failed: {label}"))
    {
        // Returning drops the gate on its owner thread. No Tauri runtime,
        // WebView, provider, or settings store is created by a secondary.
        return;
    }

    let builder = tauri::Builder::default();
    // This must stay first so a secondary Windows launch exits before any
    // other plugin, renderer, tray, or settings store is initialized.
    #[cfg(target_os = "windows")]
    let builder = builder.plugin(windows_startup::single_instance_plugin());
    #[cfg(target_os = "linux")]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
        desktop_shortcuts::dispatch_linux_launch(app, &args);
    }));
    let builder = builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_positioner::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build());
    // X11 grabs cannot act as desktop shortcuts in a Wayland session.
    let builder = if desktop_shortcuts::uses_system_shortcuts() {
        builder
    } else {
        builder.plugin(tauri_plugin_global_shortcut::Builder::new().build())
    };
    #[cfg(target_os = "macos")]
    let builder = builder.plugin(tauri_nspanel::init());

    builder
        .setup(move |app| {
            tracing::info!("mimi starting");

            // Dev-build marker: the settings window is created from the
            // static config title, so adjust it at runtime so the dev binary
            // is distinguishable from the installed release app.
            if windows::is_dev_build() {
                if let Some(window) = app.get_webview_window("settings") {
                    let _ = window.set_title(&windows::dev_title("mimi 设置"));
                }
            }

            let app_handle = app.handle().clone();
            if windows::is_dev_build()
                && app.config().identifier.as_str() != DEVELOPMENT_APPLICATION_IDENTIFIER
            {
                return Err("development builds require the isolated Tauri identifier".into());
            }
            let is_ui_test = std::env::var("MIMI_UI_TEST").as_deref() == Ok("1");
            #[cfg(any(test, feature = "development-debugger"))]
            development_debugger::initialize(&app_handle);
            if is_ui_test {
                if let Some(window) = app.get_webview_window("settings") {
                    let _ = window.set_title("mimi UI test settings");
                }
            }
            let settings = Arc::new(if is_ui_test {
                match std::env::var_os("MIMI_UI_TEST_PREFERENCES_DIR") {
                    Some(directory) => SettingsStore::load_ui_test_preferences(directory.into())?,
                    None => SettingsStore::load(Default::default(), true, &app.config().identifier),
                }
            } else {
                SettingsStore::load(
                    app.path().app_config_dir().unwrap_or_default(),
                    false,
                    &app.config().identifier,
                )
            });
            // Apply the saved global Dock preference at startup. Missing
            // preferences show Mimi in the Dock, independently of credentials.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(crate::mac_dock::policy(settings.preferences().show_in_dock));
            // A deterministic standard-overlay fixture is useful for native
            // window-level checks. It changes only the in-memory UI-test
            // snapshot; `SettingsStore` never persists UI-test writes.
            if is_ui_test && std::env::var("MIMI_UI_TEST_STANDARD_OVERLAY").as_deref() == Ok("1") {
                let _ = settings.save_preferences(|preferences| {
                    preferences.subtitle_blends_with_background = false;
                    preferences.overlay_locked = false;
                });
            }
            let session = SessionManager::new(app_handle.clone(), Arc::clone(&settings));

            let overlay = Arc::new(std::sync::Mutex::new(windows::OverlayState::load(
                &app_handle,
                &settings,
            )));
            app.manage(AppState {
                settings: Arc::clone(&settings),
                session: Arc::clone(&session),
                overlay: Arc::clone(&overlay),
            });
            app.manage(windows::OverlayControlState::default());
            app.manage(windows::OverlayPresentationState::default());
            windows::OverlayWindowManager::ensure_overlay(&app_handle, &overlay);
            #[cfg(target_os = "macos")]
            windows::install_active_space_observer(&app_handle, &overlay, &settings);
            let preferences = settings.preferences();
            windows::OverlayWindowManager::update_locked(
                &app_handle,
                preferences.overlay_locked || preferences.subtitle_blends_with_background,
            );
            windows::TrayPanelManager::ensure(&app_handle);
            windows::OverlayControlWindowManager::ensure(&app_handle);
            #[cfg(target_os = "windows")]
            app.manage(windows::install_windows_workspace_follower(&app_handle));

            setup_tray(&app_handle)?;
            #[cfg(target_os = "macos")]
            setup_application_menu(&app_handle)?;
            #[cfg(target_os = "macos")]
            mac_native_quit::install(&app_handle)?;
            setup_global_shortcuts(&app_handle, Arc::clone(&session))?;
            #[cfg(target_os = "linux")]
            {
                let args = std::env::args().collect::<Vec<_>>();
                if desktop_shortcuts::DesktopAction::from_args(&args).is_some() {
                    desktop_shortcuts::dispatch_linux_launch(&app_handle, &args);
                }
            }

            // Test-only probe: `SessionManager` handles UI-test starts as a
            // synthetic local state transition. It never reads the keychain,
            // opens a network connection, or starts system-audio capture.
            if is_ui_test && std::env::var("MIMI_AUTO_START").as_deref() == Ok("1") {
                let session_for_probe = Arc::clone(&session);
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    let _ = session_for_probe.start().await;
                });
            }

            // Release queued cold launches only after setup returns to the
            // Windows event loop. A secondary uses a bounded synchronous,
            // payload-free activation message; releasing earlier can block it
            // while this main thread is still creating WebViews, tray state,
            // and shortcuts.
            #[cfg(target_os = "windows")]
            {
                let app_handle_for_gate = app.handle().clone();
                std::thread::spawn(move || {
                    if app_handle_for_gate
                        .run_on_main_thread(move || drop(startup_gate))
                        .is_err()
                    {
                        // The event loop cannot become a usable primary, and
                        // this mutex is owned by its main thread. Abort so the
                        // kernel abandons it for the next launch.
                        std::process::abort();
                    }
                });
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            let app = window.app_handle();
            match event {
                #[cfg(target_os = "macos")]
                WindowEvent::Destroyed if window.label() == "overlay" => {
                    windows::remove_overlay_pointer_tracking(app);
                }
                // The overlay geometry manager folds the final frame in after
                // a debounce; transient states (control panel, collapse
                // animation steps) are never persisted.
                WindowEvent::Moved(_)
                | WindowEvent::Resized(_)
                | WindowEvent::ScaleFactorChanged { .. }
                    if window.label() == "overlay" =>
                {
                    if let Some(state) = app.try_state::<AppState>() {
                        windows::OverlayWindowManager::on_geometry_event(
                            app,
                            &state.overlay,
                            &state.settings,
                        );
                    }
                    // AppKit moves an NSWindow child in the same native drag
                    // transaction as its parent. Repositioning it again from
                    // the later Moved event makes the control visibly trail.
                    // Windows owned windows and Linux transient windows still
                    // need explicit live following.
                    #[cfg(not(target_os = "macos"))]
                    windows::OverlayControlWindowManager::follow_overlay(app);
                }
                WindowEvent::Focused(false) if window.label() == "tray-panel" => {
                    windows::TrayPanelManager::hide(app);
                }
                // An expanded control panel returns to its compact island on
                // focus loss. The island itself remains available even while
                // the subtitle canvas is click-through.
                WindowEvent::Focused(false) if window.label() == "overlay-control" => {
                    windows::OverlayControlWindowManager::schedule_dismiss(app);
                }
                WindowEvent::Focused(true) if window.label() == "overlay-control" => {
                    windows::OverlayControlWindowManager::cancel_scheduled_dismiss(app);
                }
                WindowEvent::CloseRequested { api, .. } if window.label() == "settings" => {
                    // AppIndicator can accept an icon even when the desktop
                    // has no tray host. Linux users minimize to keep running;
                    // closing Settings must not strand an invisible process.
                    if cfg!(target_os = "linux") {
                        if let Some(state) = app.try_state::<AppState>() {
                            let session = Arc::clone(&state.session);
                            let app = app.clone();
                            tauri::async_runtime::spawn(async move {
                                let _ = commands::quit_application(app, session).await;
                            });
                        }
                        api.prevent_close();
                        return;
                    }
                    // Hiding instead of closing keeps the window alive so
                    // the tray or a repeated launch can restore it instantly.
                    // Windows users may keep tray icons in the notification
                    // overflow, so explicitly keep the native icon visible.
                    api.prevent_close();
                    let tray_ready = if let Some(tray) = app.tray_by_id("mimi-tray") {
                        if let Err(error) = tray.set_visible(true) {
                            tracing::warn!(
                                error = %error,
                                "settings close could not keep tray icon visible"
                            );
                            false
                        } else {
                            record_ui_test_tray_visible();
                            true
                        }
                    } else {
                        tracing::warn!("settings close could not find tray icon");
                        false
                    };
                    // Never strand the user with neither Settings nor a tray
                    // entry. `prevent_close` keeps this window visible when
                    // the native tray handle cannot be confirmed.
                    if !tray_ready {
                        return;
                    }
                    if let Err(error) = window.hide() {
                        tracing::warn!(error = %error, "settings close could not hide window");
                        // A later tray/relaunch activation recreates a missing
                        // settings window, avoiding an uncloseable stale shell.
                        if let Err(error) = window.destroy() {
                            tracing::warn!(
                                error = %error,
                                "settings close could not destroy stale window"
                            );
                        }
                    }
                }
                WindowEvent::CloseRequested { api, .. }
                    if window.label() == "overlay"
                        || window.label() == "overlay-control"
                        || window.label() == "tray-panel" =>
                {
                    api.prevent_close();
                    #[cfg(target_os = "macos")]
                    if window.label() == "overlay" {
                        windows::clear_overlay_pointer_hover(app);
                    }
                    if let Err(error) = window.hide() {
                        tracing::warn!(
                            window_label = window.label(),
                            error = %error,
                            "auxiliary window close could not hide window"
                        );
                    }
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::settings_get,
            commands::get_apple_speech_support,
            commands::prepare_apple_speech_language,
            commands::get_apple_translation_support,
            commands::get_apple_translation_status,
            commands::prepare_apple_translation_languages,
            fonts::installed_font_families,
            commands::windows_audio_status,
            commands::audio_census,
            commands::support_diagnostics,
            #[cfg(any(test, feature = "development-debugger"))]
            development_debugger::development_debug_snapshot,
            #[cfg(any(test, feature = "development-debugger"))]
            development_debugger::development_debug_start,
            #[cfg(any(test, feature = "development-debugger"))]
            development_debugger::development_debug_stop,
            #[cfg(any(test, feature = "development-debugger"))]
            development_debugger::development_debug_observe,
            #[cfg(any(test, feature = "development-debugger"))]
            development_debugger::development_debug_flush_ack,
            #[cfg(any(test, feature = "development-debugger"))]
            development_debugger::development_debug_export,
            #[cfg(any(test, feature = "development-debugger"))]
            development_debugger::development_debug_audio,
            #[cfg(any(test, feature = "development-debugger"))]
            development_debugger::development_debug_replay,
            #[cfg(any(test, feature = "development-debugger"))]
            development_debugger::development_debug_cases,
            #[cfg(any(test, feature = "development-debugger"))]
            development_debugger::development_debug_open_case,
            #[cfg(any(test, feature = "development-debugger"))]
            development_debugger::development_debug_private_events,
            #[cfg(any(test, feature = "development-debugger"))]
            development_debugger::development_debug_trace_events,
            commands::app_open_support_issue,
            commands::capture_status,
            commands::audio_applications,
            commands::app_is_ui_test,
            commands::app_ui_test_frontend_ready,
            commands::app_is_portable,
            commands::app_is_linux_package,
            desktop_shortcuts::app_desktop_shortcut_commands,
            commands::app_open_releases,
            commands::open_tencent_setup_page,
            commands::settings_save,
            commands::profile_create,
            commands::profile_update,
            commands::profile_select,
            commands::profile_delete,
            commands::profile_save_credentials,
            commands::profile_test_connection,
            commands::profile_delete_api_key,
            commands::profile_reveal_credential,
            commands::profile_credential_editor_state,
            crate::session_export::session_archive_state,
            crate::session_export::session_transcript_page,
            crate::session_export::session_history_list,
            crate::session_export::session_history_page,
            crate::session_export::session_history_audio,
            crate::session_export::session_history_delete,
            crate::session_export::session_archive_clear,
            crate::session_export::session_export,
            commands::session_start,
            commands::session_stop,
            commands::session_toggle_paused,
            commands::session_clear_subtitles,
            commands::session_switch_source_language,
            commands::session_switch_target_language,
            commands::session_switch_audio_input,
            commands::session_switch_system_audio_target,
            commands::session_switch_translation_mode,
            commands::overlay_set_collapsed,
            commands::overlay_set_locked,
            commands::overlay_set_pointer_cursor,
            commands::overlay_show,
            commands::overlay_move_start,
            commands::overlay_popover_toggle,
            commands::overlay_popover_hide,
            commands::overlay_control_state,
            commands::overlay_control_set_panel_height,
            commands::overlay_control_set_island_width,
            commands::session_get_state,
            commands::resize_start,
            commands::resize_move,
            commands::resize_end,
            commands::tray_panel_hide,
            commands::app_show_settings,
            commands::app_quit,
        ])
        .build(context)
        .expect("error while building tauri application")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if matches!(event, tauri::RunEvent::Exit) {
                mac_native_quit::remove();
                windows::remove_overlay_pointer_tracking(app);
            }
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                if app.state::<AppState>().settings.preferences().show_in_dock {
                    let _ = commands::app_show_settings(app.clone(), None);
                }
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (app, event);
        });
}

fn record_ui_test_tray_visible() {
    if std::env::var("MIMI_UI_TEST").as_deref() != Ok("1") {
        return;
    }
    let Some(path) = std::env::var_os("MIMI_UI_TEST_TRAY_VISIBLE_FILE") else {
        return;
    };
    if let Err(error) = std::fs::write(path, b"visible") {
        tracing::warn!(error = %error, "could not write UI-test tray visibility marker");
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeMenuLanguage {
    Chinese,
    TraditionalChinese,
    English,
    Japanese,
    German,
    French,
    Korean,
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
fn setup_application_menu(app: &tauri::AppHandle) -> tauri::Result<()> {
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
        | Some("ko") => override_language,
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
            quit: "退出 mimi",
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
            quit: "結束 mimi",
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
            quit: "mimiを終了",
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
            quit: "mimi beenden",
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
            quit: "Quitter mimi",
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
            quit: "mimi 종료",
            subtitle_display: "자막 표시",
            display_modes: ["번역만", "원문과 번역", "원문만"],
            #[cfg(any(target_os = "macos", test))]
            show_in_dock: "Dock에 표시",
        },
        NativeMenuLanguage::English => NativeMenuLabels {
            start_subtitles: "Start Subtitles",
            stop_subtitles: "Stop Subtitles",
            toggle_devtools: "Open DevTools",
            settings: "Settings…",
            quit: "Quit mimi",
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
    use objc2::runtime::{AnyClass, AnyObject};
    use std::ffi::{c_char, CStr};
    unsafe {
        let locale_class = AnyClass::get(c"NSLocale")?;
        let current: *mut AnyObject = msg_send![locale_class, currentLocale];
        if current.is_null() {
            return None;
        }
        let code: *mut AnyObject = msg_send![current, languageCode];
        if code.is_null() {
            return None;
        }
        let ptr: *const c_char = msg_send![code, UTF8String];
        if ptr.is_null() {
            return None;
        }
        Some(CStr::from_ptr(ptr).to_string_lossy().into_owned())
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

/// Tray icon with a compact native menu and a left-click popup control panel.
/// Copy follows the saved UI override, falling back to the system language.
fn tray_icon_bytes(is_windows: bool) -> &'static [u8] {
    if is_windows {
        // Other desktops do not implement macOS template-image recolouring. Use
        // the branded full-colour icon so it remains visible on dark taskbars.
        include_bytes!("../icons/32x32.png")
    } else {
        include_bytes!("../icons/tray-template.png")
    }
}

fn setup_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
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
        .tooltip(windows::dev_title("mimi"))
        .menu(&menu)
        .show_menu_on_left_click(false)
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
                        if commands::set_subtitle_display_mode(&app, Some(mode))
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
                        if commands::toggle_dock_visibility(app.clone()).await.is_err() {
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
                    let _ = commands::app_show_settings(app.clone(), None);
                }
                id if is_normal_quit_menu_event(id) => {
                    let session = Arc::clone(&state.session);
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        if commands::quit_application(app.clone(), session)
                            .await
                            .is_err()
                        {
                            // archive_state exposes the existing, content-free
                            // historySaveError; the export page explains why
                            // quitting failed and keeps its retry reachable.
                            let _ = commands::app_show_settings(
                                app,
                                Some(commands::SettingsNavigationTarget::Export),
                            );
                        }
                    });
                }
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, event| {
            // Preserve the positioner fallback state as well as passing the
            // click rectangle to the edge-aware primary placement path.
            tauri_plugin_positioner::on_tray_event(tray.app_handle(), &event);
            if let TrayIconEvent::Click {
                rect,
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                windows::TrayPanelManager::toggle(app, &rect);
                // Refresh the panel's snapshots so its pickers and check
                // states always match the current session, even if its
                // webview missed events while hidden.
                if let Some(state) = app.try_state::<AppState>() {
                    state.session.publish_settings();
                    state.session.publish_state();
                }
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
    // tray panel, settings, shortcut, recovery). Update the native action
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

/// Registers the global session, Immersive Mode, and subtitle display shortcuts. Each action
/// owns an independent 500ms debounce so presentation switching never blocks
/// a session lifecycle action (or vice versa).
fn setup_global_shortcuts(
    app: &tauri::AppHandle,
    session: Arc<SessionManager>,
) -> tauri::Result<()> {
    use tauri_plugin_global_shortcut::{
        Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState,
    };

    if desktop_shortcuts::uses_system_shortcuts() {
        tracing::info!("Wayland shortcuts are configured in desktop settings");
        return Ok(());
    }

    // macOS: Cmd+Shift (SUPER is the Command key); Windows: Ctrl+Shift.
    #[cfg(target_os = "macos")]
    let modifiers = Modifiers::SUPER | Modifiers::SHIFT;
    #[cfg(not(target_os = "macos"))]
    let modifiers = Modifiers::CONTROL | Modifiers::SHIFT;
    let session_shortcut = Shortcut::new(Some(modifiers), Code::Space);
    let immersive_shortcut = Shortcut::new(Some(modifiers), Code::KeyM);
    let display_shortcut = Shortcut::new(Some(modifiers), Code::KeyB);

    let last_trigger = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let session_for_handler = Arc::clone(&session);

    // Register the global shortcut. A failure must not abort startup: another
    // app (e.g. a second mimi instance) may already own the combination, in
    // which case the OS keeps delivering it to that app.
    let session_register =
        app.global_shortcut()
            .on_shortcut(session_shortcut, move |_app, _shortcut, event| {
                if event.state() != ShortcutState::Pressed {
                    return;
                }
                // Debounce repeated global-shortcut presses.
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                let previous = last_trigger.load(std::sync::atomic::Ordering::SeqCst);
                if now_ms.saturating_sub(previous) < 500 {
                    return;
                }
                last_trigger.store(now_ms, std::sync::atomic::Ordering::SeqCst);
                let session = Arc::clone(&session_for_handler);
                tauri::async_runtime::spawn(async move {
                    let status = session.status_kind();
                    if status == "connecting" || status == "stopping" {
                        return;
                    }
                    if session.is_active() {
                        session.stop().await;
                    } else {
                        let _ = session.start().await;
                    }
                });
            });
    match session_register {
        Ok(()) => tracing::info!("global session shortcut registered"),
        Err(error) => tracing::warn!(
            "global session shortcut could not be registered: {error} \
             (another app may already own the start/stop combination)"
        ),
    }

    let immersive_last_trigger = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let immersive_register =
        app.global_shortcut()
            .on_shortcut(immersive_shortcut, move |app, _shortcut, event| {
                if event.state() != ShortcutState::Pressed {
                    return;
                }
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                let previous = immersive_last_trigger.load(std::sync::atomic::Ordering::SeqCst);
                if now_ms.saturating_sub(previous) < 500 {
                    return;
                }
                immersive_last_trigger.store(now_ms, std::sync::atomic::Ordering::SeqCst);
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = commands::toggle_immersive_mode(&app).await {
                        tracing::warn!(
                            "immersive shortcut failed label=settings_unavailable error={error}"
                        );
                    }
                });
            });
    match immersive_register {
        Ok(()) => tracing::info!("global immersive shortcut registered"),
        Err(error) => tracing::warn!(
            "global immersive shortcut could not be registered: {error} \
             (another app may already own the combination)"
        ),
    }
    let display_last_trigger = std::sync::atomic::AtomicU64::new(0);
    let display_register =
        app.global_shortcut()
            .on_shortcut(display_shortcut, move |app, _, event| {
                if event.state() != ShortcutState::Pressed {
                    return;
                }
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                let previous = display_last_trigger.load(Ordering::SeqCst);
                if now_ms.saturating_sub(previous) < 500 {
                    return;
                }
                display_last_trigger.store(now_ms, Ordering::SeqCst);
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if commands::set_subtitle_display_mode(&app, None)
                        .await
                        .is_err()
                    {
                        tracing::warn!(
                            "subtitle display shortcut failed label=settings_unavailable"
                        );
                    }
                });
            });
    match display_register {
        Ok(()) => tracing::info!("global subtitle display shortcut registered"),
        Err(error) => {
            tracing::warn!("global subtitle display shortcut could not be registered: {error}")
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
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
                effective_native_menu_language(Some("system"), Some(locale)),
                NativeMenuLanguage::TraditionalChinese,
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
                effective_native_menu_language(None, Some(locale)),
                NativeMenuLanguage::Chinese,
                "{locale}"
            );
        }
        assert_eq!(
            effective_native_menu_language(Some("zh-TW"), Some("zh-CN")),
            NativeMenuLanguage::TraditionalChinese
        );
        assert_eq!(
            effective_native_menu_language(Some("zh"), Some("zh-Hant")),
            NativeMenuLanguage::Chinese
        );
        let labels = native_menu_labels(NativeMenuLanguage::TraditionalChinese);
        assert_eq!(labels.start_subtitles, "開始字幕");
        assert_eq!(labels.stop_subtitles, "停止字幕");
        assert_eq!(labels.toggle_devtools, "開啟開發者工具");
        assert_eq!(labels.settings, "設定…");
        assert_eq!(labels.quit, "結束 mimi");
        assert_eq!(labels.subtitle_display, "字幕顯示");
        assert_eq!(labels.display_modes, ["僅譯文", "原文與譯文", "僅原文"]);
        assert_eq!(labels.show_in_dock, "在 Dock 中顯示");
    }

    #[test]
    fn added_interface_languages_localize_native_menus() {
        for (code, region, language, label) in [
            ("de", "de-DE", NativeMenuLanguage::German, "Einstellungen…"),
            ("fr", "fr-CA", NativeMenuLanguage::French, "Réglages…"),
            ("ko", "ko-KR", NativeMenuLanguage::Korean, "설정…"),
        ] {
            assert_eq!(
                effective_native_menu_language(Some(code), Some("en-US")),
                language
            );
            assert_eq!(
                effective_native_menu_language(Some("system"), Some(region)),
                language
            );
            assert_eq!(native_menu_labels(language).settings, label);
            assert_ne!(
                native_menu_labels(language).quit,
                native_menu_labels(NativeMenuLanguage::English).quit
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
            (NativeMenuLanguage::Chinese, "设置…"),
            (NativeMenuLanguage::English, "Settings…"),
            (NativeMenuLanguage::Japanese, "設定…"),
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
            (NativeMenuLanguage::Chinese, "退出 mimi"),
            (NativeMenuLanguage::English, "Quit mimi"),
            (NativeMenuLanguage::Japanese, "mimiを終了"),
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
            (NativeMenuLanguage::Chinese, "开始字幕", "停止字幕"),
            (
                NativeMenuLanguage::English,
                "Start Subtitles",
                "Stop Subtitles",
            ),
            (NativeMenuLanguage::Japanese, "字幕を開始", "字幕を停止"),
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
            effective_native_menu_language(Some("ja"), Some("zh-CN")),
            NativeMenuLanguage::Japanese
        );
        assert_eq!(
            effective_native_menu_language(None, Some("zh-Hans")),
            NativeMenuLanguage::Chinese
        );
        assert_eq!(
            effective_native_menu_language(Some("system"), Some("ja-JP")),
            NativeMenuLanguage::Japanese
        );
        assert_eq!(
            effective_native_menu_language(None, Some("en-US")),
            NativeMenuLanguage::English
        );
        assert_eq!(
            native_menu_labels(NativeMenuLanguage::Japanese).settings,
            "設定…"
        );
    }

    #[test]
    fn native_dock_visibility_labels_match_the_existing_frontend_copy() {
        let frontend_i18n = include_str!("../../src/lib/i18n.ts");
        for language in [
            NativeMenuLanguage::Chinese,
            NativeMenuLanguage::English,
            NativeMenuLanguage::Japanese,
        ] {
            let label = native_menu_labels(language).show_in_dock;
            assert!(frontend_i18n.contains(&format!("showInDock: \"{label}\"")));
        }
    }

    #[test]
    fn development_tauri_config_uses_the_isolated_identifier() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.dev.conf.json")).unwrap();
        assert_eq!(
            config["identifier"].as_str(),
            Some(DEVELOPMENT_APPLICATION_IDENTIFIER)
        );
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
}
