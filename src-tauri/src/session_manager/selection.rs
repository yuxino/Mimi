//! Apply source/profile/language changes transactionally, then reconnect only the request that still owns the lifecycle epoch.

use super::ownership::{
    retain_lifecycle_during_teardown, update_owned_value, SESSION_START_CANCELLED,
};
use super::SessionManager;
use crate::core::audio_input::{AudioInput, AudioSource};
use crate::core::configuration::LiveTranslationConfiguration;
use crate::core::models::{SessionStatus, SourceLanguage, TranslationMode};
use crate::core::provider::ProviderKind;
use crate::core::session::TranslationSessionController;
use crate::core::system_audio_target::SystemAudioTarget;
use crate::pipeline_log;
use mimi_runtime::core::recovery::{
    advance_lifecycle_sequence_if_current, lifecycle_sequence_matches, NO_GENERATION,
};
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub(super) fn translation_mode_after_source_switch(
    _provider: ProviderKind,
    _source_language: SourceLanguage,
    _current_mode: TranslationMode,
) -> TranslationMode {
    TranslationMode::Turbo
}

pub(super) fn pipeline_settings_mutation_is_allowed(
    status: &SessionStatus,
    lifecycle_operations: usize,
) -> bool {
    lifecycle_operations == 0
        && !matches!(status, SessionStatus::Connecting | SessionStatus::Stopping)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProfileSwitchAction {
    SelectOnly,
    KeepPaused,
    Reconnect,
}

pub(super) fn profile_switch_action(
    status: &SessionStatus,
    paused: bool,
    recovering: bool,
    lifecycle_operations: usize,
) -> Result<ProfileSwitchAction, &'static str> {
    if recovering || !pipeline_settings_mutation_is_allowed(status, lifecycle_operations) {
        return Err("profile_switch_busy");
    }
    if paused {
        Ok(ProfileSwitchAction::KeepPaused)
    } else if matches!(status, SessionStatus::Listening) {
        Ok(ProfileSwitchAction::Reconnect)
    } else {
        Ok(ProfileSwitchAction::SelectOnly)
    }
}

pub(super) fn ensure_profile_switch_audio_format(
    recording: bool,
    current_sample_rate: u32,
    next_sample_rate: u32,
) -> Result<(), String> {
    if recording && current_sample_rate != next_sample_rate {
        Err("profile_switch_recording_requires_stop".into())
    } else {
        Ok(())
    }
}

/// Caller holds the lifecycle and generation-transition gates. Persist before
/// replacing the resumable configuration, retaining its current owner (which
/// can be NO_GENERATION after a failed resume leaves the session paused).
pub(super) fn commit_profile_selection(
    lifecycle_sequence: &AtomicU64,
    expected_epoch: u64,
    active_settings: &Mutex<Option<LiveTranslationConfiguration>>,
    proposed: Option<LiveTranslationConfiguration>,
    select: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    if !lifecycle_sequence_matches(lifecycle_sequence, expected_epoch) {
        return Err("profile_switch_superseded".into());
    }
    select()?;
    if let Some(proposed) = proposed {
        *active_settings.lock().unwrap() = Some(proposed);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AudioInputSwitchAction {
    ReconfigureOnly,
    Reconnect,
}

pub(super) fn validate_system_audio_target(
    target: &SystemAudioTarget,
    ui_test: bool,
    application_capture_supported: impl FnOnce() -> bool,
) -> Result<(), String> {
    if !target.validate() {
        return Err("application_audio_invalid_target".into());
    }
    if target.application_id().is_some() && !ui_test && !application_capture_supported() {
        return Err(crate::audio::SystemAudioCaptureError::ApplicationUnsupported.to_string());
    }
    Ok(())
}

/// Selecting an application while only the microphone is enabled is a future
/// system-lane preference; it must not interrupt the unrelated microphone.
pub(super) fn audio_capture_selection_needs_restart(
    previous: AudioInput,
    selected: AudioInput,
) -> bool {
    previous != selected || selected.sources().contains(&AudioSource::System)
}

pub(super) fn audio_input_switch_action(
    status: &SessionStatus,
    paused: bool,
    recovering: bool,
    lifecycle_operations: usize,
) -> Result<AudioInputSwitchAction, &'static str> {
    if recovering || !pipeline_settings_mutation_is_allowed(status, lifecycle_operations) {
        return Err("audio_input_switch_busy");
    }
    if paused || !matches!(status, SessionStatus::Listening) {
        Ok(AudioInputSwitchAction::ReconfigureOnly)
    } else {
        Ok(AudioInputSwitchAction::Reconnect)
    }
}

/// Caller owns both the content gate and generation-transition lock. No
/// provider event can enter between sealing the old generation and resetting
/// its confirmation watermarks; a superseded/save-failed request resets none.
pub(super) fn commit_audio_input_switch_boundary(
    active_generation: &AtomicU64,
    lifecycle_sequence: &AtomicU64,
    expected_epoch: u64,
    controller: &mut TranslationSessionController,
    input: AudioInput,
    persist: impl FnOnce() -> Result<(), String>,
) -> Result<(u64, u64), String> {
    if !lifecycle_sequence_matches(lifecycle_sequence, expected_epoch) {
        return Err("audio_input_switch_superseded".into());
    }
    persist()?;
    let generation = advance_lifecycle_sequence_if_current(lifecycle_sequence, expected_epoch)
        .ok_or_else(|| "audio_input_switch_superseded".to_string())?;
    let previous = active_generation.swap(NO_GENERATION, Ordering::SeqCst);
    controller.reconfigure_audio_input(input);
    Ok((generation, previous))
}

/// Switching off a source must confirm native release even when the session
/// is already paused, idle, or failed. Those states may still be tearing down.
pub(super) async fn release_reconfigured_audio_sources<Stop, Stopped>(
    previous: AudioInput,
    selected: AudioInput,
    system_target_changed: bool,
    mut stop: Stop,
) -> Result<(), String>
where
    Stop: FnMut(AudioSource) -> Stopped,
    Stopped: Future<Output = Result<(), String>>,
{
    for &source in previous.sources() {
        if !selected.sources().contains(&source)
            || (system_target_changed && source == AudioSource::System)
        {
            stop(source).await?;
        }
    }
    Ok(())
}

pub(super) fn source_switch_requires_reconnect(
    is_listening: bool,
    current_source: SourceLanguage,
    current_target: crate::core::models::TargetLanguage,
    current_mode: TranslationMode,
    next_source: SourceLanguage,
    next_target: crate::core::models::TargetLanguage,
    next_mode: TranslationMode,
) -> bool {
    is_listening
        && (current_source != next_source
            || current_target != next_target
            || current_mode != next_mode)
}

impl SessionManager {
    /// The compact audio switches are explicit live reconfiguration. Preserve
    /// confirmed subtitles, revoke recording consent, and restart only when
    /// currently listening; a paused session must never briefly open a mic.
    pub async fn switch_audio_input(self: &Arc<Self>, input: AudioInput) -> Result<(), String> {
        self.switch_audio_capture(Some(input), None).await
    }

    pub async fn switch_system_audio_target(
        self: &Arc<Self>,
        target: SystemAudioTarget,
    ) -> Result<(), String> {
        validate_system_audio_target(
            &target,
            self.is_ui_test(),
            crate::audio::applications::supported,
        )?;
        self.switch_audio_capture(None, Some(target)).await
    }

    pub(super) async fn switch_audio_capture(
        self: &Arc<Self>,
        input: Option<AudioInput>,
        target: Option<SystemAudioTarget>,
    ) -> Result<(), String> {
        let switch_epoch = self.lifecycle_sequence.load(Ordering::SeqCst);
        let lifecycle = self.settings_mutation_guard(false).await?;
        if !self.is_lifecycle_request_current(switch_epoch) {
            return Err("audio_input_switch_superseded".into());
        }
        let status = self.controller.lock().unwrap().state.status.clone();
        let action = audio_input_switch_action(
            &status,
            self.is_paused(),
            self.is_recovering.load(Ordering::SeqCst),
            self.lifecycle_operations.load(Ordering::SeqCst),
        )
        .map_err(str::to_owned)?;
        let preferences = self.settings.preferences();
        let input = input.unwrap_or(preferences.audio_input);
        let target = target.unwrap_or_else(|| preferences.system_audio_target.clone());
        if preferences.audio_input == input && preferences.system_audio_target == target {
            return Ok(());
        }
        let persist = || {
            self.settings
                .save_preferences(|prefs| {
                    prefs.apply_audio_preferences(Some(input), None);
                    prefs.apply_system_audio_target(target.clone());
                })
                .map_err(|_| "audio_input_switch_save_failed".to_string())
        };
        if !audio_capture_selection_needs_restart(preferences.audio_input, input) {
            {
                let _transition = self.generation_transition.lock().unwrap();
                if !self.is_lifecycle_request_current(switch_epoch) {
                    return Err("audio_input_switch_superseded".into());
                }
                persist()?;
            }
            self.apply_archive_opt_out(None, Some(false));
            self.publish_settings();
            return Ok(());
        }
        let previous_input = *self.active_audio_input.lock().unwrap();
        let _operation = self.begin_lifecycle_operation();
        let (generation, old_generation) = {
            let _content = self.subtitle_content_lock.lock().await;
            let _transition = self.generation_transition.lock().unwrap();
            let boundary = commit_audio_input_switch_boundary(
                &self.active_generation,
                &self.lifecycle_sequence,
                switch_epoch,
                &mut self.controller.lock().unwrap(),
                input,
                persist,
            )?;
            self.recovery_continuity.lock().unwrap().clear();
            *self.active_audio_input.lock().unwrap() = input;
            self.lifecycle_notify.notify_waiters();
            boundary
        };
        self.apply_archive_opt_out(None, Some(false));
        self.publish_settings();
        self.stop_health_checks().await;
        self.cancel_recovery().await;
        self.cancel_translation_timeout();
        if action == AudioInputSwitchAction::Reconnect {
            self.controller.lock().unwrap().begin_connecting();
        }
        self.publish_state();
        let lifecycle = retain_lifecycle_during_teardown(lifecycle, async {
            if old_generation != NO_GENERATION {
                self.cleanup_generation(old_generation).await;
            }
        })
        .await;
        if !self.is_lifecycle_request_current(generation) {
            return Err("audio_input_switch_superseded".into());
        }
        if !self.is_ui_test() {
            // A target change keeps the system lane selected but retires its
            // former native application capture just like disabling that lane.
            let released = release_reconfigured_audio_sources(
                previous_input,
                input,
                preferences.system_audio_target != target,
                |source| {
                    let capture = self.lane(source).audio.lock().unwrap().clone();
                    async move {
                        match tokio::time::timeout(Duration::from_secs(5), capture.stop_and_wait())
                            .await
                        {
                            Ok(Ok(())) => Ok(()),
                            _ => Err("audio_input_switch_stop_failed".into()),
                        }
                    }
                },
            )
            .await;
            if let Err(error) = released {
                if self.is_lifecycle_request_current(generation) {
                    self.clear_active_settings();
                    self.is_paused.store(false, Ordering::SeqCst);
                    self.controller.lock().unwrap().did_fail(error.clone());
                    self.publish_state();
                }
                return Err(error);
            }
        }
        if !self.is_lifecycle_request_current(generation) {
            return Err("audio_input_switch_superseded".into());
        }
        if action == AudioInputSwitchAction::ReconfigureOnly {
            self.publish_state();
            return Ok(());
        }
        self.active_generation.store(generation, Ordering::SeqCst);
        self.retag_active_settings(generation);
        drop(lifecycle);
        self.establish_session(generation).await.map_err(|error| {
            if error == SESSION_START_CANCELLED {
                "audio_input_switch_superseded".into()
            } else {
                error
            }
        })
    }

    /// Selects a saved profile while retaining the current session and its
    /// selected sources. A live selection reconnects; a paused one only
    /// replaces the configuration used by Resume.
    pub async fn switch_profile(self: &Arc<Self>, profile_id: &str) -> Result<(), String> {
        self.switch_profile_with_source(profile_id, None).await
    }

    pub async fn switch_profile_with_source(
        self: &Arc<Self>,
        profile_id: &str,
        source_language: Option<SourceLanguage>,
    ) -> Result<(), String> {
        let switch_epoch = self.lifecycle_sequence.load(Ordering::SeqCst);
        let lifecycle = self
            .settings_mutation_guard(source_language.is_some())
            .await?;
        if !self.is_lifecycle_request_current(switch_epoch) {
            return Err("profile_switch_superseded".into());
        }
        let status = self.controller.lock().unwrap().state.status.clone();
        let action = profile_switch_action(
            &status,
            self.is_paused(),
            self.is_recovering.load(Ordering::SeqCst),
            self.lifecycle_operations.load(Ordering::SeqCst),
        )
        .map_err(str::to_owned)?;
        let current_profile = self.settings.active_profile()?;
        if current_profile.id == profile_id
            && source_language.is_none()
            && current_profile.language_preset.is_none()
        {
            return Ok(());
        }
        let configuration_failure = self.configuration_failure_snapshot();
        let (_, profiles) = self.settings.profile_catalog()?;
        let profile = profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .ok_or_else(|| "The service profile does not exist.".to_string())?;
        let selection = self
            .settings
            .preferences_for_profile_selection(profile, source_language)?;
        let apple_support = if profile.provider == ProviderKind::AppleSpeech {
            if self.is_ui_test() {
                return Err("apple_speech_ui_test_unavailable".into());
            }
            let support = crate::apple_speech_support::refresh()
                .await
                .and_then(|support| {
                    if support.available {
                        Ok(support)
                    } else {
                        Err("apple_speech_unavailable".into())
                    }
                })
                .inspect_err(|_| self.publish_settings())?;
            if !self.is_lifecycle_request_current(switch_epoch) {
                return Err("profile_switch_superseded".into());
            }
            Some(support)
        } else {
            None
        };
        let live = action != ProfileSwitchAction::SelectOnly;
        if profile.text_translation() == crate::core::provider::TextTranslation::Apple {
            if self.is_ui_test() && selection.target_language.translates_audio() {
                return Err("apple_translation_unavailable".into());
            }
            if !live
                && selection.source_language == SourceLanguage::Automatic
                && selection.target_language.translates_audio()
            {
                // Idle metadata can await an explicit source choice; do not
                // guess another language merely to activate the editor.
                if !crate::apple_translation_support::refresh().await?.available {
                    return Err("apple_translation_unavailable".into());
                }
            } else {
                crate::apple_translation_support::validate_pair(
                    selection.source_language,
                    selection.target_language,
                    live || (source_language.is_none() && profile.language_preset.is_some()),
                )
                .await?;
            }
            if !self.is_lifecycle_request_current(switch_epoch) {
                return Err("profile_switch_superseded".into());
            }
        }
        // Validate before persistence: an incomplete profile must not replace
        // the working session. UI fixtures never resolve credentials.
        let proposed = if live && !self.is_ui_test() {
            Some(match source_language {
                Some(source) => self
                    .settings
                    .configuration_for_profile_selection_with_source(profile, Some(source))?,
                None => self.settings.configuration_for_profile_selection(profile)?,
            })
        } else {
            None
        };
        if let Some(support) = apple_support {
            crate::apple_speech_support::validate_refreshed_profile_source(
                Ok(support),
                profile,
                selection.source_language,
                selection.target_language,
                live || source_language.is_some() || profile.language_preset.is_some(),
                || self.publish_settings(),
            )?;
        }
        if live {
            let target = self.settings.preferences().target_language;
            let current_rate = self
                .active_settings
                .lock()
                .unwrap()
                .as_ref()
                .map(|configuration| configuration.capabilities().input_sample_rate_hz)
                .unwrap_or_else(|| current_profile.capabilities(target).input_sample_rate_hz);
            let next_rate = proposed
                .as_ref()
                .map(|configuration| configuration.capabilities().input_sample_rate_hz)
                .unwrap_or_else(|| profile.capabilities(target).input_sample_rate_hz);
            // Every session audio track has one fixed PCM rate. Keep its
            // existing bytes and consent intact rather than dropping a track
            // or silently ceasing recording after a 16/24 kHz switch.
            ensure_profile_switch_audio_format(
                self.history_pending_audio.load(Ordering::SeqCst),
                current_rate,
                next_rate,
            )?;
        }
        {
            let _transition = self.generation_transition.lock().unwrap();
            commit_profile_selection(
                &self.lifecycle_sequence,
                switch_epoch,
                &self.active_settings,
                proposed,
                || {
                    match source_language {
                        Some(source) => self
                            .settings
                            .select_profile_with_source(profile_id, Some(source)),
                        None => self.settings.select_profile(profile_id),
                    }
                    .map_err(|_| "profile_switch_save_failed".to_string())
                },
            )?;
            self.recovery_continuity.lock().unwrap().clear();
        }
        // The persisted selection repairs only the failure observed under
        // this lifecycle guard. Keep subtitles and never start an idle session.
        self.configuration_saved(configuration_failure, true);
        self.publish_settings();
        if action == ProfileSwitchAction::Reconnect {
            drop(lifecycle);
            return self
                .try_reconnect_if_current(switch_epoch)
                .await
                .map_err(|error| {
                    if error == "language_switch_superseded" {
                        "profile_switch_superseded".into()
                    } else {
                        error
                    }
                });
        }
        Ok(())
    }

    /// Quick-switches the source language, reconnecting when needed.
    /// Source changes preserve the single Turbo path for every provider.
    pub async fn switch_source_language(
        self: &Arc<Self>,
        language: SourceLanguage,
    ) -> Result<(), String> {
        let switch_epoch = self.lifecycle_sequence.load(Ordering::SeqCst);
        let lifecycle = self
            .settings_mutation_guard(false)
            .await
            .map_err(|_| "source_switch_busy".to_string())?;
        if !self.is_lifecycle_request_current(switch_epoch) {
            return Err("source_switch_superseded".into());
        }
        let status = self.controller.lock().unwrap().state.status.clone();
        if !pipeline_settings_mutation_is_allowed(
            &status,
            self.lifecycle_operations.load(Ordering::SeqCst),
        ) {
            return Err("source_switch_busy".into());
        }
        let configuration_failure = self.configuration_failure_snapshot();
        let profile = self
            .settings
            .active_profile()
            .map_err(|_| "source_switch_profile".to_string())?;
        let provider = profile.effective_provider();
        let prefs = self.settings.preferences();
        if provider == ProviderKind::AppleSpeech {
            if self.is_ui_test() {
                return Err("apple_speech_ui_test_unavailable".into());
            }
            // Validate once while holding the lifecycle guard. Refusals must
            // reach IPC instead of looking like a successful language change.
            let support = crate::apple_speech_support::refresh().await;
            let installed_required = self.has_active_session() || self.is_paused();
            crate::apple_speech_support::validate_refreshed_profile_source(
                support,
                &profile,
                language,
                prefs.target_language,
                installed_required,
                || self.publish_settings(),
            )?;
            if !self.is_lifecycle_request_current(switch_epoch) {
                return Err("source_switch_superseded".into());
            }
        }
        let capabilities = profile.capabilities(prefs.target_language);
        if !capabilities.source_languages.contains(&language) {
            return Err("source_switch_unsupported".into());
        }
        let (target_language, next_mode, needs_reconnect) = {
            // Menus and settings share explicit source-selection semantics:
            // retain the target unless this route requires normalization.
            let target =
                capabilities.target_language_after_source_switch(language, prefs.target_language);
            if !profile
                .capabilities(target)
                .source_languages
                .contains(&language)
            {
                return Err("source_switch_unsupported".into());
            }
            let mode =
                translation_mode_after_source_switch(provider, language, prefs.translation_mode);
            let needs_reconnect = source_switch_requires_reconnect(
                self.controller.lock().unwrap().state.status == SessionStatus::Listening,
                prefs.source_language,
                prefs.target_language,
                prefs.translation_mode,
                language,
                target,
                mode,
            );
            (target, mode, needs_reconnect)
        };
        if profile.text_translation() == crate::core::provider::TextTranslation::Apple {
            if self.is_ui_test() && target_language.translates_audio() {
                return Err("apple_translation_unavailable".into());
            }
            crate::apple_translation_support::validate_pair(
                language,
                target_language,
                self.has_active_session() || self.is_paused(),
            )
            .await?;
            if !self.is_lifecycle_request_current(switch_epoch) {
                return Err("source_switch_superseded".into());
            }
        }
        if self
            .settings
            .save_preferences_for_active_profile(|prefs| {
                prefs.source_language = language;
                prefs.target_language = target_language;
                prefs.translation_mode = next_mode;
            })
            .is_err()
        {
            pipeline_log!("preferences unavailable label=source_switch_write_failed");
            return Err("source_switch_save_failed".into());
        }
        self.configuration_saved(configuration_failure, prefs.source_language != language);
        // A pause/stop can claim a newer lifecycle epoch while waiting for
        // this guard. Keep the current session's immutable settings snapshot
        // in sync before releasing the guard so a later pause/resume cannot
        // revive the pre-switch configuration even if reconnect loses that
        // newer-intent race.
        update_owned_value(
            &self.active_settings,
            &self.active_settings_generation,
            |configuration| {
                configuration.source_language = language;
                configuration.target_language = target_language;
                configuration.translation_mode = next_mode;
            },
        );
        // Broadcast immediately: the reconnect below can take seconds, and
        // every window (including the overlay control) must see the new
        // selection right away.
        self.publish_settings();
        if self.is_paused() || !needs_reconnect {
            return Ok(());
        }

        pipeline_log!(
            "session language switch source={} target={}",
            language.raw_value(),
            target_language.raw_value()
        );
        drop(lifecycle);
        // Preferences already identify the selected language. A reconnect
        // failure remains a session error and is also returned to its control.
        self.try_reconnect_if_current(switch_epoch).await
    }

    /// Switches to a supported translation target or recognition-only mode.
    /// Shares the source-switch lifecycle: paused sessions stay paused, live
    /// sessions reconnect, and newer stop/pause requests supersede reconnect.
    pub async fn switch_target_language(
        self: &Arc<Self>,
        target: crate::core::models::TargetLanguage,
    ) -> Result<(), String> {
        use crate::core::provider::ProviderPreferences;
        let switch_epoch = self.lifecycle_sequence.load(Ordering::SeqCst);
        let lifecycle = self.settings_mutation_guard(false).await?;
        let status = self.controller.lock().unwrap().state.status.clone();
        if !self.is_lifecycle_request_current(switch_epoch)
            || !pipeline_settings_mutation_is_allowed(
                &status,
                self.lifecycle_operations.load(Ordering::SeqCst),
            )
        {
            return Err("target_switch_busy".into());
        }
        let profile = self
            .settings
            .active_profile()
            .map_err(|_| "target_switch_profile")?;
        let prefs = self.settings.preferences();
        let selection = profile
            .preferences_after_target_switch(
                ProviderPreferences {
                    source_language: prefs.source_language,
                    target_language: prefs.target_language,
                    translation_mode: prefs.translation_mode,
                },
                target,
            )
            .ok_or("target_switch_unsupported")?;
        if prefs.target_language == target {
            return Ok(());
        }
        if profile.provider == ProviderKind::AppleSpeech {
            if self.is_ui_test() {
                return Err("apple_speech_ui_test_unavailable".into());
            }
            crate::apple_speech_support::validate_refreshed_profile_source(
                crate::apple_speech_support::refresh().await,
                &profile,
                prefs.source_language,
                target,
                self.has_active_session() || self.is_paused(),
                || self.publish_settings(),
            )?;
            if !self.is_lifecycle_request_current(switch_epoch) {
                return Err("language_switch_superseded".into());
            }
        }
        if profile.text_translation() == crate::core::provider::TextTranslation::Apple {
            if self.is_ui_test() && target.translates_audio() {
                return Err("apple_translation_unavailable".into());
            }
            crate::apple_translation_support::validate_pair(
                prefs.source_language,
                target,
                self.has_active_session() || self.is_paused(),
            )
            .await?;
            if !self.is_lifecycle_request_current(switch_epoch) {
                return Err("language_switch_superseded".into());
            }
        }
        // A session started in Original mode can lack translation credentials.
        // Validate its resumed route before saving, so a missing text key cannot
        // leave the preference and running session on different selections.
        let proposed_configuration = self.active_settings.lock().unwrap().clone();
        let proposed_configuration = proposed_configuration
            .map(|configuration| {
                self.settings
                    .configuration_for_target_switch(&profile, configuration, selection)
            })
            .transpose()?;
        self.settings
            .save_preferences_for_active_profile(|prefs| {
                prefs.source_language = selection.source_language;
                prefs.target_language = selection.target_language;
                prefs.translation_mode = selection.translation_mode;
            })
            .map_err(|_| "target_switch_save_failed")?;
        update_owned_value(
            &self.active_settings,
            &self.active_settings_generation,
            |configuration| {
                if let Some(proposed) = &proposed_configuration {
                    *configuration = proposed.clone();
                }
            },
        );
        self.publish_settings();
        if self.is_paused() || status != SessionStatus::Listening {
            return Ok(());
        }
        drop(lifecycle);
        self.try_reconnect_if_current(switch_epoch).await
    }

    /// Quick-switches the translation mode, reconnecting when needed.
    pub async fn switch_translation_mode(self: &Arc<Self>, mode: TranslationMode) {
        let switch_epoch = self.lifecycle_sequence.load(Ordering::SeqCst);
        let lifecycle = match self.settings_mutation_guard(false).await {
            Ok(guard) => guard,
            Err(_) => return,
        };
        if !self.is_lifecycle_request_current(switch_epoch) {
            return;
        }
        let status = self.controller.lock().unwrap().state.status.clone();
        if !pipeline_settings_mutation_is_allowed(
            &status,
            self.lifecycle_operations.load(Ordering::SeqCst),
        ) {
            return;
        }
        let provider = self
            .settings
            .active_profile()
            .map(|profile| profile.effective_provider())
            .unwrap_or(ProviderKind::AlibabaCloud);
        if !provider.capabilities().translation_modes.contains(&mode) {
            return;
        }
        let current = self.settings.preferences().translation_mode;
        if current == mode {
            return;
        }
        if self
            .settings
            .save_preferences_for_active_profile(|prefs| prefs.translation_mode = mode)
            .is_err()
        {
            pipeline_log!("preferences unavailable label=mode_switch_write_failed");
            return;
        }
        update_owned_value(
            &self.active_settings,
            &self.active_settings_generation,
            |configuration| configuration.translation_mode = mode,
        );
        // Broadcast immediately: the reconnect below can take seconds.
        self.publish_settings();
        if self.is_paused() {
            return;
        }
        let is_listening = self.controller.lock().unwrap().state.status == SessionStatus::Listening;
        if !is_listening {
            return;
        }

        pipeline_log!("session translation mode switch mode={:?}", mode);
        drop(lifecycle);
        self.reconnect_if_current(switch_epoch).await;
    }

    /// Tears down and re-establishes the session, keeping subtitles.
    pub(super) async fn reconnect_if_current(self: &Arc<Self>, expected_epoch: u64) {
        let _ = self.try_reconnect_if_current(expected_epoch).await;
    }

    /// Profile and language controls need the actual reconnect outcome.
    /// Other lifecycle callers retain session-state-only error handling.
    pub(super) async fn try_reconnect_if_current(
        self: &Arc<Self>,
        expected_epoch: u64,
    ) -> Result<(), String> {
        let _operation = self.begin_lifecycle_operation();
        let Some(reconnect_generation) = self.advance_lifecycle_request_if_current(expected_epoch)
        else {
            return Err("language_switch_superseded".into());
        };
        let lifecycle = Arc::clone(&self.lifecycle_lock).lock_owned().await;
        if !self.is_lifecycle_request_current(reconnect_generation) {
            return Err("language_switch_superseded".into());
        }
        let old_generation = self.active_generation.swap(NO_GENERATION, Ordering::SeqCst);
        self.stop_health_checks().await;
        self.cancel_recovery().await;
        self.controller.lock().unwrap().begin_connecting();
        self.publish_state();
        let cleanup = self.cleanup_generation(old_generation);
        drop(lifecycle);
        cleanup.await;
        let lifecycle = Arc::clone(&self.lifecycle_lock).lock_owned().await;
        if !self.is_lifecycle_request_current(reconnect_generation) {
            return Err("language_switch_superseded".into());
        }
        self.active_generation
            .store(reconnect_generation, Ordering::SeqCst);
        self.retag_active_settings(reconnect_generation);
        drop(lifecycle);
        self.establish_session(reconnect_generation)
            .await
            .map_err(|error| {
                if error == SESSION_START_CANCELLED {
                    "language_switch_superseded".into()
                } else {
                    error
                }
            })
    }
}
