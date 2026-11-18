//! Non-secret preference schema, normalization and atomic preference updates.

use super::persistence::{atomic_write, file_matches_bytes};
use super::{
    SettingsStore, DEFAULT_FONT_SIZE, DEFAULT_VISIBLE_SUBTITLE_COUNT, PREFERENCES_UNAVAILABLE,
    VISIBLE_SUBTITLE_COUNT_RANGE,
};
use crate::core::audio_input::AudioInput;
use crate::core::models::{
    SourceLanguage, SubtitleColor, SubtitleDisplayMode, TargetLanguage, TranslationMode,
};
use crate::core::network_proxy::ProxyConfig;
use crate::core::overlay_layout::normalize_font_size;
use crate::core::provider::{ProviderKind, ProviderPreferences, ServiceProfile, TextTranslation};
use crate::core::subtitle_font::normalize_family_name;
use serde::{Deserialize, Serialize};

/// Clamp the recent-caption display count to its supported range.
pub fn clamp_visible_subtitle_count(count: u8) -> u8 {
    count.clamp(
        *VISIBLE_SUBTITLE_COUNT_RANGE.start(),
        *VISIBLE_SUBTITLE_COUNT_RANGE.end(),
    )
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubtitleAlignment {
    Left,
    #[default]
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PulseStyle {
    Syllable,
    #[default]
    #[serde(other)]
    Ribbon,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OverlayFrame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub source_language: SourceLanguage,
    pub target_language: TargetLanguage,
    pub translation_mode: TranslationMode,
    pub font_size: f64,
    /// Empty follows the system font stack; otherwise one installed family name.
    pub subtitle_font_family: String,
    /// Background opacity in percent; independent of subtitle text.
    pub subtitle_background_opacity: u8,
    pub subtitle_color: SubtitleColor,
    pub microphone_subtitle_color: SubtitleColor,
    pub subtitle_alignment: SubtitleAlignment,
    pub subtitle_display_mode: SubtitleDisplayMode,
    pub show_intermediate_subtitles: bool,
    pub show_subtitle_dividers: bool,
    pub show_subtitle_timestamps: bool,
    /// Limit the overlay to the most recent confirmed subtitles. Off by
    /// default; `visible_subtitle_count` is only the stored choice.
    pub custom_subtitle_count_enabled: bool,
    /// 1..=10; ignored while `custom_subtitle_count_enabled` is false.
    pub visible_subtitle_count: u8,
    /// Animation switches: `None` follows the system reduce-motion setting.
    pub pulse_animation: Option<bool>,
    pub pulse_style: PulseStyle,
    pub subtitle_animation: Option<bool>,
    pub subtitle_blends_with_background: bool,
    pub overlay_locked: bool,
    pub overlay_frame: Option<OverlayFrame>,
    pub frame_layout_version: u64,
    /// UI language override. `None` and `Some("system")` both follow the
    /// operating-system language.
    pub ui_language: Option<String>,
    pub retain_session_history: bool,
    pub record_session_audio: bool,
    /// Explicitly selected inputs; legacy preferences remain system-only.
    pub audio_input: AudioInput,
    /// Empty means follow the Windows default output, including live changes.
    pub windows_audio_source: String,
    pub system_audio_target: crate::core::system_audio_target::SystemAudioTarget,
    /// macOS Dock/Cmd-Tab presence. Missing preferences show Mimi in the Dock.
    pub show_in_dock: bool,
    pub network_proxy: ProxyConfig,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            source_language: SourceLanguage::Automatic,
            target_language: TargetLanguage::SimplifiedChinese,
            translation_mode: TranslationMode::Turbo,
            font_size: DEFAULT_FONT_SIZE,
            subtitle_font_family: String::new(),
            subtitle_background_opacity: 80,
            subtitle_color: SubtitleColor::White,
            microphone_subtitle_color: SubtitleColor::Yellow,
            subtitle_alignment: SubtitleAlignment::Center,
            subtitle_display_mode: SubtitleDisplayMode::Translation,
            show_intermediate_subtitles: true,
            show_subtitle_dividers: false,
            show_subtitle_timestamps: false,
            custom_subtitle_count_enabled: false,
            visible_subtitle_count: DEFAULT_VISIBLE_SUBTITLE_COUNT,
            pulse_animation: None,
            pulse_style: PulseStyle::Ribbon,
            subtitle_animation: None,
            subtitle_blends_with_background: false,
            overlay_locked: false,
            overlay_frame: None,
            frame_layout_version: 0,
            ui_language: None,
            retain_session_history: false,
            record_session_audio: false,
            audio_input: AudioInput::System,
            windows_audio_source: String::new(),
            system_audio_target: Default::default(),
            show_in_dock: true,
            network_proxy: ProxyConfig::default(),
        }
    }
}

impl Preferences {
    pub fn apply_windows_audio_source(&mut self, source: String) {
        if self.windows_audio_source != source {
            self.record_session_audio = false;
        }
        self.windows_audio_source = source;
    }

    pub fn apply_system_audio_target(
        &mut self,
        target: crate::core::system_audio_target::SystemAudioTarget,
    ) {
        if self.system_audio_target != target {
            self.record_session_audio = false;
        }
        self.system_audio_target = target;
    }

    /// A recording opt-in belongs to the selected sources. Changing selection
    /// always requires a fresh opt-in, even in a combined settings draft.
    pub fn apply_audio_preferences(&mut self, input: Option<AudioInput>, recording: Option<bool>) {
        if let Some(recording) = recording {
            self.record_session_audio = recording;
        }
        if let Some(input) = input {
            if self.audio_input != input {
                self.record_session_audio = false;
            }
            self.audio_input = input;
        }
    }
}
/// Normalize the destination target first, then validate an explicit Apple
/// source without letting generic normalization silently choose another locale.
pub(super) fn profile_selection_preferences(
    mut prefs: Preferences,
    profile: &ServiceProfile,
    source_language: Option<SourceLanguage>,
) -> Result<Preferences, String> {
    if source_language.is_some() && profile.provider != ProviderKind::AppleSpeech {
        return Err("apple_speech_source_override_invalid".into());
    }
    if source_language.is_none() {
        if let Some(preset) = profile.language_preset {
            profile.validate_language_preset(preset)?;
            prefs.source_language = preset.source_language;
            prefs.target_language = preset.target_language;
            prefs.translation_mode = TranslationMode::Turbo;
            return Ok(prefs);
        }
    }
    normalize_preferences_value(&mut prefs, profile);
    if let Some(source) = source_language {
        if source == SourceLanguage::Automatic {
            return Err("apple_speech_language_unsupported".into());
        }
        if !profile
            .capabilities(prefs.target_language)
            .source_languages
            .contains(&source)
        {
            return Err("apple_speech_translation_language_unsupported".into());
        }
        prefs.source_language = source;
    }
    Ok(prefs)
}

pub(super) fn normalize_preferences_value(prefs: &mut Preferences, profile: &ServiceProfile) {
    let original_source = prefs.source_language;
    let normalized = profile.normalize_preferences(ProviderPreferences {
        source_language: prefs.source_language,
        target_language: prefs.target_language,
        translation_mode: prefs.translation_mode,
    });
    prefs.source_language = if profile.text_translation() == TextTranslation::Apple
        && normalized.target_language.translates_audio()
        && original_source == SourceLanguage::Automatic
    {
        SourceLanguage::Automatic
    } else {
        normalized.source_language
    };
    prefs.target_language = normalized.target_language;
    prefs.translation_mode = normalized.translation_mode;
}
#[derive(Clone, Copy)]
pub(super) enum PreferenceWrite {
    Always,
    IfDifferent,
}
impl SettingsStore {
    pub fn is_ui_test(&self) -> bool {
        self.is_ui_test
    }

    pub fn preferences(&self) -> Preferences {
        let mut prefs = self.prefs.lock().unwrap().clone();
        // Even a blocked profile catalog or legacy UI fixture must expose the
        // one current mode instead of restoring a removed selector choice.
        prefs.translation_mode = TranslationMode::Turbo;
        prefs
    }

    /// Applies and durably persists a preference update as one in-process
    /// transaction. A failed write leaves the published in-memory snapshot
    /// unchanged, so callers never report settings that will vanish on restart.
    pub fn save_preferences(&self, update: impl FnOnce(&mut Preferences)) -> Result<(), String> {
        self.save_preferences_validated(update, |_, _| Ok(()), PreferenceWrite::Always)
    }

    pub(super) fn save_preferences_validated(
        &self,
        update: impl FnOnce(&mut Preferences),
        validate: impl FnOnce(&Preferences, &mut Preferences) -> Result<(), String>,
        write: PreferenceWrite,
    ) -> Result<(), String> {
        let mut current = self.prefs.lock().unwrap();
        let mut next = current.clone();
        update(&mut next);
        validate(&current, &mut next)?;
        next.translation_mode = TranslationMode::Turbo;
        next.font_size = normalize_font_size(next.font_size);
        next.subtitle_background_opacity = next.subtitle_background_opacity.min(100);
        next.visible_subtitle_count = clamp_visible_subtitle_count(next.visible_subtitle_count);
        next.subtitle_font_family = normalize_family_name(&next.subtitle_font_family)?.to_owned();
        next.network_proxy = next
            .network_proxy
            .validate()
            .map_err(|error| error.to_string())?;
        self.write_preferences_value(&next, write)?;
        *current = next;
        Ok(())
    }

    /// Applies a preference update and the active provider's capability
    /// normalization before committing either change.
    pub fn save_preferences_for_active_profile(
        &self,
        update: impl FnOnce(&mut Preferences),
    ) -> Result<(), String> {
        let (active_id, profiles) = self.profile_catalog()?;
        let profile = profiles.into_iter().find(|profile| profile.id == active_id);
        self.save_preferences_validated(update, |previous, next| {
            let Some(profile) = &profile else { return Ok(()); };
            let capabilities = profile.capabilities(next.target_language).for_source(next.source_language);
            if next.target_language != previous.target_language
                && !capabilities.target_languages.contains(&next.target_language) {
                return Err(crate::core::configuration::LiveTranslationConfigurationError::UnsupportedTargetLanguage.to_string());
            }
            if next.source_language != previous.source_language
                && !capabilities.source_languages.contains(&next.source_language) {
                return Err(crate::core::configuration::LiveTranslationConfigurationError::UnsupportedSourceLanguage.to_string());
            }
            normalize_preferences_value(next, profile);
            Ok(())
        }, PreferenceWrite::Always)
    }

    pub(super) fn persist_preferences_value(&self, prefs: &Preferences) -> Result<(), String> {
        self.write_preferences_value(prefs, PreferenceWrite::Always)
    }

    pub(super) fn write_preferences_value(
        &self,
        prefs: &Preferences,
        write: PreferenceWrite,
    ) -> Result<(), String> {
        #[cfg(all(test, feature = "development-debugger", target_os = "macos"))]
        if self.secret.is_read_only() {
            return Err(PREFERENCES_UNAVAILABLE.to_string());
        }
        if (self.is_ui_test && !self.ui_test_preferences_writable)
            || self.prefs_path.as_os_str().is_empty()
        {
            return Ok(());
        }
        let bytes =
            serde_json::to_vec_pretty(prefs).map_err(|_| PREFERENCES_UNAVAILABLE.to_string())?;
        if matches!(write, PreferenceWrite::IfDifferent)
            && file_matches_bytes(&self.prefs_path, &bytes)
        {
            return Ok(());
        }
        atomic_write(&self.prefs_path, &bytes).map_err(|_| PREFERENCES_UNAVAILABLE.to_string())
    }
}
