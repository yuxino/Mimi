//! Service-profile catalog validation, selection, editing and rollback.

use super::credentials::{
    cache_key, credential_account, TextTranslationDestination, XAITextDestination,
};
use super::persistence::atomic_write;
use super::preferences::{normalize_preferences_value, profile_selection_preferences};
use super::{
    Preferences, SecretStoreError, SettingsStore, CREDENTIAL_STORE_UNAVAILABLE, LAST_PROFILE,
    MAXIMUM_PROFILE_COUNT, PROFILE_CATALOG_SCHEMA_VERSION, PROFILE_CATALOG_UNAVAILABLE,
    PROFILE_NOT_FOUND,
};
use crate::core::credentials::ProviderCredentials;
use crate::core::models::{SourceLanguage, TranslationMode};
use crate::core::network_proxy::ProxyConfig;
use crate::core::provider::{
    CustomSpeechLanguagesPatch, ProviderKind, ServiceProfile, TextTranslation, TextTranslationName,
    DEFAULT_ALIBABA_PROFILE_ID,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

// Metadata compatibility only: these IDs no longer select a credential source.
pub(super) const LEGACY_ENV_ALIBABA_PROFILE_ID: &str = "alibaba-local-dev";
const LEGACY_ENV_GEMINI_PROFILE_ID: &str = "gemini-local-dev";

pub(super) fn is_legacy_env_profile_id(id: &str) -> bool {
    matches!(
        id,
        LEGACY_ENV_ALIBABA_PROFILE_ID | LEGACY_ENV_GEMINI_PROFILE_ID
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ProfileCatalog {
    pub(super) schema_version: u32,
    pub(super) active_profile_id: String,
    pub(super) profiles: Vec<ServiceProfile>,
}

impl Default for ProfileCatalog {
    fn default() -> Self {
        Self {
            schema_version: PROFILE_CATALOG_SCHEMA_VERSION,
            active_profile_id: String::new(),
            profiles: Vec::new(),
        }
    }
}

impl ProfileCatalog {
    /// Only pre-catalog installations and explicit test fixtures use this profile.
    pub(super) fn legacy_alibaba() -> Self {
        Self {
            active_profile_id: DEFAULT_ALIBABA_PROFILE_ID.to_string(),
            profiles: vec![ServiceProfile::alibaba_default()],
            ..Self::default()
        }
    }

    pub(super) fn validated(self) -> Result<Self, ()> {
        if self.schema_version != PROFILE_CATALOG_SCHEMA_VERSION
            || self
                .profiles
                .iter()
                .filter(|p| !is_legacy_env_profile_id(&p.id))
                .count()
                > MAXIMUM_PROFILE_COUNT
        {
            return Err(());
        }

        let mut ids = HashSet::with_capacity(self.profiles.len());
        let mut profiles = Vec::with_capacity(self.profiles.len());
        for profile in self.profiles {
            let validated = profile.validated().map_err(|_| ())?;
            if !ids.insert(validated.id.clone()) {
                return Err(());
            }
            profiles.push(validated);
        }
        if (profiles.is_empty() && !self.active_profile_id.is_empty())
            || (!profiles.is_empty() && !ids.contains(&self.active_profile_id))
        {
            return Err(());
        }

        Ok(Self {
            schema_version: self.schema_version,
            active_profile_id: self.active_profile_id,
            profiles,
        })
    }
}
impl SettingsStore {
    pub fn profile_catalog(&self) -> Result<(String, Vec<ServiceProfile>), String> {
        if self.catalog_write_blocked {
            return Err(PROFILE_CATALOG_UNAVAILABLE.to_string());
        }
        self.migrate_xai_translation_profiles()?;
        let catalog = self.catalog.lock().unwrap();
        Ok((catalog.active_profile_id.clone(), catalog.profiles.clone()))
    }

    pub fn preferences_and_catalog(
        &self,
    ) -> Result<(Preferences, String, Vec<ServiceProfile>), String> {
        if self.catalog_write_blocked {
            return Err(PROFILE_CATALOG_UNAVAILABLE.to_string());
        }
        self.migrate_xai_translation_profiles()?;
        let catalog = self.catalog.lock().unwrap();
        let mut prefs = self.prefs.lock().unwrap().clone();
        prefs.translation_mode = TranslationMode::Turbo;
        Ok((
            prefs,
            catalog.active_profile_id.clone(),
            catalog.profiles.clone(),
        ))
    }

    pub fn active_profile(&self) -> Result<ServiceProfile, String> {
        let (active_id, profiles) = self.profile_catalog()?;
        profiles
            .into_iter()
            .find(|profile| profile.id == active_id)
            .ok_or_else(|| PROFILE_CATALOG_UNAVAILABLE.to_string())
    }

    /// Upgrade only the old integrated STT profile, in its existing private
    /// store. The durable catalog marker follows verified destination storage;
    /// retry never overwrites a destination written by an earlier attempt.
    pub(super) fn migrate_xai_translation_profiles(&self) -> Result<(), String> {
        #[cfg(all(test, feature = "development-debugger", target_os = "macos"))]
        if self.secret.is_read_only() {
            return Ok(());
        }
        if self.is_ui_test {
            return Ok(());
        }
        let _upgrade = self.xai_translation_upgrade.lock().unwrap();
        let candidates: Vec<_> = self
            .catalog
            .lock()
            .unwrap()
            .profiles
            .iter()
            .filter(|profile| {
                profile.uses_xai_asr() && !profile.xai_independent_translation_migrated
            })
            .cloned()
            .collect();
        for profile in candidates {
            self.prepare_xai_translation_destination(&profile)?;
            self.mutate_catalog_with_normalization(Some(&profile.id), |catalog| {
                let current = catalog
                    .profiles
                    .iter_mut()
                    .find(|candidate| candidate.id == profile.id)
                    .ok_or_else(|| PROFILE_NOT_FOUND.to_string())?;
                if !current.xai_independent_translation_migrated {
                    current.text_translation = Some(TextTranslation::XAI);
                    current.xai_independent_translation_migrated = true;
                }
                Ok(())
            })?;
        }
        Ok(())
    }

    /// May precede a catalog commit. A verified destination is retained and
    /// reused if the mode/checkpoint write fails, never overwritten on retry.
    pub(super) fn prepare_xai_translation_destination(
        &self,
        profile: &ServiceProfile,
    ) -> Result<(), String> {
        self.retry_profile_credential_errors(profile, true, true);
        let previous = self.destination_value(profile)?;
        let mut destination = previous
            .as_deref()
            .map(serde_json::from_str::<TextTranslationDestination>)
            .transpose()
            .map_err(|_| CREDENTIAL_STORE_UNAVAILABLE.to_string())?
            .unwrap_or_default();
        if destination.xai.is_none() {
            let key = self
                .load_api_key_for_profile(profile)
                .map_err(SecretStoreError::public_error)?
                .map(|value| ProviderCredentials::decode_for_profile(profile, &value))
                .transpose()
                .map_err(|error| error.to_string())?
                .and_then(|speech| speech.direct_api_key().map(str::to_owned))
                .unwrap_or_default();
            let model =
                crate::core::protocols::openai_compatible::validate_model(&profile.xai_text_model)
                    .map_err(|error| error.to_string())?;
            // Preserve a legacy setup's chosen model even without a key.
            // Authentication stays missing and is never borrowed later.
            destination.xai = Some(XAITextDestination {
                api_key: key,
                model,
            });
            let value = serde_json::to_string(&destination)
                .map_err(|_| CREDENTIAL_STORE_UNAVAILABLE.to_string())?;
            self.write_destination_value(profile, Some(&value), true)?;
        }
        Ok(())
    }

    /// New desktop Alibaba configurations start with integrated translation.
    /// Legacy profiles and duplicate operations retain their stored mode.
    pub fn create_profile_for_setup(
        &self,
        provider: ProviderKind,
        name: &str,
    ) -> Result<ServiceProfile, String> {
        let mode = if provider == ProviderKind::AlibabaCloud {
            crate::core::provider::AlibabaRealtimeMode::LiveTranslate
        } else {
            crate::core::provider::AlibabaRealtimeMode::Disabled
        };
        self.create_profile_with_mode(provider, name, mode)
    }

    #[cfg(test)]
    pub fn create_profile(
        &self,
        provider: ProviderKind,
        name: &str,
    ) -> Result<ServiceProfile, String> {
        self.create_profile_with_mode(
            provider,
            name,
            crate::core::provider::AlibabaRealtimeMode::Disabled,
        )
    }

    pub(super) fn create_profile_with_mode(
        &self,
        provider: ProviderKind,
        name: &str,
        mode: crate::core::provider::AlibabaRealtimeMode,
    ) -> Result<ServiceProfile, String> {
        let mut profile = ServiceProfile::new(
            format!("profile-{}", uuid::Uuid::new_v4().simple()),
            name,
            provider,
        )
        .map_err(|error| error.to_string())?;
        profile
            .set_alibaba_realtime(mode, "")
            .map_err(|error| error.to_string())?;
        self.mutate_catalog_with_normalization(Some(&profile.id), |catalog| {
            if catalog.profiles.len() >= MAXIMUM_PROFILE_COUNT {
                return Err("No more service profiles can be added.".to_string());
            }
            if catalog.profiles.is_empty() {
                catalog.active_profile_id = profile.id.clone();
            }
            catalog.profiles.push(profile.clone());
            Ok(profile.clone())
        })
    }

    /// Copy private slots natively; callers receive only the public catalog.
    /// The original profile, active selection and preferences are never changed.
    pub fn duplicate_profile(
        &self,
        profile_id: &str,
        name: &str,
    ) -> Result<ServiceProfile, String> {
        if self.catalog_write_blocked {
            return Err(PROFILE_CATALOG_UNAVAILABLE.to_string());
        }
        let mut catalog = self.catalog.lock().unwrap();
        if catalog.profiles.len() >= MAXIMUM_PROFILE_COUNT {
            return Err("No more service profiles can be added.".to_string());
        }
        let source = catalog
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .ok_or_else(|| PROFILE_NOT_FOUND.to_string())?;
        let mut copy = source.clone();
        let identity = ServiceProfile::new(
            format!("profile-{}", uuid::Uuid::new_v4().simple()),
            name,
            source.provider,
        )
        .map_err(|error| error.to_string())?;
        copy.id = identity.id;
        copy.name = identity.name;
        let copy = copy.validated().map_err(|error| error.to_string())?;
        let mut next = catalog.clone();
        next.profiles.push(copy.clone());
        next.clone()
            .validated()
            .map_err(|_| PROFILE_CATALOG_UNAVAILABLE.to_string())?;

        // Read all source slots before creating any destination. Retained text
        // routes are copied together so changing route in the copy stays useful.
        self.retry_profile_credential_errors(source, true, true);
        let speech = self
            .load_api_key_for_profile(source)
            .map_err(SecretStoreError::public_error)?;
        let text = self.destination_value(source)?;
        let result = (|| {
            if let Some(value) = &speech {
                self.save_api_key_for_profile(&copy, value, true)?;
            }
            if let Some(value) = &text {
                self.write_destination_value(&copy, Some(value), true)?;
            }
            self.persist_catalog_value(&next)
        })();
        if let Err(error) = result {
            // A failed write/readback may already have created a slot. Delete
            // only the fresh UUID's slots, never delete or recreate the source.
            for account in [credential_account(&copy), Self::destination_account(&copy)] {
                if self
                    .delete_secret(self.profile_keychain_service, &account)
                    .is_err()
                {
                    tracing::warn!(
                        "profile duplicate rollback failed label=new_credential_cleanup"
                    );
                }
                // The failed copy never enters the catalog. Its random UUID's
                // cached missing/error slots cannot be reused on a retry.
                self.secret_cache
                    .lock()
                    .unwrap()
                    .remove(&cache_key(self.profile_keychain_service, &account));
            }
            return Err(error);
        }
        *catalog = next;
        Ok(copy)
    }

    #[cfg(test)]
    pub fn update_profile(&self, profile_id: &str, name: &str) -> Result<ServiceProfile, String> {
        self.update_profile_options(profile_id, Some(name), None, None, None, None, None, None)
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)] // Test helper for existing metadata patches.
    pub fn update_profile_options(
        &self,
        profile_id: &str,
        name: Option<&str>,
        speech_network_proxy: Option<ProxyConfig>,
        text_network_proxy: Option<ProxyConfig>,
        text_translation_name: Option<TextTranslationName>,
        speech_recognition_name: Option<&str>,
        custom_speech_languages_patch: Option<CustomSpeechLanguagesPatch>,
        language_preset_patch: Option<crate::core::provider::ProfileLanguagePresetPatch>,
    ) -> Result<ServiceProfile, String> {
        self.update_profile_options_with_model(
            profile_id,
            name,
            speech_network_proxy,
            text_network_proxy,
            text_translation_name,
            speech_recognition_name,
            custom_speech_languages_patch,
            language_preset_patch,
            None,
        )
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub fn update_profile_options_with_model(
        &self,
        profile_id: &str,
        name: Option<&str>,
        speech_network_proxy: Option<ProxyConfig>,
        text_network_proxy: Option<ProxyConfig>,
        text_translation_name: Option<TextTranslationName>,
        speech_recognition_name: Option<&str>,
        custom_speech_languages_patch: Option<CustomSpeechLanguagesPatch>,
        language_preset_patch: Option<crate::core::provider::ProfileLanguagePresetPatch>,
        qwen_mt_model: Option<crate::core::protocols::qwen_mt::QwenMTModel>,
    ) -> Result<ServiceProfile, String> {
        self.update_profile_options_with_realtime(
            profile_id,
            name,
            speech_network_proxy,
            text_network_proxy,
            text_translation_name,
            speech_recognition_name,
            custom_speech_languages_patch,
            language_preset_patch,
            qwen_mt_model,
            None,
            None,
        )
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub fn update_profile_options_with_realtime(
        &self,
        profile_id: &str,
        name: Option<&str>,
        speech_network_proxy: Option<ProxyConfig>,
        text_network_proxy: Option<ProxyConfig>,
        text_translation_name: Option<TextTranslationName>,
        speech_recognition_name: Option<&str>,
        custom_speech_languages_patch: Option<CustomSpeechLanguagesPatch>,
        language_preset_patch: Option<crate::core::provider::ProfileLanguagePresetPatch>,
        qwen_mt_model: Option<crate::core::protocols::qwen_mt::QwenMTModel>,
        alibaba_realtime_mode: Option<crate::core::provider::AlibabaRealtimeMode>,
        alibaba_workspace_id: Option<&str>,
    ) -> Result<ServiceProfile, String> {
        self.update_profile_options_with_xai(
            profile_id,
            name,
            speech_network_proxy,
            text_network_proxy,
            text_translation_name,
            speech_recognition_name,
            custom_speech_languages_patch,
            language_preset_patch,
            qwen_mt_model,
            alibaba_realtime_mode,
            alibaba_workspace_id,
            None,
            None,
        )
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub fn update_profile_options_with_xai(
        &self,
        profile_id: &str,
        name: Option<&str>,
        speech_network_proxy: Option<ProxyConfig>,
        text_network_proxy: Option<ProxyConfig>,
        text_translation_name: Option<TextTranslationName>,
        speech_recognition_name: Option<&str>,
        custom_speech_languages_patch: Option<CustomSpeechLanguagesPatch>,
        language_preset_patch: Option<crate::core::provider::ProfileLanguagePresetPatch>,
        qwen_mt_model: Option<crate::core::protocols::qwen_mt::QwenMTModel>,
        alibaba_realtime_mode: Option<crate::core::provider::AlibabaRealtimeMode>,
        alibaba_workspace_id: Option<&str>,
        xai_speech_mode: Option<crate::core::provider::XAISpeechMode>,
        xai_text_model: Option<&str>,
    ) -> Result<ServiceProfile, String> {
        self.update_profile_options_with_speech_mode(
            profile_id,
            name,
            speech_network_proxy,
            text_network_proxy,
            text_translation_name,
            speech_recognition_name,
            custom_speech_languages_patch,
            language_preset_patch,
            qwen_mt_model,
            alibaba_realtime_mode,
            alibaba_workspace_id,
            xai_speech_mode,
            xai_text_model,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_profile_options_with_speech_mode(
        &self,
        profile_id: &str,
        name: Option<&str>,
        speech_network_proxy: Option<ProxyConfig>,
        text_network_proxy: Option<ProxyConfig>,
        text_translation_name: Option<TextTranslationName>,
        speech_recognition_name: Option<&str>,
        custom_speech_languages_patch: Option<CustomSpeechLanguagesPatch>,
        language_preset_patch: Option<crate::core::provider::ProfileLanguagePresetPatch>,
        qwen_mt_model: Option<crate::core::protocols::qwen_mt::QwenMTModel>,
        alibaba_realtime_mode: Option<crate::core::provider::AlibabaRealtimeMode>,
        alibaba_workspace_id: Option<&str>,
        xai_speech_mode: Option<crate::core::provider::XAISpeechMode>,
        xai_text_model: Option<&str>,
        speech_translation_mode: Option<crate::core::provider::SpeechTranslationMode>,
    ) -> Result<ServiceProfile, String> {
        // Complete pre-existing upgrades before changing this profile. A
        // newly selected ASR route is prepared inside the same catalog update.
        self.migrate_xai_translation_profiles()?;
        let _upgrade = self.xai_translation_upgrade.lock().unwrap();
        let normalize_profile = (custom_speech_languages_patch.is_some()
            || alibaba_realtime_mode.is_some()
            || xai_speech_mode.is_some()
            || speech_translation_mode.is_some())
        .then_some(profile_id);
        let updated = self.mutate_catalog_with_normalization(normalize_profile, |catalog| {
            let current = catalog
                .profiles
                .iter_mut()
                .find(|profile| profile.id == profile_id)
                .ok_or_else(|| PROFILE_NOT_FOUND.to_string())?;
            let mut updated = current.clone();
            if let Some(mode) = speech_translation_mode {
                updated
                    .set_speech_translation_mode(mode)
                    .map_err(|error| error.to_string())?;
            }
            if alibaba_realtime_mode.is_some() || alibaba_workspace_id.is_some() {
                let workspace = alibaba_workspace_id.unwrap_or(&current.alibaba_workspace_id);
                let mode = alibaba_realtime_mode.unwrap_or(current.alibaba_realtime_mode);
                // Creation can persist an unfinished setup. An explicit mode /
                // workspace edit must still validate before committing any fields.
                if mode != crate::core::provider::AlibabaRealtimeMode::Disabled
                    && !crate::core::provider::alibaba_workspace_id_is_valid(workspace.trim())
                {
                    return Err(
                        crate::core::provider::ServiceProfileError::InvalidAlibabaWorkspaceId
                            .to_string(),
                    );
                }
                updated
                    .set_alibaba_realtime(mode, workspace)
                    .map_err(|error| error.to_string())?;
            }
            if xai_speech_mode.is_some() || xai_text_model.is_some() {
                if current.provider != ProviderKind::XAIRealtime {
                    return Err("provider-mismatch".into());
                }
                updated
                    .set_xai_speech(
                        xai_speech_mode.unwrap_or(current.xai_speech_mode),
                        xai_text_model.unwrap_or(&current.xai_text_model),
                    )
                    .map_err(|error| error.to_string())?;
            }
            if let Some(model) = qwen_mt_model {
                if current.provider != ProviderKind::AlibabaCloud {
                    return Err("provider-mismatch".into());
                }
                updated.qwen_mt_model = model;
            }
            if let Some(name) = name {
                updated.name = name.trim().to_string();
            }
            if let Some(proxy) = speech_network_proxy {
                updated.speech_network_proxy = Some(proxy);
            }
            if let Some(proxy) = text_network_proxy {
                updated.text_network_proxy = Some(proxy);
            }
            if let Some(patch) = text_translation_name {
                updated
                    .set_text_translation_name(patch.route, &patch.name)
                    .map_err(|error| error.to_string())?;
            }
            if let Some(patch) = custom_speech_languages_patch {
                updated
                    .set_custom_speech_source_languages(patch.languages)
                    .map_err(|error| error.to_string())?;
            }
            if let Some(name) = speech_recognition_name {
                updated
                    .set_speech_recognition_name(name)
                    .map_err(|error| error.to_string())?;
            }
            if let Some(patch) = language_preset_patch {
                if let Some(preset) = patch.preset {
                    updated.validate_language_preset(preset)?;
                }
                updated.language_preset = patch.preset;
            }
            let mut updated = updated.validated().map_err(|error| error.to_string())?;
            if !self.is_ui_test
                && updated.uses_xai_asr()
                && !updated.xai_independent_translation_migrated
            {
                self.prepare_xai_translation_destination(&updated)?;
                updated.text_translation = Some(TextTranslation::XAI);
                updated.xai_independent_translation_migrated = true;
            }
            *current = updated.clone();
            Ok(updated)
        })?;
        Ok(updated)
    }

    pub fn select_profile(&self, profile_id: &str) -> Result<(), String> {
        self.select_profile_with_source(profile_id, None)
    }

    /// Commit profile and an explicitly selected Apple language together. Runtime
    /// language-resource validation belongs to the guarded session mutation.
    pub fn select_profile_with_source(
        &self,
        profile_id: &str,
        source_language: Option<SourceLanguage>,
    ) -> Result<(), String> {
        if self.catalog_write_blocked {
            return Err(PROFILE_CATALOG_UNAVAILABLE.to_string());
        }

        let mut catalog = self.catalog.lock().unwrap();
        let mut next_catalog = catalog.clone();
        let profile = next_catalog
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .cloned()
            .ok_or_else(|| PROFILE_NOT_FOUND.to_string())?;
        next_catalog.active_profile_id = profile_id.to_string();
        next_catalog
            .clone()
            .validated()
            .map_err(|_| PROFILE_CATALOG_UNAVAILABLE.to_string())?;

        let mut prefs = self.prefs.lock().unwrap();
        let previous_prefs = prefs.clone();
        let next_prefs =
            profile_selection_preferences(previous_prefs.clone(), &profile, source_language)?;

        self.persist_preferences_value(&next_prefs)?;
        if let Err(error) = self.persist_catalog_value(&next_catalog) {
            if self.persist_preferences_value(&previous_prefs).is_err() {
                tracing::warn!("preferences unavailable label=profile_select_rollback_failed");
            }
            return Err(error);
        }

        *prefs = next_prefs;
        *catalog = next_catalog;
        Ok(())
    }

    pub fn delete_profile(&self, profile_id: &str) -> Result<(), String> {
        if self.catalog_write_blocked {
            return Err(PROFILE_CATALOG_UNAVAILABLE.to_string());
        }
        let mut catalog = self.catalog.lock().unwrap();
        if catalog.profiles.len() == 1 {
            return Err(LAST_PROFILE.to_string());
        }
        let profile = catalog
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .cloned()
            .ok_or_else(|| PROFILE_NOT_FOUND.to_string())?;
        let previous_secret = self
            .load_api_key_for_profile(&profile)
            .map_err(SecretStoreError::public_error)?;
        let previous_destination = self.destination_value(&profile)?;

        let previous_catalog = catalog.clone();
        let mut next_catalog = previous_catalog.clone();
        next_catalog
            .profiles
            .retain(|candidate| candidate.id != profile_id);
        if next_catalog.active_profile_id == profile_id {
            next_catalog.active_profile_id = next_catalog.profiles[0].id.clone();
        }
        next_catalog
            .clone()
            .validated()
            .map_err(|_| PROFILE_CATALOG_UNAVAILABLE.to_string())?;

        let next_profile = next_catalog
            .profiles
            .iter()
            .find(|candidate| candidate.id == next_catalog.active_profile_id)
            .cloned()
            .ok_or_else(|| PROFILE_CATALOG_UNAVAILABLE.to_string())?;
        let mut prefs = self.prefs.lock().unwrap();
        let previous_prefs = prefs.clone();
        let mut next_prefs = previous_prefs.clone();
        normalize_preferences_value(&mut next_prefs, &next_profile);

        // Persist the normalized preferences before selecting their provider.
        // The old provider also accepts the current providers' normalized
        // subset, and startup revalidates both documents after a crash.
        self.persist_preferences_value(&next_prefs)?;
        let deleted = self.delete_profile_credentials(&profile);
        if let Err(error) = deleted {
            let secret_restored = previous_secret
                .as_deref()
                .map(|value| self.save_api_key_for_profile(&profile, value, false))
                .transpose()
                .is_ok();
            if self.persist_preferences_value(&previous_prefs).is_err() {
                tracing::warn!("preferences unavailable label=profile_delete_rollback_failed");
            }
            let destination_restored = previous_destination
                .as_deref()
                .map(|value| self.write_destination_value(&profile, Some(value), false))
                .transpose()
                .is_ok();
            if !secret_restored || !destination_restored {
                tracing::warn!("service profile delete rollback failed label=credential_restore");
            }
            return Err(error);
        }
        if let Err(error) = self.persist_catalog_value(&next_catalog) {
            // Restore the credential when the public catalog cannot commit.
            // Deleting the secret first makes a process interruption leave a
            // visible profile with a missing key, never an unreachable secret.
            let secret_restored = previous_secret
                .as_deref()
                .map(|value| self.save_api_key_for_profile(&profile, value, false))
                .transpose()
                .is_ok();
            if self.persist_preferences_value(&previous_prefs).is_err() {
                tracing::warn!("preferences unavailable label=profile_delete_rollback_failed");
            }
            let destination_restored = previous_destination
                .as_deref()
                .map(|value| self.write_destination_value(&profile, Some(value), false))
                .transpose()
                .is_ok();
            if !secret_restored || !destination_restored {
                tracing::warn!("service profile delete rollback failed label=credential_restore");
            }
            return Err(error);
        }

        *catalog = next_catalog;
        *prefs = next_prefs;
        Ok(())
    }

    pub(super) fn profile(&self, profile_id: &str) -> Result<ServiceProfile, String> {
        let (_, profiles) = self.profile_catalog()?;
        profiles
            .into_iter()
            .find(|profile| profile.id == profile_id)
            .ok_or_else(|| PROFILE_NOT_FOUND.to_string())
    }

    pub(super) fn mutate_catalog_with_normalization<T>(
        &self,
        normalize_profile: Option<&str>,
        update: impl FnOnce(&mut ProfileCatalog) -> Result<T, String>,
    ) -> Result<T, String> {
        if self.catalog_write_blocked {
            return Err(PROFILE_CATALOG_UNAVAILABLE.to_string());
        }
        let mut catalog = self.catalog.lock().unwrap();
        let mut next = catalog.clone();
        let result = update(&mut next)?;
        next.clone()
            .validated()
            .map_err(|_| PROFILE_CATALOG_UNAVAILABLE.to_string())?;
        if normalize_profile == Some(next.active_profile_id.as_str()) {
            let profile = next
                .profiles
                .iter()
                .find(|profile| profile.id == next.active_profile_id)
                .ok_or_else(|| PROFILE_NOT_FOUND.to_string())?;
            let mut prefs = self.prefs.lock().unwrap();
            let previous = prefs.clone();
            let mut normalized = previous.clone();
            normalize_preferences_value(&mut normalized, profile);
            self.persist_preferences_value(&normalized)?;
            if let Err(error) = self.persist_catalog_value(&next) {
                if self.persist_preferences_value(&previous).is_err() {
                    tracing::warn!("preferences unavailable label=profile_options_rollback_failed");
                }
                return Err(error);
            }
            *prefs = normalized;
        } else {
            self.persist_catalog_value(&next)?;
        }
        *catalog = next;
        Ok(result)
    }

    pub(super) fn persist_catalog(&self) -> Result<(), String> {
        let catalog = self.catalog.lock().unwrap().clone();
        self.persist_catalog_value(&catalog)
    }

    pub(super) fn persist_catalog_value(&self, catalog: &ProfileCatalog) -> Result<(), String> {
        #[cfg(all(test, feature = "development-debugger", target_os = "macos"))]
        if self.secret.is_read_only() {
            return Err(PROFILE_CATALOG_UNAVAILABLE.to_string());
        }
        if self.catalog_write_blocked {
            return Err(PROFILE_CATALOG_UNAVAILABLE.to_string());
        }
        if self.is_ui_test || self.catalog_path.as_os_str().is_empty() {
            return Ok(());
        }
        let bytes = serde_json::to_vec_pretty(catalog)
            .map_err(|_| PROFILE_CATALOG_UNAVAILABLE.to_string())?;
        atomic_write(&self.catalog_path, &bytes)
            .map_err(|_| PROFILE_CATALOG_UNAVAILABLE.to_string())
    }
}
