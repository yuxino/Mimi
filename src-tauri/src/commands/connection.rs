//! Explicit speech/text connection checks, including unsaved configuration drafts.

use super::app::app_is_ui_test;
use super::settings::emit_settings_snapshot;
use super::AppState;
use crate::core::models::SourceLanguage;
use crate::core::services::credentials::ProviderCredentials;
use crate::core::services::provider::ProviderKind;
use tauri::AppHandle;

/// Explicit authorization/readiness check. No capture or current-session changes.
#[tauri::command]
pub async fn profile_test_connection(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    profile_id: String,
    stage: Option<crate::clients::transport::connection_diagnostics::ConnectionCheckStage>,
    credentials: Option<ProviderCredentials>,
    source_language: Option<SourceLanguage>,
) -> Result<crate::clients::transport::connection_diagnostics::ConnectionDiagnostic, String> {
    use crate::clients::transport::connection_diagnostics::{
        check_service, check_speech_service, check_text_service, preparation_failure,
        ConnectionCheckReason, ConnectionCheckStage, ConnectionDiagnostic,
    };
    let (_, profiles) = state.settings.profile_catalog()?;
    let profile = profiles
        .iter()
        .find(|p| p.id == profile_id)
        .ok_or("profile_not_found")?;
    if source_language.is_some()
        && (profile.provider != ProviderKind::AppleSpeech
            || stage != Some(ConnectionCheckStage::Speech)
            || credentials.is_some())
    {
        return Err("apple_speech_source_override_invalid".into());
    }
    if let Some(credentials) = credentials {
        // A draft is an explicit, ephemeral check, never a saved-profile
        // readiness request. In particular it must not emit a credential snapshot.
        return Ok(match stage {
            Some(ConnectionCheckStage::Text) => match state
                .settings
                .configuration_for_text_draft_probe(profile, &credentials)
            {
                Err(error) => preparation_failure(&error),
                Ok(_) if app_is_ui_test() => ConnectionDiagnostic::not_tested("present"),
                Ok(configuration) => check_text_service(&configuration).await,
            },
            stage => match state.settings.configuration_for_speech_draft_probe(
                profile,
                &credentials,
                stage.is_some(),
            ) {
                Err(error) => preparation_failure(&error),
                Ok(_) if app_is_ui_test() => ConnectionDiagnostic::not_tested("present"),
                Ok(configuration) => check_speech_service(&configuration, stage.is_none()).await,
            },
        });
    }
    if let Some(stage) = stage {
        // Stage checks deliberately avoid an aggregate credential snapshot: a
        // text-only check must not prompt for or require the recognizer's key.
        let diagnostic = match stage {
            ConnectionCheckStage::Speech => {
                let configuration = match source_language {
                    Some(source) => state
                        .settings
                        .configuration_for_speech_probe_with_source(profile, Some(source)),
                    None => state.settings.configuration_for_speech_probe(profile),
                };
                match configuration {
                    Err(error) => preparation_failure(&error),
                    Ok(_) if app_is_ui_test() => ConnectionDiagnostic::not_tested("present"),
                    Ok(configuration) => check_speech_service(&configuration, false).await,
                }
            }
            ConnectionCheckStage::Text => {
                match state.settings.configuration_for_text_probe(profile) {
                    Err(error) => preparation_failure(&error),
                    Ok(_) if app_is_ui_test() => ConnectionDiagnostic::not_tested("present"),
                    Ok(configuration) => check_text_service(&configuration).await,
                }
            }
        };
        if stage == ConnectionCheckStage::Speech && profile.provider == ProviderKind::AppleSpeech {
            // The native check refreshes resource status even on failure. Keep
            // its ready/unknown result in sync with the settings resource row.
            emit_settings_snapshot(&app, &state.settings)?;
        }
        return Ok(diagnostic);
    }
    let storage = state.settings.credential_diagnostic(profile);
    emit_settings_snapshot(&app, &state.settings)?;
    if let Some(failure) = ConnectionDiagnostic::credential_failure(storage) {
        return Ok(failure);
    }
    if app_is_ui_test() {
        return Ok(ConnectionDiagnostic::not_tested(storage));
    }
    let configuration = match state.settings.configuration_for_profile_probe(profile) {
        Ok(configuration) => configuration,
        Err(_) => {
            return Ok(ConnectionDiagnostic::unavailable(
                storage,
                ConnectionCheckReason::InvalidConfiguration,
            ));
        }
    };
    let diagnostic = check_service(&configuration).await;
    if profile.provider == ProviderKind::AppleSpeech {
        emit_settings_snapshot(&app, &state.settings)?;
    }
    Ok(diagnostic)
}
