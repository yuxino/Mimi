//! mimi — live translated subtitles for anything playing on your device.
//! Tauri v2 shell wiring: plugins, tray, global shortcut, windows, and state.

pub(crate) use mimi_runtime::pipeline_log;

#[path = "apple/speech.rs"]
mod apple_speech;
#[path = "apple/speech_support.rs"]
mod apple_speech_support;
#[path = "apple/translation.rs"]
mod apple_translation;
#[path = "apple/translation_support.rs"]
mod apple_translation_support;
mod audio;
#[cfg(test)]
#[path = "development/probes/audio3_benchmark.rs"]
mod audio3_benchmark;
mod clients;
mod commands;
mod core;
mod desktop_shortcuts;
#[cfg(any(test, feature = "development-debugger"))]
#[path = "development/audio.rs"]
mod development_audio;
#[cfg(not(any(test, feature = "development-debugger")))]
#[path = "development/audio_disabled.rs"]
mod development_audio;
#[cfg(any(test, feature = "development-debugger"))]
#[path = "development/content.rs"]
mod development_content;
#[cfg(not(any(test, feature = "development-debugger")))]
#[path = "development/content_disabled.rs"]
mod development_content;
#[cfg(any(test, feature = "development-debugger"))]
#[path = "development/debugger.rs"]
mod development_debugger;
mod fonts;
#[cfg(all(test, feature = "development-debugger", target_os = "macos"))]
#[path = "development/probes/live_translate_input_probe.rs"]
mod live_translate_input_probe;
mod platform;
#[cfg(all(test, feature = "development-debugger", target_os = "macos"))]
#[path = "development/probes/provider_public_audio_probe.rs"]
mod provider_public_audio_probe;
mod session_export;
mod session_history;
mod session_manager;
mod settings_store;
mod updater_target;
mod windows;

use commands::AppState;
use session_manager::SessionManager;
use settings_store::{SettingsStore, DEVELOPMENT_APPLICATION_IDENTIFIER};
use std::sync::Arc;
use tauri::{Manager, WindowEvent};

pub(crate) fn app_context() -> tauri::Context<tauri::Wry> {
    tauri::generate_context!()
}

/// Runs the mimi Tauri application.
pub fn run() {
    // Xlib threading must be initialized before GTK opens its display.
    #[cfg(target_os = "linux")]
    platform::linux::startup::initialize_x11_threads()
        .expect("Linux window-system initialization failed");

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let context = app_context();
    #[cfg(all(target_os = "macos", feature = "development-debugger"))]
    if let Some(options) = audio::macos_smoke::options_from_args() {
        audio::macos_smoke::run(context, options);
        return;
    }
    #[cfg(target_os = "windows")]
    let startup_gate =
        platform::windows::startup::StartupGate::acquire(context.config().identifier.as_str())
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
    let builder = builder.plugin(platform::windows::startup::single_instance_plugin());
    #[cfg(target_os = "linux")]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
        desktop_shortcuts::dispatch_linux_launch(app, &args);
    }));
    let builder = builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_process::init())
        .plugin(updater_target::builder().build());
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
                    let _ = window.set_title(&windows::dev_title("Mimi 设置"));
                }
            }

            let app_handle = app.handle().clone();
            if windows::is_dev_build()
                && app.config().identifier.as_str() != DEVELOPMENT_APPLICATION_IDENTIFIER
            {
                return Err("development builds require the isolated Tauri identifier".into());
            }
            let is_ui_test = std::env::var("MIMI_UI_TEST").as_deref() == Ok("1");
            #[cfg(target_os = "windows")]
            if !is_ui_test && !windows::is_dev_build() {
                let version = app.package_info().version.to_string();
                tauri::async_runtime::spawn_blocking(move || {
                    let result = platform::windows::updater_cleanup::cleanup(
                        &std::env::temp_dir(),
                        &version,
                    );
                    if result.removed > 0 || result.failed > 0 {
                        tracing::info!(
                            removed = result.removed,
                            failed = result.failed,
                            "updater staging cleanup"
                        );
                    }
                });
            }
            #[cfg(any(test, feature = "development-debugger"))]
            development_debugger::initialize(&app_handle);
            if is_ui_test {
                if let Some(window) = app.get_webview_window("settings") {
                    // Stable automation title: Linux and Windows smoke tests match it exactly.
                    let _ = window.set_title("mimi UI test settings");
                    if std::env::var("MIMI_UI_TEST_COMPACT_SETTINGS").as_deref() == Ok("1") {
                        window.set_size(tauri::LogicalSize::new(520.0, 560.0))?;
                    }
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
            app.set_activation_policy(platform::macos::dock::policy(
                settings.preferences().show_in_dock,
            ));
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
            app.manage(windows::OverlayControlHost::detect());
            app.manage(windows::OverlayPresentationState::default());
            #[cfg(target_os = "windows")]
            app.manage(windows::install_windows_pointer_tracking(&app_handle));
            windows::OverlayWindowManager::ensure_overlay(&app_handle, &overlay);
            #[cfg(target_os = "macos")]
            windows::install_active_space_observer(&app_handle, &overlay, &settings);
            let preferences = settings.preferences();
            windows::OverlayWindowManager::update_locked(
                &app_handle,
                preferences.overlay_locked || preferences.subtitle_blends_with_background,
            );
            windows::OverlayControlWindowManager::ensure(&app_handle);
            #[cfg(target_os = "windows")]
            app.manage(windows::install_windows_workspace_follower(&app_handle));

            platform::menu::setup_tray(&app_handle)?;
            #[cfg(target_os = "macos")]
            platform::menu::setup_application_menu(&app_handle)?;
            #[cfg(target_os = "macos")]
            platform::macos::native_quit::install(&app_handle)?;
            desktop_shortcuts::setup_global_shortcuts(&app_handle, Arc::clone(&session))?;
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
                #[cfg(any(target_os = "macos", target_os = "windows"))]
                WindowEvent::Destroyed
                    if matches!(window.label(), "overlay" | "overlay-control") =>
                {
                    windows::remove_overlay_pointer_tracking(app, window.label());
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
                // An expanded control panel returns to its compact island on
                // focus loss. Locked and immersive presentations hide controls
                // through the overlay presentation manager.
                WindowEvent::Focused(false) if window.label() == "overlay-control" => {
                    windows::OverlayControlWindowManager::schedule_dismiss(app);
                }
                WindowEvent::Focused(true) if window.label() == "overlay-control" => {
                    windows::OverlayControlWindowManager::cancel_scheduled_dismiss(app);
                }
                WindowEvent::Focused(false)
                    if window.label() == "overlay"
                        && windows::OverlayControlWindowManager::is_embedded(app) =>
                {
                    windows::OverlayControlWindowManager::schedule_dismiss(app);
                }
                WindowEvent::Focused(true)
                    if window.label() == "overlay"
                        && windows::OverlayControlWindowManager::is_embedded(app) =>
                {
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
                                let _ = commands::app::quit_application(app, session).await;
                            });
                        }
                        api.prevent_close();
                        return;
                    }
                    // Hiding instead of closing keeps the window alive so
                    // the tray or a repeated launch can restore it instantly.
                    // The tray is created visible. Windows' tray backend also
                    // re-registers it on TaskbarCreated after Explorer restarts.
                    api.prevent_close();
                    let tray_ready = if let Some(tray) = app.tray_by_id("mimi-tray") {
                        #[cfg(target_os = "windows")]
                        {
                            // Reapplying visibility calls Shell_NotifyIconW
                            // synchronously on this UI thread. It is redundant
                            // (we never hide the tray) and can wait on Explorer.
                            let _ = tray;
                            record_ui_test_tray_visible();
                            true
                        }
                        #[cfg(not(target_os = "windows"))]
                        {
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
                    if window.label() == "overlay" || window.label() == "overlay-control" =>
                {
                    api.prevent_close();
                    #[cfg(any(target_os = "macos", target_os = "windows"))]
                    windows::remove_overlay_pointer_tracking(app, window.label());
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
            commands::settings::settings_get,
            commands::apple_resources::get_apple_speech_support,
            commands::apple_resources::prepare_apple_speech_language,
            commands::apple_resources::get_apple_translation_support,
            commands::apple_resources::get_apple_translation_status,
            commands::apple_resources::prepare_apple_translation_languages,
            fonts::installed_font_families,
            commands::session::windows_audio_status,
            commands::help::support_diagnostics,
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
            commands::help::app_open_support_issue,
            commands::session::capture_status,
            commands::session::audio_applications,
            commands::app::app_is_ui_test,
            commands::app::app_ui_test_frontend_ready,
            commands::app::app_is_portable,
            commands::app::app_is_linux_package,
            desktop_shortcuts::app_desktop_shortcut_commands,
            commands::help::app_open_releases,
            commands::help::app_open_audio_privacy_settings,
            commands::help::open_tencent_setup_page,
            commands::help::open_alibaba_setup_page,
            commands::help::open_text_translation_setup_page,
            commands::help::open_service_setup_page,
            commands::settings::settings_save,
            commands::profiles::profile_create,
            commands::profiles::profile_duplicate,
            commands::profiles::profile_update,
            commands::profiles::profile_select,
            commands::profiles::profile_delete,
            commands::profiles::profile_save_credentials,
            commands::connection::profile_test_connection,
            commands::profiles::profile_delete_api_key,
            commands::profiles::profile_reveal_credential,
            commands::profiles::profile_credential_editor_state,
            crate::session_export::session_archive_state,
            crate::session_export::session_transcript_page,
            crate::session_export::session_history_list,
            crate::session_export::session_history_page,
            crate::session_export::session_history_audio,
            crate::session_export::session_archive_audio,
            crate::session_export::session_history_delete,
            crate::session_export::session_history_delete_many,
            crate::session_export::session_history_rename,
            crate::session_export::session_archive_clear,
            crate::session_export::session_export,
            commands::session::session_start,
            commands::session::session_stop,
            commands::session::session_toggle_paused,
            commands::session::session_clear_subtitles,
            commands::session::session_switch_source_language,
            commands::session::session_switch_target_language,
            commands::session::session_switch_audio_input,
            commands::session::session_switch_system_audio_target,
            commands::session::session_switch_translation_mode,
            commands::overlay::overlay_set_collapsed,
            commands::overlay::overlay_set_locked,
            commands::overlay::overlay_set_pointer_cursor,
            commands::overlay::overlay_move_start,
            commands::overlay::overlay_popover_toggle,
            commands::overlay::overlay_popover_hide,
            commands::overlay::overlay_control_state,
            commands::overlay::overlay_control_set_popup_bounds,
            commands::overlay::overlay_control_set_panel_height,
            commands::overlay::overlay_control_set_island_width,
            commands::session::session_get_state,
            commands::overlay::resize_start,
            commands::overlay::resize_move,
            commands::overlay::resize_end,
            commands::app::app_show_settings,
            commands::app::app_quit,
        ])
        .build(context)
        .expect("error while building tauri application")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if matches!(event, tauri::RunEvent::Exit) {
                platform::macos::native_quit::remove();
                windows::remove_all_overlay_pointer_tracking(app);
            }
            #[cfg(target_os = "windows")]
            if matches!(event, tauri::RunEvent::Exit) {
                windows::remove_all_overlay_pointer_tracking(app);
            }
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                if app.state::<AppState>().settings.preferences().show_in_dock {
                    let _ = commands::app::app_show_settings(app.clone(), None);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn development_tauri_config_uses_the_isolated_identifier() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../config/development.json")).unwrap();
        assert_eq!(
            config["identifier"].as_str(),
            Some(DEVELOPMENT_APPLICATION_IDENTIFIER)
        );
    }
}
