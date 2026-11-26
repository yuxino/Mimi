use super::*;
use crate::apple_speech::AppleSpeechLocale;

fn locale(identifier: &str, installed: bool) -> AppleSpeechLocale {
    AppleSpeechLocale {
        identifier: identifier.into(),
        status: if installed {
            AppleSpeechResourceStatus::Installed
        } else {
            AppleSpeechResourceStatus::Supported
        },
    }
}

#[tokio::test]
async fn snapshot_loads_share_completed_attempts_and_retry_failed_scans() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let ready = map_capabilities(AppleSpeechCapabilities {
        available: true,
        locales: vec![locale("en-US", true)],
    });
    for initial in [
        Ok(ready.clone()),
        Ok(AppleSpeechSupport::default()),
        Err("apple_speech_status_failed".into()),
    ] {
        let cache = Mutex::new(SupportCache::default());
        let refreshing = tokio::sync::Mutex::new(());
        let held = refreshing.lock().await;
        let queries = AtomicUsize::new(0);
        let mut first = Box::pin(load_support(&cache, &refreshing, true, async {
            queries.fetch_add(1, Ordering::SeqCst);
            initial.clone()
        }));
        let mut second = Box::pin(load_support(&cache, &refreshing, true, async {
            queries.fetch_add(1, Ordering::SeqCst);
            initial.clone()
        }));
        assert!(futures_util::poll!(first.as_mut()).is_pending());
        assert!(futures_util::poll!(second.as_mut()).is_pending());
        drop(held);
        let (first, second) = tokio::join!(first, second);
        assert_eq!(first, initial);
        assert_eq!(second, initial);
        assert_eq!(
            queries.load(Ordering::SeqCst),
            1,
            "Concurrent snapshots must share the completed query, including failure"
        );

        let retry = load_support(&cache, &refreshing, true, async {
            queries.fetch_add(1, Ordering::SeqCst);
            Ok(ready.clone())
        })
        .await;
        assert_eq!(retry, Ok(initial.clone().unwrap_or_else(|_| ready.clone())));
        assert_eq!(
            queries.load(Ordering::SeqCst),
            if initial.is_ok() { 1 } else { 2 },
            "Only a new request after failure should start another scan"
        );
        assert!(cache.lock().unwrap().is_loaded());
    }
}

#[tokio::test]
async fn explicit_refreshes_serialize_before_publishing_the_newest_inventory() {
    let ready = map_capabilities(AppleSpeechCapabilities {
        available: true,
        locales: vec![locale("en-US", true)],
    });
    let cache = Mutex::new(SupportCache::default());
    let refreshing = tokio::sync::Mutex::new(());
    let (release, released) = tokio::sync::oneshot::channel();
    let mut first = Box::pin(load_support(&cache, &refreshing, false, async {
        released.await.unwrap();
        Err("apple_speech_status_failed".into())
    }));
    assert!(futures_util::poll!(first.as_mut()).is_pending());
    let mut second = Box::pin(load_support(&cache, &refreshing, false, async {
        Ok(ready.clone())
    }));
    assert!(
        futures_util::poll!(second.as_mut()).is_pending(),
        "A newer refresh must wait for the older native query"
    );
    release.send(()).unwrap();
    let (first, second) = tokio::join!(first, second);
    assert!(first.is_err());
    assert_eq!(second, Ok(ready.clone()));
    assert_eq!(cache.lock().unwrap().snapshot().0, ready);
    assert!(cache.lock().unwrap().is_loaded());
}

#[tokio::test]
async fn a_cancelled_inventory_query_releases_the_next_snapshot_retry() {
    let ready = map_capabilities(AppleSpeechCapabilities {
        available: true,
        locales: vec![locale("en-US", true)],
    });
    let cache = Mutex::new(SupportCache::default());
    let refreshing = tokio::sync::Mutex::new(());
    let mut cancelled = Box::pin(load_support(
        &cache,
        &refreshing,
        true,
        std::future::pending(),
    ));
    assert!(futures_util::poll!(cancelled.as_mut()).is_pending());
    let mut retry = Box::pin(load_support(&cache, &refreshing, true, async {
        Ok(ready.clone())
    }));
    assert!(futures_util::poll!(retry.as_mut()).is_pending());
    drop(cancelled);
    assert_eq!(retry.await, Ok(ready));
    assert!(cache.lock().unwrap().is_loaded());
}

#[test]
fn failed_inventory_queries_clear_claims_and_remain_retryable() {
    let ready = map_capabilities(AppleSpeechCapabilities {
        available: true,
        locales: vec![locale("en-US", true), locale("ja-JP", false)],
    });
    let mut cache = SupportCache::default();
    assert!(!cache.is_loaded());
    let failed = Err("apple_speech_status_failed".into());
    cache.record_refresh(&failed);
    assert!(
        !cache.is_loaded(),
        "A failed initial scan must permit retry"
    );
    assert_eq!(cache.snapshot().0, AppleSpeechSupport::default());

    cache.record_refresh(&Ok(ready.clone()));
    assert!(cache.is_loaded());
    assert_eq!(cache.snapshot().0, ready);
    let ready_revision = cache.snapshot().1;
    cache.record_refresh(&failed);
    assert!(!cache.is_loaded(), "A failed rescan must permit retry");
    assert_eq!(
        cache.snapshot(),
        (AppleSpeechSupport::default(), ready_revision + 1)
    );
    let failed_revision = cache.snapshot().1;
    cache.record_refresh(&failed);
    assert_eq!(
        cache.snapshot().1,
        failed_revision,
        "Repeated failures must not republish the same empty inventory"
    );

    cache.record_refresh(&Ok(AppleSpeechSupport::default()));
    assert!(
        cache.is_loaded(),
        "A successful unavailable scan is authoritative"
    );
    assert_eq!(
        cache.snapshot(),
        (AppleSpeechSupport::default(), failed_revision)
    );
    cache.record_refresh(&Ok(ready.clone()));
    assert!(cache.is_loaded());
    assert_eq!(cache.snapshot(), (ready, failed_revision + 1));
}

#[test]
fn preparation_after_failed_inventory_does_not_mark_the_complete_scan_loaded() {
    let before = map_capabilities(AppleSpeechCapabilities {
        available: true,
        locales: vec![locale("en-US", true), locale("ja-JP", false)],
    });
    let mut cache = SupportCache::default();
    cache.record_refresh(&Ok(before.clone()));
    let before_failure = cache.refresh_attempt;
    cache.record_refresh(&Err("apple_speech_status_failed".into()));
    let after = cache.prepared_locale("ja-JP", before);
    assert!(
        !cache.is_loaded(),
        "One prepared locale cannot validate the complete inventory"
    );
    assert_eq!(
        cache.completed_since(before_failure),
        Some(Err("apple_speech_status_failed".into())),
        "Waiters must receive the failed complete scan, even after one locale is prepared"
    );
    assert_eq!(after.languages.len(), 1);
    assert!(
        require_language(&after, SourceLanguage::Japanese)
            .unwrap()
            .installed
    );
    assert!(require_language(&after, SourceLanguage::English).is_err());
    let prepared_revision = cache.snapshot().1;
    cache.record_refresh(&Ok(after.clone()));
    assert!(cache.is_loaded());
    assert_eq!(cache.snapshot(), (after, prepared_revision));
}

#[test]
fn only_supported_status_is_a_missing_resource() {
    for status in [
        AppleSpeechResourceStatus::Unsupported,
        AppleSpeechResourceStatus::Unknown,
    ] {
        let support = map_capabilities(AppleSpeechCapabilities {
            available: true,
            locales: vec![AppleSpeechLocale {
                identifier: "ja-JP".into(),
                status,
            }],
        });
        let language = require_language(&support, SourceLanguage::Japanese).unwrap();
        assert_eq!(language.status, status);
        assert!(!language.installed);
        assert!(!language.downloading);
        for installed_required in [false, true] {
            assert_eq!(
                validate_source(&support, SourceLanguage::Japanese, installed_required)
                    .unwrap_err(),
                "apple_speech_status_failed"
            );
        }
    }
    let support = map_capabilities(AppleSpeechCapabilities {
        available: true,
        locales: vec![locale("ja-JP", false)],
    });
    assert!(validate_source(&support, SourceLanguage::Japanese, false).is_ok());
    assert_eq!(
        validate_source(&support, SourceLanguage::Japanese, true).unwrap_err(),
        "apple_speech_assets_missing"
    );
}

#[test]
fn failed_readiness_invalidates_missing_claim_and_publishes_a_revision() {
    let before = map_capabilities(AppleSpeechCapabilities {
        available: true,
        locales: vec![locale("en-US", true), locale("ja-JP", false)],
    });
    let mut cache = SupportCache::default();
    cache.replace(before.clone());
    let revision = cache.snapshot().1;
    let after = cache.unconfirmed_locale("ja-JP", &before);
    assert_eq!(cache.snapshot().1, revision + 1);
    assert!(
        require_language(&after, SourceLanguage::English)
            .unwrap()
            .installed
    );
    let uncertain = require_language(&after, SourceLanguage::Japanese).unwrap();
    assert_eq!(uncertain.status, AppleSpeechResourceStatus::Unknown);
    assert!(!uncertain.installed);
    assert!(!uncertain.downloading);
    assert_eq!(
        validate_source(&after, SourceLanguage::Japanese, true).unwrap_err(),
        "apple_speech_status_failed"
    );
    cache.unconfirmed_locale("ja-JP", &before);
    assert_eq!(cache.snapshot().1, revision + 1);
}

#[test]
fn failed_readiness_preserves_newer_download_or_install_confirmation() {
    let before = map_capabilities(AppleSpeechCapabilities {
        available: true,
        locales: vec![locale("ja-JP", false)],
    });
    for status in [
        AppleSpeechResourceStatus::Installed,
        AppleSpeechResourceStatus::Downloading,
    ] {
        let mut cache = SupportCache::default();
        cache.replace(map_capabilities(AppleSpeechCapabilities {
            available: true,
            locales: vec![AppleSpeechLocale {
                identifier: "ja-JP".into(),
                status,
            }],
        }));
        let revision = cache.snapshot().1;
        let after = cache.unconfirmed_locale("ja-JP", &before);
        assert_eq!(
            require_language(&after, SourceLanguage::Japanese)
                .unwrap()
                .status,
            status
        );
        assert_eq!(cache.snapshot().1, revision);
    }
}

#[test]
fn prepared_locale_keeps_newer_status_of_other_languages() {
    let before = map_capabilities(AppleSpeechCapabilities {
        available: true,
        locales: vec![locale("en-US", true), locale("ja-JP", false)],
    });
    let mut cache = SupportCache::default();
    cache.replace(map_capabilities(AppleSpeechCapabilities {
        available: true,
        locales: vec![
            locale("en-US", false),
            locale("ja-JP", false),
            locale("fr-FR", true),
        ],
    }));
    let after = cache.prepared_locale("ja-JP", before);
    assert!(
        require_language(&after, SourceLanguage::Japanese)
            .unwrap()
            .installed
    );
    assert!(
        !require_language(&after, SourceLanguage::English)
            .unwrap()
            .installed
    );
    assert!(
        require_language(&after, SourceLanguage::French)
            .unwrap()
            .installed
    );
}

#[test]
fn prepared_locale_does_not_restore_unrelated_stale_readiness() {
    let before = map_capabilities(AppleSpeechCapabilities {
        available: true,
        locales: vec![locale("en-US", true), locale("ja-JP", false)],
    });
    let mut cache = SupportCache::default();
    cache.replace(AppleSpeechSupport::default());
    let after = cache.prepared_locale("ja-JP", before);
    assert!(after.available);
    assert_eq!(after.languages.len(), 1);
    assert!(
        require_language(&after, SourceLanguage::Japanese)
            .unwrap()
            .installed
    );
    assert!(require_language(&after, SourceLanguage::English).is_err());
}

#[test]
fn background_download_is_distinct_from_missing_or_installed_assets() {
    let mut pending = locale("ja-JP", false);
    pending.status = AppleSpeechResourceStatus::Downloading;
    let support = map_capabilities(AppleSpeechCapabilities {
        available: true,
        locales: vec![pending],
    });
    assert!(
        require_language(&support, SourceLanguage::Japanese)
            .unwrap()
            .downloading
    );
    assert_eq!(
        validate_source(&support, SourceLanguage::Japanese, true).unwrap_err(),
        "apple_speech_preparing"
    );
    let mut cache = SupportCache::default();
    cache.replace(support.clone());
    let after = cache.prepared_locale("ja-JP", support);
    let ready = validate_source(&after, SourceLanguage::Japanese, true).unwrap();
    assert!(ready.installed);
    assert!(!ready.downloading);
}

#[test]
fn preparation_failures_keep_actionable_safe_categories() {
    for (error, expected) in [
        (
            AppleSpeechError::AssetsDownloading,
            "apple_speech_preparing",
        ),
        (
            AppleSpeechError::ServiceUnavailable,
            "apple_speech_service_unavailable",
        ),
        (
            AppleSpeechError::ReservationLimit,
            "apple_speech_reservation_limit",
        ),
        (
            AppleSpeechError::ResourcesUnavailable,
            "apple_speech_resources_unavailable",
        ),
        (
            AppleSpeechError::DownloadCancelled,
            "apple_speech_download_cancelled",
        ),
        (
            AppleSpeechError::DownloadNetwork,
            "apple_speech_download_network",
        ),
        (
            AppleSpeechError::DownloadStorage,
            "apple_speech_download_storage",
        ),
        (
            AppleSpeechError::StatusUnavailable,
            "apple_speech_status_failed",
        ),
        (AppleSpeechError::Timeout, "apple_speech_download_timeout"),
        (
            AppleSpeechError::Native {
                domain: "private/path/content".into(),
                code: 9,
            },
            "apple_speech_prepare_failed",
        ),
    ] {
        assert_eq!(prepare_error_label(&error), expected);
    }
}

#[test]
fn equivalent_refreshes_keep_the_complete_support_revision_stable() {
    let mut cache = SupportCache::default();
    assert!(cache.support.is_none());
    assert_eq!(cache.snapshot(), (AppleSpeechSupport::default(), 0));
    cache.replace(map_capabilities(AppleSpeechCapabilities {
        available: true,
        locales: vec![locale("en-US", true), locale("ja-JP", false)],
    }));
    let first = cache.snapshot();
    assert_eq!(first.1, 1);
    // Native ordering has no semantic effect after locale mapping.
    cache.replace(map_capabilities(AppleSpeechCapabilities {
        available: true,
        locales: vec![locale("ja-JP", false), locale("en-US", true)],
    }));
    assert_eq!(cache.snapshot(), first);
    // Repeated failed/unavailable refreshes must not create event loops.
    cache.replace(AppleSpeechSupport::default());
    assert_eq!(cache.snapshot(), (AppleSpeechSupport::default(), 2));
    cache.replace(AppleSpeechSupport::default());
    assert_eq!(cache.snapshot(), (AppleSpeechSupport::default(), 2));
}

#[test]
fn support_revision_changes_even_when_the_route_ready_list_does_not() {
    use crate::core::services::provider::{ProviderKind, TextTranslation};
    let mut profile =
        ServiceProfile::new("synthetic", "Synthetic", ProviderKind::AppleSpeech).unwrap();
    profile.text_translation = Some(TextTranslation::DeepL);
    let ready_sources = |support: &AppleSpeechSupport| {
        profile
            .capabilities(TargetLanguage::English)
            .source_languages
            .into_iter()
            .filter(|source| validate_source(support, *source, true).is_ok())
            .collect::<Vec<_>>()
    };
    let mut cache = SupportCache::default();
    let mut support = map_capabilities(AppleSpeechCapabilities {
        available: true,
        locales: vec![locale("en-US", true), locale("km-KH", true)],
    });
    cache.replace(support.clone());
    let before = cache.snapshot();
    assert_eq!(ready_sources(&before.0), vec![SourceLanguage::English]);
    support
        .languages
        .iter_mut()
        .find(|language| language.source_language == SourceLanguage::Khmer)
        .unwrap()
        .set_status(AppleSpeechResourceStatus::Supported);
    cache.replace(support.clone());
    let after = cache.snapshot();
    assert_eq!(ready_sources(&after.0), ready_sources(&before.0));
    assert_eq!(after.1, before.1 + 1);

    support
        .languages
        .iter_mut()
        .for_each(|language| language.set_status(AppleSpeechResourceStatus::Supported));
    cache.replace(support);
    let available_without_ready_assets = cache.snapshot();
    assert!(available_without_ready_assets.0.available);
    assert!(ready_sources(&available_without_ready_assets.0).is_empty());
    cache.replace(AppleSpeechSupport::default());
    let unavailable = cache.snapshot();
    assert!(!unavailable.0.available);
    assert!(ready_sources(&unavailable.0).is_empty());
    assert_eq!(unavailable.1, available_without_ready_assets.1 + 1);
}

#[test]
fn live_language_selection_distinguishes_missing_assets_from_unsupported_locales() {
    let support = map_capabilities(AppleSpeechCapabilities {
        available: true,
        locales: vec![locale("en-US", true), locale("ja-JP", false)],
    });
    assert!(validate_source(&support, SourceLanguage::English, true).is_ok());
    assert!(validate_source(&support, SourceLanguage::Japanese, false).is_ok());
    assert_eq!(
        validate_source(&support, SourceLanguage::Japanese, true).unwrap_err(),
        "apple_speech_assets_missing"
    );
    assert_eq!(
        validate_source(&support, SourceLanguage::French, true).unwrap_err(),
        "apple_speech_language_unsupported"
    );
    assert_eq!(
        validate_source(
            &AppleSpeechSupport::default(),
            SourceLanguage::English,
            true
        )
        .unwrap_err(),
        "apple_speech_unavailable"
    );
}

#[test]
fn rejects_incompatible_text_routes_without_inventing_an_os_language() {
    use crate::core::services::provider::{ProviderKind, TextTranslation};
    let support = map_capabilities(AppleSpeechCapabilities {
        available: true,
        locales: vec![locale("en-US", true), locale("km-KH", true)],
    });
    for route in [TextTranslation::DeepL, TextTranslation::DeepLX] {
        let mut profile =
            ServiceProfile::new("synthetic", "Synthetic", ProviderKind::AppleSpeech).unwrap();
        profile.text_translation = Some(route);
        assert!(validate_profile_source(
            &support,
            &profile,
            SourceLanguage::Khmer,
            TargetLanguage::Original,
            true,
        )
        .is_ok());
        assert_eq!(
            validate_profile_source(
                &support,
                &profile,
                SourceLanguage::Khmer,
                TargetLanguage::English,
                true,
            )
            .unwrap_err(),
            "apple_speech_translation_language_unsupported"
        );
        assert!(validate_profile_source(
            &support,
            &profile,
            SourceLanguage::English,
            TargetLanguage::English,
            true,
        )
        .is_ok());
        assert_eq!(
            validate_profile_source(
                &support,
                &profile,
                SourceLanguage::Chinese,
                TargetLanguage::English,
                true,
            )
            .unwrap_err(),
            "apple_speech_language_unsupported"
        );
    }
}

#[test]
fn rejected_refreshed_selections_publish_capabilities_and_preserve_the_actual_error() {
    use crate::core::services::provider::{ProviderKind, TextTranslation};
    let mut profile =
        ServiceProfile::new("synthetic", "Synthetic", ProviderKind::AppleSpeech).unwrap();
    profile.text_translation = Some(TextTranslation::DeepL);
    let support = map_capabilities(AppleSpeechCapabilities {
        available: true,
        locales: vec![
            locale("en-US", true),
            locale("ja-JP", false),
            locale("km-KH", true),
        ],
    });
    for (refreshed, source, expected) in [
        (
            Err("apple_speech_status_failed".to_string()),
            SourceLanguage::English,
            "apple_speech_status_failed",
        ),
        (
            Ok(AppleSpeechSupport::default()),
            SourceLanguage::English,
            "apple_speech_unavailable",
        ),
        (
            Ok(support.clone()),
            SourceLanguage::Japanese,
            "apple_speech_assets_missing",
        ),
        (
            Ok(support.clone()),
            SourceLanguage::Chinese,
            "apple_speech_language_unsupported",
        ),
        (
            Ok(support.clone()),
            SourceLanguage::Khmer,
            "apple_speech_translation_language_unsupported",
        ),
    ] {
        let mut publications = 0;
        let result = validate_refreshed_profile_source(
            refreshed,
            &profile,
            source,
            TargetLanguage::English,
            true,
            || publications += 1,
        );
        assert_eq!(result.unwrap_err(), expected);
        assert_eq!(publications, 1);
    }
    // Accepted changes keep the normal publish-after-save boundary. Idle
    // preferences may still name a supported language before preparation.
    for (source, installed_required) in [
        (SourceLanguage::English, true),
        (SourceLanguage::Japanese, false),
    ] {
        assert!(validate_refreshed_profile_source(
            Ok(support.clone()),
            &profile,
            source,
            TargetLanguage::English,
            installed_required,
            || panic!("successful validation published before saving"),
        )
        .is_ok());
    }
}

#[test]
fn hides_unavailable_devices_and_unrepresentable_languages() {
    for native in [
        AppleSpeechCapabilities {
            available: false,
            locales: vec![locale("en-US", true)],
        },
        AppleSpeechCapabilities {
            available: true,
            locales: vec![locale("zz-XX", true)],
        },
    ] {
        assert_eq!(map_capabilities(native), AppleSpeechSupport::default());
    }
}

#[test]
fn language_choices_are_dynamic_explicit_and_region_stable() {
    let support = map_capabilities(AppleSpeechCapabilities {
        available: true,
        locales: vec![
            locale("en_AU", true),
            locale("en_US", false),
            locale("ja_JP", true),
            locale("fr-FR", true),
            locale("yue-HK", true),
        ],
    });
    assert!(support.available);
    assert_eq!(support.languages.len(), 5);
    let english = require_language(&support, SourceLanguage::English).unwrap();
    assert_eq!(english.locale, "en_US");
    assert!(!english.installed);
    assert!(require_language(&support, SourceLanguage::Automatic).is_err());
    assert!(require_language(&support, SourceLanguage::Chinese).is_err());
    assert_eq!(
        require_language(&support, SourceLanguage::Japanese)
            .unwrap()
            .locale,
        "ja_JP"
    );
}
