//! Translation request-budget continuity across client replacement.
//!
//! This module owns route identity and the single retained budget. Desktop and
//! JNI lifecycle owners keep their existing locks and choose when to reset it.
//! A retirement lease captures its token before asynchronously exporting the
//! client budget, so a late export cannot replace a newer owner's accounting.

use crate::core::models::TranslationMode;
use crate::core::protocols::translation::qwen_mt::QwenMTModel;
use crate::core::services::configuration::LiveTranslationConfiguration;
use crate::core::services::credentials::{ProviderCredentials, TextTranslationCredentials};
use crate::core::services::provider::ProviderKind;
use crate::core::subtitles::preview_pacing::MTRequestBudget;

#[derive(Clone, PartialEq, Eq)]
pub enum MTBudgetRoute {
    Apple,
    Qwen(QwenMTModel),
    DeepL,
    DeepLX,
    OpenAICompatible,
    ChatMock,
    XAI,
}

#[derive(Clone, PartialEq, Eq)]
pub struct MTBudgetScope {
    pub profile_id: String,
    pub provider: ProviderKind,
    pub route: MTBudgetRoute,
}

impl MTBudgetScope {
    pub fn for_configuration(
        profile_id: String,
        configuration: &LiveTranslationConfiguration,
    ) -> Option<Self> {
        let route = if let Some(credentials) = &configuration.text_credentials {
            if !configuration.target_language.translates_audio() {
                return None;
            }
            match credentials {
                TextTranslationCredentials::Apple => MTBudgetRoute::Apple,
                TextTranslationCredentials::DeepL { .. } => MTBudgetRoute::DeepL,
                TextTranslationCredentials::DeepLX { .. } => MTBudgetRoute::DeepLX,
                TextTranslationCredentials::OpenAICompatible { .. } => {
                    MTBudgetRoute::OpenAICompatible
                }
                TextTranslationCredentials::ChatMock { .. } => MTBudgetRoute::ChatMock,
                TextTranslationCredentials::XAI { .. } => MTBudgetRoute::XAI,
            }
        } else if configuration.is_standalone_asr() {
            return None;
        } else {
            match &configuration.credentials {
                ProviderCredentials::DeepL { .. } => MTBudgetRoute::DeepL,
                ProviderCredentials::DeepLX { .. } => MTBudgetRoute::DeepLX,
                ProviderCredentials::OpenAICompatible { .. } => MTBudgetRoute::OpenAICompatible,
                ProviderCredentials::ChatMock { .. } => MTBudgetRoute::ChatMock,
                ProviderCredentials::ApiKey { .. }
                    if configuration.provider == ProviderKind::AlibabaCloud =>
                {
                    let model = match configuration.effective_translation_mode() {
                        TranslationMode::Turbo => configuration.qwen_mt_model,
                        TranslationMode::HighQuality => QwenMTModel::Plus,
                        TranslationMode::LowLatency => return None,
                    };
                    MTBudgetRoute::Qwen(model)
                }
                _ => return None,
            }
        };
        Some(Self {
            profile_id,
            provider: configuration.provider,
            route,
        })
    }
}

/// One bounded continuation slot. Its token rejects late teardown snapshots
/// after stop/new-start or after another client/route has already been prepared.
#[derive(Default)]
pub struct MTBudgetContinuity {
    token: u64,
    client_scope: Option<(u64, u64, MTBudgetScope)>,
    retained: Option<(MTBudgetScope, MTRequestBudget)>,
}

impl MTBudgetContinuity {
    pub fn reset(&mut self) {
        self.token = self.token.wrapping_add(1).max(1);
        self.client_scope = None;
        self.retained = None;
    }

    pub fn prepare(
        &mut self,
        generation: u64,
        scope: Option<MTBudgetScope>,
    ) -> Option<MTRequestBudget> {
        let Some(scope) = scope else {
            self.reset();
            return None;
        };
        if self
            .retained
            .as_ref()
            .is_some_and(|(previous, _)| previous != &scope)
        {
            self.retained = None;
        }
        self.token = self.token.wrapping_add(1).max(1);
        self.client_scope = Some((generation, self.token, scope));
        self.retained.as_ref().map(|(_, budget)| *budget)
    }

    pub fn take_lease(&mut self, generation: u64) -> Option<(u64, MTBudgetScope)> {
        if self
            .client_scope
            .as_ref()
            .is_none_or(|(owner, _, _)| *owner != generation)
        {
            return None;
        }
        self.client_scope
            .take()
            .map(|(_, token, scope)| (token, scope))
    }

    pub fn remember(&mut self, token: u64, scope: MTBudgetScope, budget: MTRequestBudget) {
        if token == self.token {
            self.retained = Some((scope, budget));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::{SourceLanguage, TargetLanguage};
    use crate::core::protocols::translation::qwen_mt::REALTIME_MT_MODEL;
    use crate::core::services::provider::XAISpeechMode;
    use std::time::{Duration, Instant};

    #[test]
    fn xai_asr_budget_uses_the_selected_text_route_and_original_has_no_budget() {
        let configuration = LiveTranslationConfiguration::for_provider(
            ProviderKind::XAIRealtime,
            "speech-key",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            TranslationMode::Turbo,
        )
        .with_xai_speech(XAISpeechMode::AsrMt, "legacy-unused");
        assert!(MTBudgetScope::for_configuration("profile".into(), &configuration).is_none());
        for (credentials, route) in [
            (
                TextTranslationCredentials::DeepL {
                    api_key: "text:fx".into(),
                },
                MTBudgetRoute::DeepL,
            ),
            (
                TextTranslationCredentials::OpenAICompatible {
                    endpoint: "http://127.0.0.1:1".into(),
                    model: "model".into(),
                    api_key: "text".into(),
                },
                MTBudgetRoute::OpenAICompatible,
            ),
            (
                TextTranslationCredentials::XAI {
                    api_key: "text".into(),
                    model: "grok".into(),
                },
                MTBudgetRoute::XAI,
            ),
        ] {
            let mut configuration = configuration.clone().with_text_credentials(credentials);
            assert!(
                MTBudgetScope::for_configuration("profile".into(), &configuration)
                    .unwrap()
                    .route
                    == route
            );
            configuration.target_language = TargetLanguage::Original;
            assert!(MTBudgetScope::for_configuration("profile".into(), &configuration).is_none());
        }
    }

    #[test]
    fn custom_speech_budget_tracks_only_the_independent_translation_route() {
        use crate::core::models::TargetLanguage;
        for provider in [
            ProviderKind::CustomDashScopeASR,
            ProviderKind::CustomOpenAIASR,
        ] {
            let configuration = LiveTranslationConfiguration::with_credentials(
                provider,
                ProviderCredentials::CustomSpeech {
                    endpoint: "wss://example.com/recognition".into(),
                    model: "synthetic-recognition-model".into(),
                    api_key: "synthetic-recognition-key".into(),
                },
                SourceLanguage::Automatic,
                TargetLanguage::Japanese,
                TranslationMode::Turbo,
            );
            assert!(
                MTBudgetScope::for_configuration("synthetic-profile".into(), &configuration)
                    .is_none()
            );
            for (credentials, expected_route) in [
                (
                    TextTranslationCredentials::DeepL {
                        api_key: "synthetic-text-key".into(),
                    },
                    MTBudgetRoute::DeepL,
                ),
                (
                    TextTranslationCredentials::DeepLX {
                        endpoint: "https://example.com/translate".into(),
                        token: String::new(),
                    },
                    MTBudgetRoute::DeepLX,
                ),
                (
                    TextTranslationCredentials::OpenAICompatible {
                        endpoint: "https://example.com/v1".into(),
                        model: "synthetic-text-model".into(),
                        api_key: "synthetic-text-key".into(),
                    },
                    MTBudgetRoute::OpenAICompatible,
                ),
                (
                    TextTranslationCredentials::ChatMock {
                        endpoint: "https://example.com/v1".into(),
                        model: "synthetic-text-model".into(),
                        api_key: "synthetic-text-key".into(),
                    },
                    MTBudgetRoute::ChatMock,
                ),
            ] {
                let mut selected = configuration.clone().with_text_credentials(credentials);
                let scope = MTBudgetScope::for_configuration("synthetic-profile".into(), &selected)
                    .unwrap();
                assert!(scope.route == expected_route);
                assert_eq!(scope.provider, provider);
                assert_eq!(scope.profile_id, "synthetic-profile");
                selected.target_language = TargetLanguage::Original;
                assert!(
                    MTBudgetScope::for_configuration("synthetic-profile".into(), &selected)
                        .is_none()
                );
            }
        }
    }

    #[test]
    fn chatmock_and_generic_compatible_routes_never_share_budget_continuations() {
        use crate::core::models::TargetLanguage;
        use crate::core::subtitles::preview_pacing::PreviewRequestPacer;
        let scope = |chatmock: bool| {
            let credentials = if chatmock {
                ProviderCredentials::ChatMock {
                    asr_api_key: "synthetic-asr".into(),
                    endpoint: "https://example.com/v1".into(),
                    api_key: "synthetic-text-key".into(),
                    model: "synthetic-model".into(),
                }
            } else {
                ProviderCredentials::OpenAICompatible {
                    asr_api_key: "synthetic-asr".into(),
                    endpoint: "https://example.com/v1".into(),
                    api_key: "synthetic-text-key".into(),
                    model: "synthetic-model".into(),
                }
            };
            MTBudgetScope::for_configuration(
                "same-profile".into(),
                &LiveTranslationConfiguration::with_credentials(
                    ProviderKind::AlibabaCloud,
                    credentials,
                    SourceLanguage::Automatic,
                    TargetLanguage::Japanese,
                    TranslationMode::Turbo,
                ),
            )
            .unwrap()
        };
        for chatmock_first in [false, true] {
            let first = scope(chatmock_first);
            let other = scope(!chatmock_first);
            assert!(first.route != other.route);
            let mut continuity = MTBudgetContinuity::default();
            assert!(continuity.prepare(1, Some(first.clone())).is_none());
            let (old_token, old_scope) = continuity.take_lease(1).unwrap();
            continuity.remember(
                old_token,
                old_scope,
                PreviewRequestPacer::default().export_budget(),
            );
            assert!(continuity.prepare(2, Some(other.clone())).is_none());
            continuity.remember(
                old_token,
                first,
                PreviewRequestPacer::default().export_budget(),
            );
            assert!(
                continuity.retained.is_none(),
                "old route teardown cannot restore its budget"
            );
            let (token, owned_scope) = continuity.take_lease(2).unwrap();
            continuity.remember(
                token,
                owned_scope,
                PreviewRequestPacer::default().export_budget(),
            );
            assert!(
                continuity.prepare(3, Some(other)).is_some(),
                "same route keeps its own continuation"
            );
        }
    }

    #[test]
    fn pause_and_reconnect_keep_same_profile_budget_but_stop_rejects_late_teardown() {
        use crate::core::subtitles::preview_pacing::{
            PreviewCandidate, PreviewRequestPacer, RATE_LIMIT_PREVIEW_PAUSE,
        };
        let now = Instant::now();
        let mut pacer = PreviewRequestPacer::default();
        pacer.record_candidate_start(
            now,
            PreviewCandidate::new("Synthetic request", Some("en"), 0),
        );
        pacer.suppress_after_rate_limit(now);
        pacer.set_shared_cooldown(Some(now + Duration::from_secs(8)));
        let mut continuity = MTBudgetContinuity::default();
        let scope = synthetic_mt_scope("fixture-profile", MTBudgetRoute::Qwen(REALTIME_MT_MODEL));
        assert!(continuity.prepare(7, Some(scope.clone())).is_none());
        let (token, owned_scope) = continuity.take_lease(7).unwrap();
        continuity.remember(token, owned_scope, pacer.export_budget());
        let restored = continuity.prepare(8, Some(scope.clone())).unwrap();
        let mut resumed = PreviewRequestPacer::default();
        resumed.restore_budget(restored);
        assert_eq!(resumed.suppression_remaining(now), RATE_LIMIT_PREVIEW_PAUSE);
        assert_eq!(
            resumed.next_shared_start_at(now),
            now + Duration::from_secs(8)
        );
        let (old_token, old_scope) = continuity.take_lease(8).unwrap();
        continuity.reset();
        continuity.remember(old_token, old_scope, restored);
        assert!(
            continuity.prepare(9, Some(scope)).is_none(),
            "late paused/recovery export must not undo explicit stop"
        );
    }

    #[test]
    fn mt_budget_continuation_never_crosses_profiles_routes_models_or_new_client_tokens() {
        use crate::core::subtitles::preview_pacing::PreviewRequestPacer;
        let scope = synthetic_mt_scope("fixture-profile", MTBudgetRoute::Qwen(REALTIME_MT_MODEL));
        for different in [
            synthetic_mt_scope("other-profile", MTBudgetRoute::Qwen(REALTIME_MT_MODEL)),
            synthetic_mt_scope("fixture-profile", MTBudgetRoute::DeepL),
            synthetic_mt_scope("fixture-profile", MTBudgetRoute::Qwen(QwenMTModel::Plus)),
        ] {
            let mut continuity = MTBudgetContinuity::default();
            continuity.prepare(7, Some(scope.clone()));
            let (token, owned_scope) = continuity.take_lease(7).unwrap();
            continuity.remember(
                token,
                owned_scope,
                PreviewRequestPacer::default().export_budget(),
            );
            assert!(continuity.prepare(8, Some(different)).is_none());
            continuity.remember(
                token,
                scope.clone(),
                PreviewRequestPacer::default().export_budget(),
            );
            assert!(
                continuity.retained.is_none(),
                "superseded old-client export must remain rejected"
            );
        }
    }

    fn synthetic_mt_scope(profile_id: &str, route: MTBudgetRoute) -> MTBudgetScope {
        MTBudgetScope {
            profile_id: profile_id.into(),
            provider: ProviderKind::AlibabaCloud,
            route,
        }
    }
}
