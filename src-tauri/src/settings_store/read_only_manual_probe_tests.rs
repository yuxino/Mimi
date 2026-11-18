//! Read-only manual probe regressions; never access native credential stores.

use super::*;
use crate::core::models::{SourceLanguage, TranslationMode};
use crate::core::services::provider::ProviderKind;
use std::os::unix::fs::{MetadataExt, PermissionsExt};

fn fixture() -> (tempfile::TempDir, PathBuf) {
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.path().join(DEVELOPMENT_APPLICATION_IDENTIFIER);
    let credentials_directory = directory.join("credentials");
    std::fs::create_dir_all(&credentials_directory).unwrap();
    std::fs::set_permissions(
        &credentials_directory,
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let prefs = Preferences {
        source_language: SourceLanguage::English,
        translation_mode: TranslationMode::HighQuality,
        font_size: 90.0,
        subtitle_font_family: "  Synthetic Font  ".into(),
        subtitle_background_opacity: 200,
        ..Preferences::default()
    };
    let profile = ServiceProfile::alibaba_default();
    for (name, bytes) in [
        ("preferences.json", serde_json::to_vec(&prefs).unwrap()),
        (
            PROFILE_CATALOG_FILE,
            serde_json::to_vec(&ProfileCatalog::legacy_alibaba()).unwrap(),
        ),
        ("credentials-file-mode-complete", b"1\n".to_vec()),
        (
            "credentials/credentials.json",
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion": 1,
                "entries": {
                    DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE: {
                        (credential_account(&profile)): "synthetic-read-only-key"
                    }
                },
                "pendingImport": [],
                "pendingCleanup": []
            }))
            .unwrap(),
        ),
    ] {
        let file = directory.join(name);
        std::fs::write(&file, bytes).unwrap();
        std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    (temporary, directory)
}

fn snapshot(directory: &Path) -> Vec<(Vec<u8>, u32, u64, i64, i64)> {
    [
        "preferences.json",
        PROFILE_CATALOG_FILE,
        "credentials-file-mode-complete",
        "credentials/credentials.json",
    ]
    .into_iter()
    .map(|name| {
        let file = directory.join(name);
        let metadata = std::fs::symlink_metadata(&file).unwrap();
        (
            std::fs::read(file).unwrap(),
            metadata.mode(),
            metadata.ino(),
            metadata.mtime(),
            metadata.mtime_nsec(),
        )
    })
    .collect()
}

#[test]
fn production_manual_probe_is_explicit_read_only_and_has_no_development_fallback() {
    let (temporary, development) = fixture();
    let production = temporary.path().join("app.yuxino.mimi");
    std::fs::rename(&development, &production).unwrap();
    let file = production.join("credentials/credentials.json");
    let original = std::fs::read_to_string(&file).unwrap();
    std::fs::write(
        &file,
        original.replace(
            DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE,
            PROFILE_KEYCHAIN_SERVICE,
        ),
    )
    .unwrap();
    let before = snapshot(&production);
    assert!(SettingsStore::load_for_read_only_manual_probe(production.clone()).is_err());
    assert!(
        SettingsStore::load_for_scoped_read_only_manual_probe(production.clone(), "unknown")
            .is_err()
    );
    let store = SettingsStore::load_for_scoped_read_only_manual_probe(
        production.clone(),
        "app.yuxino.mimi",
    )
    .unwrap();
    let profile = store.active_profile().unwrap();
    assert_eq!(
        store
            .configuration_for_profile_probe(&profile)
            .unwrap()
            .credentials
            .alibaba_key(),
        Some("synthetic-read-only-key")
    );
    assert!(store
        .save_api_key(&profile.id, "synthetic-replacement")
        .is_err());
    assert!(store
        .save_preferences(|prefs| prefs.font_size = 19.0)
        .is_err());
    assert!(store.secret.migrate_legacy().is_err());
    assert_eq!(snapshot(&production), before);
    // An entry from another app namespace is never a fallback.
    std::fs::write(&file, original).unwrap();
    let store =
        SettingsStore::load_for_scoped_read_only_manual_probe(production, "app.yuxino.mimi")
            .unwrap();
    assert!(store.configuration_for_profile_probe(&profile).is_err());
}

#[test]
fn normalization_and_configuration_read_never_write_back() {
    let (_temporary, directory) = fixture();
    let before = snapshot(&directory);
    let store = SettingsStore::load_for_read_only_manual_probe(directory.clone()).unwrap();
    let prefs = store.preferences();
    assert_eq!(prefs.translation_mode, TranslationMode::Turbo);
    assert_eq!(prefs.font_size, 48.0);
    assert_eq!(prefs.subtitle_font_family, "Synthetic Font");
    assert_eq!(prefs.subtitle_background_opacity, 100);
    let profile = store.active_profile().unwrap();
    let configuration = store.configuration_for_profile_probe(&profile).unwrap();
    assert!(configuration.credentials.alibaba_key() == Some("synthetic-read-only-key"));
    assert!(store
        .save_preferences(|prefs| prefs.font_size = 19.0)
        .is_err());
    assert!(store.update_profile(&profile.id, "Changed").is_err());
    assert!(store
        .save_api_key(&profile.id, "synthetic-replacement")
        .is_err());
    assert!(store.delete_api_key(&profile.id).is_err());
    assert!(store.secret.migrate_legacy().is_err());
    assert_eq!(snapshot(&directory), before);
}

#[test]
fn unrelated_legacy_xai_profile_does_not_block_read_only_catalog() {
    let (_temporary, directory) = fixture();
    let path = directory.join(PROFILE_CATALOG_FILE);
    let mut catalog: ProfileCatalog =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let mut legacy =
        ServiceProfile::new("legacy-xai", "Legacy", ProviderKind::XAIRealtime).unwrap();
    legacy.xai_speech_mode = crate::core::services::provider::XAISpeechMode::AsrMt;
    legacy.xai_independent_translation_migrated = false;
    legacy.text_translation = None;
    catalog.profiles.push(legacy);
    std::fs::write(&path, serde_json::to_vec(&catalog).unwrap()).unwrap();
    let before = snapshot(&directory);
    let store = SettingsStore::load_for_read_only_manual_probe(directory.clone()).unwrap();
    let (_, profiles) = store.profile_catalog().unwrap();
    assert_eq!(profiles.len(), 2);
    let legacy = profiles.iter().find(|p| p.id == "legacy-xai").unwrap();
    assert!(!legacy.xai_independent_translation_migrated);
    assert_eq!(legacy.text_translation, None);
    assert!(store
        .configuration_for_profile_probe(&store.active_profile().unwrap())
        .is_ok());
    assert_eq!(snapshot(&directory), before);
}

#[test]
fn missing_required_files_fail_closed_without_creating_them() {
    for missing in [
        "preferences.json",
        PROFILE_CATALOG_FILE,
        "credentials-file-mode-complete",
        "credentials/credentials.json",
    ] {
        let (_temporary, directory) = fixture();
        let removed = directory.join(missing);
        std::fs::remove_file(&removed).unwrap();
        assert!(SettingsStore::load_for_read_only_manual_probe(directory).is_err());
        assert!(!removed.exists());
    }
}

#[test]
fn unsafe_namespace_symlinks_and_permissions_are_rejected_without_repairs() {
    let (_temporary, directory) = fixture();
    let other = directory.with_file_name("app.yuxino.mimi");
    std::fs::rename(&directory, &other).unwrap();
    assert!(SettingsStore::load_for_read_only_manual_probe(other.clone()).is_err());
    std::os::unix::fs::symlink(&other, &directory).unwrap();
    assert!(SettingsStore::load_for_read_only_manual_probe(directory).is_err());

    for name in [
        "preferences.json",
        PROFILE_CATALOG_FILE,
        "credentials-file-mode-complete",
        "credentials/credentials.json",
    ] {
        let (_temporary, directory) = fixture();
        let file = directory.join(name);
        let moved = directory.join("synthetic-moved-file");
        std::fs::rename(&file, &moved).unwrap();
        std::os::unix::fs::symlink(moved, file).unwrap();
        assert!(SettingsStore::load_for_read_only_manual_probe(directory).is_err());
    }
    let (_temporary, directory) = fixture();
    let credentials = directory.join("credentials");
    std::fs::set_permissions(&credentials, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(SettingsStore::load_for_read_only_manual_probe(directory).is_err());
    assert_eq!(
        std::fs::metadata(credentials).unwrap().mode() & 0o7777,
        0o755
    );
}

#[test]
fn disappearance_after_loading_never_reopens_import_or_creates_credentials() {
    for missing in [
        "credentials-file-mode-complete",
        "credentials/credentials.json",
    ] {
        let (_temporary, directory) = fixture();
        let store = SettingsStore::load_for_read_only_manual_probe(directory.clone()).unwrap();
        let removed = directory.join(missing);
        std::fs::remove_file(&removed).unwrap();
        assert_eq!(
            store.secret.pending_imports(),
            Err(SecretStoreError::Unavailable)
        );
        assert!(store
            .configuration_for_profile_probe(&store.active_profile().unwrap())
            .is_err());
        assert!(store.secret.migrate_legacy().is_err());
        assert!(!removed.exists());
    }
}
