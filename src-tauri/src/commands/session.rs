//! Session and selected-audio-input IPC. Lifecycle policy stays with SessionManager.

use super::app::app_is_ui_test;
use super::AppState;
use crate::core::audio::audio_input::AudioInput;
use crate::core::models::{SourceLanguage, TargetLanguage, TranslationMode};
use crate::session_manager::SessionStateEvent;
use tauri::{AppHandle, State};

#[tauri::command]
pub async fn session_start(state: State<'_, AppState>) -> Result<(), String> {
    ensure_audio_input_available(state.settings.preferences().audio_input)?;
    state.session.start().await
}

#[tauri::command]
pub async fn session_stop(state: State<'_, AppState>) -> Result<(), String> {
    state.session.stop().await;
    Ok(())
}

#[tauri::command]
pub async fn session_toggle_paused(state: State<'_, AppState>) -> Result<(), String> {
    state.session.toggle_paused().await
}

#[tauri::command]
pub async fn session_clear_subtitles(state: State<'_, AppState>) -> Result<(), String> {
    // Clear from the tray/overlay must invalidate export snapshots atomically
    // with their acquisition and final write, just like settings-page clear.
    let _lifecycle = state.session.settings_mutation_guard(false).await?;
    state
        .session
        .clear_subtitles()
        .await
        .map_err(|_| "Could not clear subtitles.")?;
    Ok(())
}

#[tauri::command]
pub async fn session_switch_audio_input(
    state: State<'_, AppState>,
    input: AudioInput,
) -> Result<(), String> {
    ensure_audio_input_available(input)?;
    state.session.switch_audio_input(input).await
}

pub(super) fn ensure_audio_input_available(input: AudioInput) -> Result<(), String> {
    if input.is_available() {
        Ok(())
    } else {
        Err("microphone_input_unavailable".into())
    }
}

#[tauri::command]
pub async fn session_switch_system_audio_target(
    state: State<'_, AppState>,
    target: crate::core::audio::system_audio_target::SystemAudioTarget,
) -> Result<(), String> {
    state.session.switch_system_audio_target(target).await
}

#[tauri::command]
pub async fn session_switch_source_language(
    state: State<'_, AppState>,
    language: SourceLanguage,
) -> Result<(), String> {
    // The session manager broadcasts settings-changed immediately after the
    // preference write, so no window keeps a stale selection while the
    // reconnect (which this awaits) is still in flight.
    state.session.switch_source_language(language).await
}

#[tauri::command]
pub async fn session_switch_target_language(
    state: State<'_, AppState>,
    language: TargetLanguage,
) -> Result<(), String> {
    state.session.switch_target_language(language).await
}

#[tauri::command]
pub async fn session_switch_translation_mode(
    state: State<'_, AppState>,
    mode: TranslationMode,
) -> Result<(), String> {
    state.session.switch_translation_mode(mode).await;
    Ok(())
}

/// The current session state snapshot, for windows that boot after the last
/// broadcast (e.g. the overlay control). Async: cloning the full controller
/// state (subtitle history included) must not run on the main thread.
#[tauri::command]
pub async fn session_get_state(state: State<'_, AppState>) -> Result<SessionStateEvent, String> {
    Ok(state.session.current_state_event())
}

#[tauri::command]
pub async fn windows_audio_status(
    state: State<'_, AppState>,
) -> Result<Option<crate::audio::AudioSourceSnapshot>, String> {
    state.session.windows_audio_status()
}

#[tauri::command]
pub async fn capture_status(
    state: State<'_, AppState>,
) -> Result<crate::audio::CaptureStatus, String> {
    Ok(state.session.capture_status())
}

/// Listing does not start capture or read provider credentials. UI-test mode
/// returns synthetic choices and never invokes OS capture APIs.
#[tauri::command]
pub async fn audio_applications(
    _app: AppHandle,
    state: State<'_, AppState>,
    cached_only: Option<bool>,
) -> Result<crate::audio::applications::ApplicationSnapshot, String> {
    if app_is_ui_test() {
        return Ok(crate::audio::applications::ApplicationSnapshot {
            supported: true,
            applications: vec![crate::audio::applications::AudioApplication {
                id: "test.player".into(),
                name: if cfg!(target_os = "linux") {
                    "Test Player · Test stream"
                } else {
                    "Test Player"
                }
                .into(),
                icon_data_url: None,
            }],
        });
    }
    let snapshot = state
        .session
        .audio_applications(cached_only.unwrap_or(false))
        .await
        .map_err(|error| error.to_string())?;
    #[cfg(target_os = "linux")]
    let snapshot = crate::audio::linux_application_icons::populate(&_app, snapshot).await;
    Ok(snapshot)
}

#[cfg(test)]
mod tests;
