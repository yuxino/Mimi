use super::*;
use std::sync::Arc;
#[derive(Default)]
struct LegacyState {
    values: HashMap<(String, String), String>,
    denied: HashSet<(String, String)>,
    reads: HashMap<(String, String), usize>,
    deletes: usize,
    deny_deletion: bool,
}
#[derive(Clone, Default)]
struct Legacy(Arc<Mutex<LegacyState>>);
impl SecretStore for Legacy {
    fn load(&self, service: &str, account: &str) -> Result<Option<String>, SecretStoreError> {
        let mut state = self.0.lock().unwrap();
        let key = (service.into(), account.into());
        *state.reads.entry(key.clone()).or_default() += 1;
        if state.denied.contains(&key) {
            return Err(SecretStoreError::Unavailable);
        }
        Ok(state.values.get(&key).cloned())
    }
    fn save(&self, _: &str, _: &str, _: &str) -> Result<(), SecretStoreError> {
        panic!("native write is forbidden")
    }
    fn delete(&self, service: &str, account: &str) -> Result<(), SecretStoreError> {
        let mut state = self.0.lock().unwrap();
        state.deletes += 1;
        if state.deny_deletion {
            return Err(SecretStoreError::Unavailable);
        }
        state.values.remove(&(service.into(), account.into()));
        Ok(())
    }
}
fn slot(account: &str) -> Slot {
    Slot {
        service: "test.service".into(),
        account: account.into(),
        legacy_alibaba: false,
    }
}
fn legacy_value(legacy: &Legacy, account: &str, value: &str) {
    legacy
        .0
        .lock()
        .unwrap()
        .values
        .insert(("test.service".into(), account.into()), value.into());
}
fn reads(legacy: &Legacy) -> usize {
    legacy.0.lock().unwrap().reads.values().sum()
}
#[test]
fn new_install_save_load_delete_and_restart_never_use_native_store() {
    let directory = tempfile::tempdir().unwrap();
    let legacy = Legacy::default();
    let store = FileCredentialStore::new(
        directory.path(),
        "test.service",
        vec![],
        Box::new(legacy.clone()),
    );
    store
        .save("test.service", "speech", "public-placeholder")
        .unwrap();
    store
        .save("test.service", "text", "public-text-placeholder")
        .unwrap();
    assert_eq!(
        store.load("test.service", "speech").unwrap().as_deref(),
        Some("public-placeholder")
    );
    store.delete("test.service", "speech").unwrap();
    drop(store);
    let store = FileCredentialStore::new(
        directory.path(),
        "test.service",
        vec![slot("speech")],
        Box::new(legacy.clone()),
    );
    store.migrate_legacy().unwrap();
    assert!(!store.contains("test.service", "speech").unwrap());
    assert!(store.contains("test.service", "text").unwrap());
    assert_eq!(store.pending_imports().unwrap(), 0);
    assert_eq!(reads(&legacy), 0);
}
#[test]
fn import_is_explicit_and_completed_slots_never_touch_native_storage_again() {
    let directory = tempfile::tempdir().unwrap();
    let legacy = Legacy::default();
    for name in ["ali", "baidu", "gemini", "speech", "text"] {
        legacy_value(&legacy, name, &format!("public-{name}-placeholder"));
    }
    let slots = ["ali", "baidu", "gemini", "speech", "text"]
        .map(slot)
        .to_vec();
    let store = FileCredentialStore::new(
        directory.path(),
        "test.service",
        slots.clone(),
        Box::new(legacy.clone()),
    );
    for value in &slots {
        assert_eq!(
            store.contains(&value.service, &value.account),
            Err(SecretStoreError::MigrationRequired)
        );
    }
    assert_eq!(reads(&legacy), 0);
    store.migrate_legacy().unwrap();
    assert_eq!(reads(&legacy), 5);
    drop(store);
    legacy.0.lock().unwrap().denied.extend(
        slots
            .iter()
            .map(|slot| (slot.service.clone(), slot.account.clone())),
    );
    let store = FileCredentialStore::new(
        directory.path(),
        "test.service",
        slots.clone(),
        Box::new(legacy.clone()),
    );
    for _ in 0..3 {
        store.migrate_legacy().unwrap();
        for slot in &slots {
            assert!(store.contains(&slot.service, &slot.account).unwrap());
            assert!(store.load(&slot.service, &slot.account).unwrap().is_some());
        }
    }
    store
        .save("test.service", "ali", "public-updated-placeholder")
        .unwrap();
    store.delete("test.service", "baidu").unwrap();
    assert_eq!(reads(&legacy), 5);
    assert_eq!(legacy.0.lock().unwrap().deletes, 5);
}
#[test]
fn denied_import_retains_only_pending_slot_and_retries_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let legacy = Legacy::default();
    legacy_value(&legacy, "a", "public-a");
    legacy_value(&legacy, "b", "public-b");
    legacy
        .0
        .lock()
        .unwrap()
        .denied
        .insert(("test.service".into(), "b".into()));
    let slots = ["a", "b", "missing"].map(slot).to_vec();
    let store = FileCredentialStore::new(
        directory.path(),
        "test.service",
        slots.clone(),
        Box::new(legacy.clone()),
    );
    assert_eq!(store.migrate_legacy(), Err(SecretStoreError::Unavailable));
    assert_eq!(store.pending_imports().unwrap(), 1);
    assert!(store.contains("test.service", "a").unwrap());
    assert!(!store.contains("test.service", "missing").unwrap());
    drop(store);
    legacy.0.lock().unwrap().denied.clear();
    let store = FileCredentialStore::new(
        directory.path(),
        "test.service",
        slots,
        Box::new(legacy.clone()),
    );
    store.migrate_legacy().unwrap();
    assert_eq!(reads(&legacy), 4);
    assert_eq!(store.pending_imports().unwrap(), 0);
    assert_eq!(legacy.0.lock().unwrap().values.len(), 0);
}
#[test]
fn replacement_and_deletion_before_import_retire_old_slots() {
    let directory = tempfile::tempdir().unwrap();
    let legacy = Legacy::default();
    legacy_value(&legacy, "replacement", "old-public-value");
    legacy_value(&legacy, "deleted", "old-public-value");
    let store = FileCredentialStore::new(
        directory.path(),
        "test.service",
        vec![slot("replacement"), slot("deleted")],
        Box::new(legacy.clone()),
    );
    store
        .save("test.service", "replacement", "new-public-value")
        .unwrap();
    store.delete("test.service", "deleted").unwrap();
    store.migrate_legacy().unwrap();
    assert_eq!(
        store
            .load("test.service", "replacement")
            .unwrap()
            .as_deref(),
        Some("new-public-value")
    );
    assert!(!store.contains("test.service", "deleted").unwrap());
    assert_eq!(reads(&legacy), 0);
}

fn verify_local_write_retires_import_after_restart_and_file_loss(delete: bool) {
    let directory = tempfile::tempdir().unwrap();
    let legacy = Legacy::default();
    legacy_value(&legacy, "key", "synthetic-old-native-value");
    let store = FileCredentialStore::new(
        directory.path(),
        "test.service",
        vec![slot("key")],
        Box::new(legacy.clone()),
    );
    if delete {
        store.delete("test.service", "key").unwrap();
    } else {
        store
            .save("test.service", "key", "synthetic-new-local-value")
            .unwrap();
    }
    assert_eq!(store.pending_imports().unwrap(), 0);
    assert_eq!(reads(&legacy), 0);
    drop(store);
    let restarted = FileCredentialStore::new(
        directory.path(),
        "test.service",
        vec![slot("key")],
        Box::new(legacy.clone()),
    );
    assert_eq!(
        restarted.load("test.service", "key").unwrap().as_deref(),
        if delete {
            None
        } else {
            Some("synthetic-new-local-value")
        }
    );
    fs::remove_file(&restarted.path).unwrap();
    drop(restarted);
    let missing = FileCredentialStore::new(
        directory.path(),
        "test.service",
        vec![slot("key")],
        Box::new(legacy.clone()),
    );
    // Match startup's automatic import gate without touching an OS store.
    if missing.pending_imports().unwrap() > 0 {
        missing.migrate_legacy().unwrap();
    }
    assert_eq!(reads(&legacy), 0);
    assert_eq!(missing.load("test.service", "key").unwrap(), None);
    assert!(missing.complete_path.is_file());
}

#[test]
fn normal_save_retires_import_after_restart_and_file_loss() {
    verify_local_write_retires_import_after_restart_and_file_loss(false);
}

#[test]
fn normal_delete_retires_import_after_restart_and_file_loss() {
    verify_local_write_retires_import_after_restart_and_file_loss(true);
}

#[test]
fn completion_marker_retries_after_write_failure_and_rejects_damaged_document() {
    let directory = tempfile::tempdir().unwrap();
    let legacy = Legacy::default();
    let mut store = FileCredentialStore::new(
        directory.path(),
        "test.service",
        Vec::new(),
        Box::new(legacy.clone()),
    );
    let blocked_parent = directory.path().join("synthetic-marker-parent");
    fs::write(&blocked_parent, b"synthetic path obstruction").unwrap();
    store.complete_path = blocked_parent.join("complete");
    assert_eq!(
        store.load("test.service", "key"),
        Err(SecretStoreError::Unavailable)
    );
    let valid_document = fs::read(&store.path).unwrap();
    assert!(serde_json::from_slice::<Document>(&valid_document).is_ok());
    assert!(!store.complete_path.exists());
    fs::remove_file(&blocked_parent).unwrap();
    fs::create_dir(&blocked_parent).unwrap();
    fs::write(&store.path, b"synthetic damaged document").unwrap();
    assert_eq!(
        store.load("test.service", "key"),
        Err(SecretStoreError::Unavailable)
    );
    assert!(!store.complete_path.exists());
    fs::write(&store.path, valid_document).unwrap();
    assert_eq!(store.load("test.service", "key").unwrap(), None);
    assert!(store.complete_path.is_file());
    assert_eq!(reads(&legacy), 0);
}

#[test]
fn last_local_write_reports_marker_failure_and_keeps_the_durable_key_for_retry() {
    let directory = tempfile::tempdir().unwrap();
    let legacy = Legacy::default();
    let mut store = FileCredentialStore::new(
        directory.path(),
        "test.service",
        vec![slot("key")],
        Box::new(legacy.clone()),
    );
    let blocked_parent = directory.path().join("synthetic-marker-parent");
    fs::write(&blocked_parent, b"synthetic path obstruction").unwrap();
    store.complete_path = blocked_parent.join("complete");
    assert_eq!(
        store.save("test.service", "key", "synthetic-local-value"),
        Err(SecretStoreError::Unavailable)
    );
    let document: Document = serde_json::from_slice(&fs::read(&store.path).unwrap()).unwrap();
    assert_eq!(
        document.entries["test.service"]["key"].as_deref(),
        Some("synthetic-local-value")
    );
    assert!(document.pending_import.is_empty());
    assert!(!store.complete_path.exists());
    assert_eq!(
        store.load("test.service", "key"),
        Err(SecretStoreError::Unavailable)
    );
    fs::remove_file(&blocked_parent).unwrap();
    fs::create_dir(&blocked_parent).unwrap();
    assert_eq!(
        store.load("test.service", "key").unwrap().as_deref(),
        Some("synthetic-local-value")
    );
    assert!(store.complete_path.is_file());
    assert_eq!(reads(&legacy), 0);
}

#[test]
fn local_replacement_does_not_complete_other_pending_imports() {
    let directory = tempfile::tempdir().unwrap();
    let legacy = Legacy::default();
    legacy_value(&legacy, "pending", "synthetic-native-value");
    let store = FileCredentialStore::new(
        directory.path(),
        "test.service",
        vec![slot("replaced"), slot("pending")],
        Box::new(legacy.clone()),
    );
    store
        .save("test.service", "replaced", "synthetic-local-value")
        .unwrap();
    assert_eq!(store.pending_imports().unwrap(), 1);
    assert!(!store.complete_path.exists());
    assert_eq!(reads(&legacy), 0);
    store.migrate_legacy().unwrap();
    assert_eq!(reads(&legacy), 1);
    assert_eq!(store.pending_imports().unwrap(), 0);
    assert!(store.complete_path.is_file());
    assert_eq!(
        store.load("test.service", "replaced").unwrap().as_deref(),
        Some("synthetic-local-value")
    );
}

#[test]
fn local_replacement_keeps_pending_native_cleanup_until_verified_retirement() {
    let directory = tempfile::tempdir().unwrap();
    let legacy = Legacy::default();
    legacy_value(&legacy, "key", "synthetic-native-value");
    legacy.0.lock().unwrap().deny_deletion = true;
    let store = FileCredentialStore::new(
        directory.path(),
        "test.service",
        vec![slot("key")],
        Box::new(legacy.clone()),
    );
    assert_eq!(store.migrate_legacy(), Err(SecretStoreError::Unavailable));
    store
        .save("test.service", "key", "synthetic-local-value")
        .unwrap();
    assert_eq!(store.pending_imports().unwrap(), 1);
    assert!(!store.complete_path.exists());
    assert_eq!(
        store.load("test.service", "key").unwrap().as_deref(),
        Some("synthetic-local-value")
    );
    assert_eq!(reads(&legacy), 1);
    drop(store);
    legacy.0.lock().unwrap().deny_deletion = false;
    let restarted = FileCredentialStore::new(
        directory.path(),
        "test.service",
        vec![slot("key")],
        Box::new(legacy.clone()),
    );
    restarted.migrate_legacy().unwrap();
    assert_eq!(reads(&legacy), 1);
    assert_eq!(restarted.pending_imports().unwrap(), 0);
    assert!(restarted.complete_path.is_file());
    assert_eq!(
        restarted.load("test.service", "key").unwrap().as_deref(),
        Some("synthetic-local-value")
    );
}

#[cfg(all(feature = "development-debugger", target_os = "macos"))]
#[test]
fn read_only_manual_probe_never_repairs_permissions_or_recreates_storage() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let service = super::super::DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE;
    let store = FileCredentialStore::new(
        directory.path(),
        service,
        Vec::new(),
        Box::new(Legacy::default()),
    );
    store
        .save(service, "synthetic-slot", "synthetic-local-value")
        .unwrap();
    let document = fs::read(&store.path).unwrap();
    let document_modified = fs::metadata(&store.path).unwrap().modified().unwrap();
    let marker_modified = fs::metadata(&store.complete_path)
        .unwrap()
        .modified()
        .unwrap();
    let probe = FileCredentialStore::for_read_only_manual_probe(directory.path()).unwrap();
    assert_eq!(
        probe.load(service, "synthetic-slot").unwrap().as_deref(),
        Some("synthetic-local-value")
    );
    assert_eq!(fs::read(&probe.path).unwrap(), document);
    assert_eq!(
        fs::metadata(&probe.path).unwrap().modified().unwrap(),
        document_modified
    );
    assert_eq!(
        fs::metadata(&probe.complete_path)
            .unwrap()
            .modified()
            .unwrap(),
        marker_modified
    );

    let credentials_directory = probe.path.parent().unwrap();
    fs::set_permissions(credentials_directory, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        probe.load(service, "synthetic-slot"),
        Err(SecretStoreError::Unavailable)
    );
    assert_eq!(
        fs::metadata(credentials_directory)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    fs::set_permissions(credentials_directory, fs::Permissions::from_mode(0o700)).unwrap();
    fs::remove_file(&probe.complete_path).unwrap();
    assert_eq!(
        probe.load(service, "synthetic-slot"),
        Err(SecretStoreError::Unavailable)
    );
    assert_eq!(
        probe.save(service, "synthetic-slot", "synthetic-new-value"),
        Err(SecretStoreError::Unavailable)
    );
    assert!(!probe.complete_path.exists());
    assert_eq!(fs::read(&probe.path).unwrap(), document);
    fs::remove_file(&probe.path).unwrap();
    assert_eq!(
        probe.load(service, "synthetic-slot"),
        Err(SecretStoreError::Unavailable)
    );
    assert!(!probe.path.exists());
    assert!(!probe.complete_path.exists());
}
#[test]
fn damaged_or_future_schema_is_preserved_without_native_fallback() {
    for content in [
        "broken",
        r#"{"schemaVersion":2,"entries":{},"pendingImport":[]}"#,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let legacy = Legacy::default();
        let store = FileCredentialStore::new(
            directory.path(),
            "test.service",
            vec![slot("key")],
            Box::new(legacy.clone()),
        );
        assert_eq!(store.pending_imports().unwrap(), 1);
        fs::write(&store.path, content).unwrap();
        assert_eq!(
            store.save("test.service", "key", "public-placeholder"),
            Err(SecretStoreError::Unavailable)
        );
        assert_eq!(store.migrate_legacy(), Err(SecretStoreError::Unavailable));
        assert_eq!(fs::read_to_string(&store.path).unwrap(), content);
        assert_eq!(reads(&legacy), 0);
    }
}
#[test]
fn import_persists_the_verified_copy_before_deleting_the_native_item() {
    struct CheckedLegacy {
        inner: Legacy,
        path: PathBuf,
    }
    impl SecretStore for CheckedLegacy {
        fn load(&self, service: &str, account: &str) -> Result<Option<String>, SecretStoreError> {
            self.inner.load(service, account)
        }
        fn save(&self, _: &str, _: &str, _: &str) -> Result<(), SecretStoreError> {
            panic!("native write is forbidden")
        }
        fn delete(&self, service: &str, account: &str) -> Result<(), SecretStoreError> {
            let persisted: Document =
                serde_json::from_slice(&fs::read(&self.path).unwrap()).unwrap();
            assert_eq!(
                persisted.entries[service][account],
                self.inner.load(service, account).unwrap()
            );
            assert!(persisted.pending_import.is_empty());
            assert!(persisted
                .pending_cleanup
                .iter()
                .any(|slot| { slot.service == service && slot.account == account }));
            self.inner.delete(service, account)
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let legacy = Legacy::default();
    legacy_value(&legacy, "key", "public-placeholder");
    let store = FileCredentialStore::new(
        directory.path(),
        "test.service",
        vec![slot("key")],
        Box::new(CheckedLegacy {
            inner: legacy.clone(),
            path: directory.path().join("credentials/credentials.json"),
        }),
    );
    store.migrate_legacy().unwrap();
    assert_eq!(legacy.0.lock().unwrap().deletes, 1);
    assert!(legacy.0.lock().unwrap().values.is_empty());
    assert_eq!(
        store.load("test.service", "key").unwrap().as_deref(),
        Some("public-placeholder")
    );
    assert_eq!(store.pending_imports().unwrap(), 0);
}

#[test]
fn failed_durable_write_does_not_mark_import_complete() {
    let directory = tempfile::tempdir().unwrap();
    let legacy = Legacy::default();
    legacy_value(&legacy, "key", &"x".repeat(MAX_BYTES as usize));
    let store = FileCredentialStore::new(
        directory.path(),
        "test.service",
        vec![slot("key")],
        Box::new(legacy.clone()),
    );
    assert_eq!(store.migrate_legacy(), Err(SecretStoreError::Unavailable));
    assert_eq!(store.pending_imports().unwrap(), 1);
    assert_eq!(legacy.0.lock().unwrap().deletes, 0);
    assert_eq!(
        store.load("test.service", "key"),
        Err(SecretStoreError::MigrationRequired)
    );
}
#[test]
fn legacy_single_slot_respects_tombstone_and_skips_old_entries_when_profile_exists() {
    for (profile, tombstone, expected, read_count) in [
        (Some("profile-public"), None, Some("profile-public"), 1),
        (None, Some(LEGACY_MIGRATION_TOMBSTONE_VALUE), None, 2),
        (None, None, Some("legacy-public"), 3),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let legacy = Legacy::default();
        if let Some(value) = profile {
            legacy_value(&legacy, "key", value);
        }
        if let Some(value) = tombstone {
            legacy_value(&legacy, LEGACY_MIGRATION_TOMBSTONE_ACCOUNT, value);
        }
        legacy.0.lock().unwrap().values.insert(
            (
                LEGACY_KEYCHAIN_SERVICE_V3.into(),
                LEGACY_KEYCHAIN_ACCOUNT.into(),
            ),
            "legacy-public".into(),
        );
        let store = FileCredentialStore::new(
            directory.path(),
            "test.service",
            vec![Slot {
                legacy_alibaba: true,
                ..slot("key")
            }],
            Box::new(legacy.clone()),
        );
        store.migrate_legacy().unwrap();
        assert_eq!(
            store.load("test.service", "key").unwrap().as_deref(),
            expected
        );
        assert_eq!(reads(&legacy), read_count);
    }
}
#[test]
fn blank_legacy_slot_does_not_hide_a_later_valid_credential() {
    let directory = tempfile::tempdir().unwrap();
    let legacy = Legacy::default();
    for (service, value) in [
        (LEGACY_KEYCHAIN_SERVICE_V3, "   "),
        (LEGACY_KEYCHAIN_SERVICE_V2, "synthetic-legacy-key"),
    ] {
        legacy.0.lock().unwrap().values.insert(
            (service.into(), LEGACY_KEYCHAIN_ACCOUNT.into()),
            value.into(),
        );
    }
    let store = FileCredentialStore::new(
        directory.path(),
        "test.service",
        vec![Slot {
            legacy_alibaba: true,
            ..slot("key")
        }],
        Box::new(legacy.clone()),
    );

    store.migrate_legacy().unwrap();

    assert_eq!(
        store.load("test.service", "key").unwrap().as_deref(),
        Some("synthetic-legacy-key")
    );
    assert_eq!(reads(&legacy), 4);
    assert_eq!(store.pending_imports().unwrap(), 0);
}
#[test]
fn missing_local_profile_credential_never_reads_native_migration_items() {
    let directory = tempfile::tempdir().unwrap();
    let legacy = Legacy::default();
    let profile = ProfileCatalog::legacy_alibaba().profiles.remove(0);
    for (service, account) in [
        (
            super::super::PROFILE_KEYCHAIN_SERVICE,
            credential_account(&profile),
        ),
        (
            super::super::PROFILE_KEYCHAIN_SERVICE,
            LEGACY_MIGRATION_TOMBSTONE_ACCOUNT.into(),
        ),
        (LEGACY_KEYCHAIN_SERVICE_V3, LEGACY_KEYCHAIN_ACCOUNT.into()),
        (LEGACY_KEYCHAIN_SERVICE_V2, LEGACY_KEYCHAIN_ACCOUNT.into()),
        (LEGACY_KEYCHAIN_SERVICE, LEGACY_KEYCHAIN_ACCOUNT.into()),
    ] {
        legacy
            .0
            .lock()
            .unwrap()
            .values
            .insert((service.into(), account), "synthetic-native-value".into());
    }
    let file = FileCredentialStore::new(
        directory.path(),
        super::super::PROFILE_KEYCHAIN_SERVICE,
        vec![],
        Box::new(legacy.clone()),
    );
    let store = SettingsStore::at_path(directory.path().into(), Box::new(file));

    assert_eq!(store.load_api_key().unwrap(), None);
    assert_eq!(
        store.credential_state_for_snapshot(&profile),
        super::super::CredentialState::Missing
    );
    assert_eq!(reads(&legacy), 0);
    assert_eq!(legacy.0.lock().unwrap().deletes, 0);
}
#[test]
fn real_settings_snapshot_uses_file_metadata_and_import_clears_stale_errors() {
    let directory = tempfile::tempdir().unwrap();
    let legacy = Legacy::default();
    let profile = ProfileCatalog::legacy_alibaba().profiles.remove(0);
    let account = credential_account(&profile);
    legacy.0.lock().unwrap().values.insert(
        (
            super::super::PROFILE_KEYCHAIN_SERVICE.into(),
            account.clone(),
        ),
        "public-api-placeholder".into(),
    );
    let file = FileCredentialStore::new(
        directory.path(),
        super::super::PROFILE_KEYCHAIN_SERVICE,
        vec![Slot {
            service: super::super::PROFILE_KEYCHAIN_SERVICE.into(),
            account,
            legacy_alibaba: false,
        }],
        Box::new(legacy.clone()),
    );
    let store = SettingsStore::at_path(directory.path().into(), Box::new(file));
    assert_eq!(store.credential_storage(), "localFile");
    assert_eq!(
        store.credential_state_for_snapshot(&profile),
        super::super::CredentialState::Unavailable
    );
    assert!(store.credentials_for_profile(&profile).is_err());
    assert_eq!(reads(&legacy), 0);
    store.migrate_credentials().unwrap();
    assert_eq!(
        store.credential_state_for_snapshot(&profile),
        super::super::CredentialState::Present
    );
    assert!(store.credentials_for_profile(&profile).unwrap().is_some());
    assert_eq!(reads(&legacy), 1);
}
#[test]
fn cleanup_failure_keeps_readable_local_key_and_retries_only_cleanup() {
    let directory = tempfile::tempdir().unwrap();
    let legacy = Legacy::default();
    legacy_value(&legacy, "key", "public-placeholder");
    legacy.0.lock().unwrap().deny_deletion = true;
    let store = FileCredentialStore::new(
        directory.path(),
        "test.service",
        vec![slot("key")],
        Box::new(legacy.clone()),
    );
    assert_eq!(store.migrate_legacy(), Err(SecretStoreError::Unavailable));
    assert_eq!(
        store.load("test.service", "key").unwrap().as_deref(),
        Some("public-placeholder")
    );
    assert_eq!(store.pending_imports().unwrap(), 1);
    drop(store);
    legacy.0.lock().unwrap().deny_deletion = false;
    let store = FileCredentialStore::new(
        directory.path(),
        "test.service",
        vec![slot("key")],
        Box::new(legacy.clone()),
    );
    store.migrate_legacy().unwrap();
    assert_eq!(store.pending_imports().unwrap(), 0);
    assert_eq!(reads(&legacy), 1);
    assert!(legacy.0.lock().unwrap().values.is_empty());
    store.migrate_legacy().unwrap();
    assert_eq!(legacy.0.lock().unwrap().deletes, 2);
}

#[test]
fn damaged_catalog_does_not_invent_slots_or_finish_import() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join(PROFILE_CATALOG_FILE), "broken").unwrap();
    fs::write(directory.path().join("preferences.json"), "{}").unwrap();
    let store = FileCredentialStore::for_app(
        directory.path(),
        super::super::PROFILE_KEYCHAIN_SERVICE,
        true,
    );
    assert_eq!(store.pending_imports(), Err(SecretStoreError::Unavailable));
    assert_eq!(store.migrate_legacy(), Err(SecretStoreError::Unavailable));
    assert!(!store.path.exists());
    assert!(!store.complete_path.exists());
    fs::write(
        directory.path().join(PROFILE_CATALOG_FILE),
        serde_json::to_vec(&ProfileCatalog::legacy_alibaba()).unwrap(),
    )
    .unwrap();
    let store = FileCredentialStore::for_app(
        directory.path(),
        super::super::PROFILE_KEYCHAIN_SERVICE,
        true,
    );
    assert!(store.pending_imports().unwrap() > 0);
    assert!(!store.complete_path.exists());
}

#[test]
fn unowned_import_or_cleanup_and_missing_copy_never_touch_native_storage() {
    for kind in ["import", "cleanup", "missing-copy"] {
        let directory = tempfile::tempdir().unwrap();
        let legacy = Legacy::default();
        let store = FileCredentialStore::new(
            directory.path(),
            "test.service",
            vec![slot("key")],
            Box::new(legacy.clone()),
        );
        let mut document = store.read().unwrap();
        if kind == "import" {
            document.pending_import[0].service = "unrelated.application".into();
        } else {
            document.pending_import.clear();
            if kind != "missing-copy" {
                document
                    .entries
                    .entry("test.service".into())
                    .or_default()
                    .insert("key".into(), Some("public-placeholder".into()));
            }
            document.pending_cleanup.push(Cleanup {
                service: if kind == "cleanup" {
                    "unrelated.application"
                } else {
                    "test.service"
                }
                .into(),
                account: "key".into(),
                destination_service: "test.service".into(),
                destination_account: "key".into(),
            });
        }
        store.write(&document).unwrap();
        assert_eq!(store.migrate_legacy(), Err(SecretStoreError::Unavailable));
        assert_eq!(reads(&legacy), 0);
        assert_eq!(legacy.0.lock().unwrap().deletes, 0);
    }
}
#[test]
fn missing_file_after_completed_upgrade_never_reimports_native_keys() {
    let directory = tempfile::tempdir().unwrap();
    let legacy = Legacy::default();
    legacy_value(&legacy, "key", "public-placeholder");
    let store = FileCredentialStore::new(
        directory.path(),
        "test.service",
        vec![slot("key")],
        Box::new(legacy.clone()),
    );
    store.migrate_legacy().unwrap();
    fs::remove_file(&store.path).unwrap();
    // The live process may observe the missing file before it exits.
    assert!(!store.contains("test.service", "key").unwrap());
    assert_eq!(store.pending_imports().unwrap(), 0);
    drop(store);
    let store = FileCredentialStore::new(
        directory.path(),
        "test.service",
        vec![slot("key")],
        Box::new(legacy.clone()),
    );
    store.migrate_legacy().unwrap();
    assert!(!store.contains("test.service", "key").unwrap());
    assert_eq!(reads(&legacy), 1);
    assert_eq!(legacy.0.lock().unwrap().deletes, 1);
}

#[test]
fn completed_marker_overrides_restored_import_and_cleanup_bookkeeping() {
    let directory = tempfile::tempdir().unwrap();
    let legacy = Legacy::default();
    legacy_value(&legacy, "key", "public-placeholder");
    let store = FileCredentialStore::new(
        directory.path(),
        "test.service",
        vec![slot("key")],
        Box::new(legacy.clone()),
    );
    store.read().unwrap();
    let before_import = fs::read(&store.path).unwrap();
    legacy.0.lock().unwrap().deny_deletion = true;
    assert_eq!(store.migrate_legacy(), Err(SecretStoreError::Unavailable));
    let before_cleanup = fs::read(&store.path).unwrap();
    legacy.0.lock().unwrap().deny_deletion = false;
    store.migrate_legacy().unwrap();
    for stale in [before_import, before_cleanup] {
        fs::write(&store.path, stale).unwrap();
        assert_eq!(store.pending_imports().unwrap(), 0);
        store.migrate_legacy().unwrap();
        let restarted = FileCredentialStore::new(
            directory.path(),
            "test.service",
            vec![slot("key")],
            Box::new(legacy.clone()),
        );
        assert_eq!(restarted.pending_imports().unwrap(), 0);
        restarted.migrate_legacy().unwrap();
        assert_eq!(reads(&legacy), 1);
        assert_eq!(legacy.0.lock().unwrap().deletes, 2);
    }
}

#[cfg(unix)]
#[test]
fn private_permissions_and_symlink_rejection() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let directory = tempfile::tempdir().unwrap();
    let legacy = Legacy::default();
    let store = FileCredentialStore::new(
        directory.path(),
        "test.service",
        vec![],
        Box::new(legacy.clone()),
    );
    store.save("test", "key", "public-placeholder").unwrap();
    assert_eq!(
        fs::metadata(&store.path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(store.path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    fs::set_permissions(&store.path, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        store.load("test", "key"),
        Err(SecretStoreError::Unavailable)
    );
    fs::remove_file(&store.path).unwrap();
    let target = directory.path().join("foreign.json");
    fs::write(&target, "public-foreign-value").unwrap();
    symlink(&target, &store.path).unwrap();
    assert_eq!(
        store.save("test", "key", "new-public-placeholder"),
        Err(SecretStoreError::Unavailable)
    );
    assert_eq!(fs::read_to_string(target).unwrap(), "public-foreign-value");
    assert_eq!(reads(&legacy), 0);
}

fn pending_custom_settings(
    provider: crate::core::services::provider::ProviderKind,
) -> (
    tempfile::TempDir,
    FileCredentialStore,
    super::super::ServiceProfile,
    Legacy,
) {
    let directory = tempfile::tempdir().unwrap();
    let legacy = Legacy::default();
    let service = super::super::PROFILE_KEYCHAIN_SERVICE;
    let bootstrap = SettingsStore::load_with_secret(
        directory.path().into(),
        false,
        Box::new(legacy.clone()),
        service,
    );
    let profile = bootstrap
        .create_profile(provider, "Synthetic import repair")
        .unwrap();
    drop(bootstrap);
    let speech = credential_account(&profile);
    let text = SettingsStore::destination_account(&profile);
    {
        let mut state = legacy.0.lock().unwrap();
        state.denied.insert((service.into(), speech.clone()));
        let destination = super::super::TextTranslationDestination {
            endpoint: "https://example.com/translate".into(),
            token: "synthetic retained text value".into(),
            ..Default::default()
        };
        state.values.insert(
            (service.into(), text.clone()),
            serde_json::to_string(&destination).unwrap(),
        );
    }
    let local = FileCredentialStore::new(
        directory.path(),
        service,
        vec![
            Slot {
                service: service.into(),
                account: speech,
                legacy_alibaba: false,
            },
            Slot {
                service: service.into(),
                account: text,
                legacy_alibaba: false,
            },
        ],
        Box::new(legacy.clone()),
    );
    assert_eq!(local.migrate_legacy(), Err(SecretStoreError::Unavailable));
    assert_eq!(local.pending_imports().unwrap(), 1);
    (directory, local, profile, legacy)
}

fn complete_speech_request(key: &str) -> crate::core::services::credentials::ProviderCredentials {
    crate::core::services::credentials::ProviderCredentials::CustomSpeech {
        endpoint: "wss://speech.example/realtime".into(),
        model: "synthetic-model".into(),
        api_key: key.into(),
    }
}

#[test]
fn complete_custom_speech_replacement_repairs_only_its_malformed_saved_value() {
    use crate::core::services::{credentials::ProviderCredentials, provider::ProviderKind};
    for provider in [
        ProviderKind::CustomOpenAIASR,
        ProviderKind::CustomDashScopeASR,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let legacy = Legacy::default();
        let service = super::super::PROFILE_KEYCHAIN_SERVICE;
        let bootstrap = SettingsStore::load_with_secret(
            directory.path().into(),
            false,
            Box::new(legacy.clone()),
            service,
        );
        let profile = bootstrap
            .create_profile(provider, "Synthetic repair")
            .unwrap();
        drop(bootstrap);
        let local = FileCredentialStore::new(
            directory.path(),
            service,
            Vec::new(),
            Box::new(legacy.clone()),
        );
        let path = local.path.clone();
        let speech = credential_account(&profile);
        let text = SettingsStore::destination_account(&profile);
        local
            .save(service, &speech, "malformed synthetic speech value")
            .unwrap();
        let destination = super::super::TextTranslationDestination {
            endpoint: "https://text.example/translate".into(),
            token: "synthetic retained text value".into(),
            ..Default::default()
        };
        let destination = serde_json::to_string(&destination).unwrap();
        local.save(service, &text, &destination).unwrap();
        let store = SettingsStore::load_with_secret(
            directory.path().into(),
            false,
            Box::new(local),
            service,
        );
        let before = fs::read(&path).unwrap();
        let metadata = store.profile_catalog().unwrap();
        for request in [
            ProviderCredentials::CustomSpeech {
                endpoint: String::new(),
                model: "synthetic-model".into(),
                api_key: "synthetic-replacement-key".into(),
            },
            ProviderCredentials::CustomSpeech {
                endpoint: "wss://speech.example/realtime".into(),
                model: String::new(),
                api_key: "synthetic-replacement-key".into(),
            },
            complete_speech_request(""),
        ] {
            assert_eq!(
                    store.save_credentials(&profile.id, &request),
                    Err(
                        crate::core::services::credentials::ProviderCredentialsError::InvalidStoredValue
                            .to_string()
                    )
                );
            assert_eq!(fs::read(&path).unwrap(), before);
        }
        let request = complete_speech_request("synthetic-replacement-key");
        assert!(store
            .configuration_for_speech_draft_probe(&profile, &request, true)
            .is_ok());
        store.save_credentials(&profile.id, &request).unwrap();
        assert_eq!(store.profile_catalog().unwrap(), metadata);
        assert_eq!(
            store.destination_value(&profile).unwrap(),
            Some(destination.clone())
        );
        drop(store);
        let restarted = FileCredentialStore::new(
            directory.path(),
            service,
            Vec::new(),
            Box::new(legacy.clone()),
        );
        let store = SettingsStore::load_with_secret(
            directory.path().into(),
            false,
            Box::new(restarted),
            service,
        );
        assert_eq!(
            store.credentials_for_profile(&profile).unwrap(),
            Some(request)
        );
        assert_eq!(
            store.destination_value(&profile).unwrap(),
            Some(destination)
        );
        assert_eq!(reads(&legacy), 0);
        assert_eq!(legacy.0.lock().unwrap().deletes, 0);
    }
}

#[test]
fn complete_custom_speech_replacement_retires_only_its_pending_import() {
    use crate::core::services::{credentials::ProviderCredentials, provider::ProviderKind};
    for provider in [
        ProviderKind::CustomOpenAIASR,
        ProviderKind::CustomDashScopeASR,
    ] {
        let (directory, local, profile, legacy) = pending_custom_settings(provider);
        let path = local.path.clone();
        let original = fs::read(&path).unwrap();
        let native_reads = reads(&legacy);
        let mut store = SettingsStore::load_with_secret(
            directory.path().into(),
            false,
            Box::new(local),
            super::super::PROFILE_KEYCHAIN_SERVICE,
        );
        let metadata = store.profile_catalog().unwrap();
        for request in [
            ProviderCredentials::CustomSpeech {
                endpoint: String::new(),
                model: "synthetic-model".into(),
                api_key: "synthetic-new-key".into(),
            },
            ProviderCredentials::CustomSpeech {
                endpoint: "wss://speech.example/realtime".into(),
                model: String::new(),
                api_key: "synthetic-new-key".into(),
            },
            ProviderCredentials::CustomSpeech {
                endpoint: "wss://speech.example/realtime".into(),
                model: "synthetic-model".into(),
                api_key: String::new(),
            },
        ] {
            assert!(store.save_credentials(&profile.id, &request).is_err());
            assert_eq!(fs::read(&path).unwrap(), original);
        }
        let blocked = directory.path().join("blocked-metadata-output");
        fs::create_dir(&blocked).unwrap();
        store.catalog_path = blocked.clone();
        store.prefs_path = blocked;
        store
            .save_credentials(&profile.id, &complete_speech_request("synthetic-new-key"))
            .unwrap();
        assert_eq!(store.profile_catalog().unwrap(), metadata);
        let text = store.destination_value(&profile).unwrap().unwrap();
        let text: super::super::TextTranslationDestination = serde_json::from_str(&text).unwrap();
        assert_eq!(text.token, "synthetic retained text value");
        assert!(directory
            .path()
            .join("credentials-file-mode-complete")
            .is_file());
        assert_eq!(reads(&legacy), native_reads);
        drop(store);
        let restarted = FileCredentialStore::new(
            directory.path(),
            super::super::PROFILE_KEYCHAIN_SERVICE,
            Vec::new(),
            Box::new(legacy.clone()),
        );
        let store = SettingsStore::load_with_secret(
            directory.path().into(),
            false,
            Box::new(restarted),
            super::super::PROFILE_KEYCHAIN_SERVICE,
        );
        assert!(
            matches!(store.credentials_for_profile(&profile).unwrap(), Some(ProviderCredentials::CustomSpeech { api_key, .. }) if api_key == "synthetic-new-key")
        );
        assert_eq!(reads(&legacy), native_reads);
    }
}

#[test]
fn pending_custom_speech_marker_failure_keeps_the_durable_replacement_for_retry() {
    let (directory, mut local, profile, legacy) =
        pending_custom_settings(crate::core::services::provider::ProviderKind::CustomOpenAIASR);
    let path = local.path.clone();
    let blocker = directory.path().join("blocked-marker-parent");
    fs::write(&blocker, b"synthetic obstruction").unwrap();
    local.complete_path = blocker.join("complete");
    let native_reads = reads(&legacy);
    let store = SettingsStore::load_with_secret(
        directory.path().into(),
        false,
        Box::new(local),
        super::super::PROFILE_KEYCHAIN_SERVICE,
    );
    assert!(store
        .save_credentials(&profile.id, &complete_speech_request("synthetic-new-key"))
        .is_err());
    let document: Document = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert!(document.entries[super::super::PROFILE_KEYCHAIN_SERVICE]
        .get(&credential_account(&profile))
        .is_some_and(Option::is_some));
    assert!(document.pending_import.is_empty());
    assert_eq!(reads(&legacy), native_reads);
    fs::remove_file(&blocker).unwrap();
    fs::create_dir(&blocker).unwrap();
    store
        .save_credentials(&profile.id, &complete_speech_request("synthetic-new-key"))
        .unwrap();
    assert!(blocker.join("complete").is_file());
    assert_eq!(reads(&legacy), native_reads);
}

#[test]
fn pending_custom_speech_failed_local_write_preserves_the_pending_document() {
    let (directory, local, profile, legacy) =
        pending_custom_settings(crate::core::services::provider::ProviderKind::CustomOpenAIASR);
    let path = local.path.clone();
    let mut document: Document = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    document
        .entries
        .entry("synthetic unrelated service".into())
        .or_default()
        .insert(
            "padding".into(),
            Some("x".repeat(MAX_BYTES as usize - 3000)),
        );
    let original = serde_json::to_vec(&document).unwrap();
    assert!((original.len() as u64) < MAX_BYTES);
    fs::write(&path, &original).unwrap();
    let native_reads = reads(&legacy);
    let store = SettingsStore::load_with_secret(
        directory.path().into(),
        false,
        Box::new(local),
        super::super::PROFILE_KEYCHAIN_SERVICE,
    );
    assert!(store
        .save_credentials(&profile.id, &complete_speech_request(&"k".repeat(8192)))
        .is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(!directory
        .path()
        .join("credentials-file-mode-complete")
        .exists());
    document.entries.remove("synthetic unrelated service");
    fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    store
        .save_credentials(&profile.id, &complete_speech_request("synthetic-new-key"))
        .unwrap();
    assert!(directory
        .path()
        .join("credentials-file-mode-complete")
        .is_file());
    assert_eq!(reads(&legacy), native_reads);
}

#[test]
fn complete_custom_speech_replacement_cannot_overwrite_a_damaged_document() {
    let (directory, local, profile, legacy) =
        pending_custom_settings(crate::core::services::provider::ProviderKind::CustomOpenAIASR);
    let path = local.path.clone();
    let native_reads = reads(&legacy);
    let store = SettingsStore::load_with_secret(
        directory.path().into(),
        false,
        Box::new(local),
        super::super::PROFILE_KEYCHAIN_SERVICE,
    );
    fs::write(&path, b"synthetic damaged document").unwrap();
    assert!(store
        .save_credentials(&profile.id, &complete_speech_request("synthetic-new-key"))
        .is_err());
    assert_eq!(fs::read(&path).unwrap(), b"synthetic damaged document");
    assert_eq!(reads(&legacy), native_reads);
    assert!(!directory
        .path()
        .join("credentials-file-mode-complete")
        .exists());
}

#[test]
fn complete_text_route_cannot_replace_an_unread_multi_route_import() {
    use crate::core::services::{
        credentials::ProviderCredentials, provider::ProviderKind, provider::TextTranslation,
    };
    let directory = tempfile::tempdir().unwrap();
    let legacy = Legacy::default();
    let service = super::super::PROFILE_KEYCHAIN_SERVICE;
    let bootstrap = SettingsStore::load_with_secret(
        directory.path().into(),
        false,
        Box::new(legacy.clone()),
        service,
    );
    let profile = bootstrap
        .create_profile(ProviderKind::CustomOpenAIASR, "Synthetic text import")
        .unwrap();
    drop(bootstrap);
    let speech = credential_account(&profile);
    let text = SettingsStore::destination_account(&profile);
    let speech_value = complete_speech_request("synthetic-old-speech")
        .encode_for_keychain(profile.provider)
        .unwrap();
    let destination = super::super::TextTranslationDestination {
        endpoint: "https://old.example/translate".into(),
        token: "synthetic-old-deeplx".into(),
        open_ai_compatible: Some(super::super::OpenAICompatibleDestination {
            endpoint: "https://old.example/v1".into(),
            api_key: "synthetic-old-openai".into(),
            model: "synthetic-old-model".into(),
        }),
        chat_mock: Some(super::super::OpenAICompatibleDestination {
            endpoint: "http://localhost:8000/v1".into(),
            api_key: "synthetic-old-chatmock".into(),
            model: "synthetic-other-model".into(),
        }),
        ..Default::default()
    };
    let destination_value = serde_json::to_string(&destination).unwrap();
    {
        let mut state = legacy.0.lock().unwrap();
        state
            .values
            .insert((service.into(), speech.clone()), speech_value.clone());
        state
            .values
            .insert((service.into(), text.clone()), destination_value.clone());
        state.denied.insert((service.into(), text.clone()));
    }
    let local = FileCredentialStore::new(
        directory.path(),
        service,
        vec![
            Slot {
                service: service.into(),
                account: speech.clone(),
                legacy_alibaba: false,
            },
            Slot {
                service: service.into(),
                account: text.clone(),
                legacy_alibaba: false,
            },
        ],
        Box::new(legacy.clone()),
    );
    assert_eq!(local.migrate_legacy(), Err(SecretStoreError::Unavailable));
    assert_eq!(local.pending_imports().unwrap(), 1);
    let path = local.path.clone();
    let original = fs::read(&path).unwrap();
    let native_reads = reads(&legacy);
    let store =
        SettingsStore::load_with_secret(directory.path().into(), false, Box::new(local), service);
    let metadata = store.profile_catalog().unwrap();
    let request = ProviderCredentials::AlibabaTranslation {
        api_key: String::new(),
        text_translation: TextTranslation::OpenAICompatible,
        endpoint: "https://replacement.example/v1".into(),
        token: "synthetic-new-openai".into(),
        model: "synthetic-new-model".into(),
        clear_token: false,
    };
    assert_eq!(
        store.save_credentials(&profile.id, &request),
        Err(SecretStoreError::MigrationRequired.public_error())
    );
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(store.profile_catalog().unwrap(), metadata);
    assert_eq!(
        store.load_api_key_for_profile(&profile).unwrap(),
        Some(speech_value)
    );
    assert_eq!(reads(&legacy), native_reads);
    assert_eq!(
        legacy.0.lock().unwrap().values.get(&(service.into(), text)),
        Some(&destination_value)
    );
    assert!(!directory
        .path()
        .join("credentials-file-mode-complete")
        .exists());
}
