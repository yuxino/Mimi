//! Native credentials behavior regressions.

use super::super::native_credentials::{
    ensure_local_credential_persistence, secret_service_operation_error,
};
use super::*;

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires an isolated desktop Secret Service and synthetic native prompt interaction"]
fn linux_secret_service_first_save_and_restart() {
    use secret_service::{blocking::SecretService, EncryptionType};
    let mode = std::env::var("MIMI_TEST_SECRET_SERVICE_FIRST_SAVE").unwrap();
    let directory = std::env::var("MIMI_TEST_PRIVATE_KEYRING_DIRECTORY").unwrap();
    assert!(std::env::var("DBUS_SESSION_BUS_ADDRESS")
        .unwrap()
        .contains(&directory));
    let store = SettingsStore::in_memory_with_scope(
        Box::new(KeyringSecretStore),
        false,
        "app.yuxino.mimi.test.secret-service-first-save",
    );
    let profile = store.active_profile().unwrap();
    let synthetic_value = "synthetic-first-save-value";
    if mode == "read" {
        assert_eq!(store.credential_state(&profile), CredentialState::Present);
        assert!(store.load_api_key().unwrap().as_deref() == Some(synthetic_value));
        store.delete_api_key(&profile.id).unwrap();
        assert_eq!(store.credential_state(&profile), CredentialState::Missing);
        return;
    }
    assert_eq!(mode, "write");
    let service = SecretService::connect(EncryptionType::Dh).unwrap();
    let assert_no_default = || {
        assert!(matches!(
            service.get_default_collection(),
            Err(secret_service::Error::NoResult)
        ));
    };
    assert_no_default();
    assert_eq!(store.credential_diagnostic(&profile), "missing");
    assert_no_default();
    // The background-migration write path must also leave the collection
    // absent instead of raising a first-run prompt during a settings read.
    assert_eq!(
        KeyringSecretStore.save(
            store.profile_keychain_service,
            &credential_account(&profile),
            synthetic_value,
        ),
        Err(SecretStoreError::AccessDenied)
    );
    assert_no_default();
    // The script dismisses the first native prompt, then enters a password
    // for an encrypted synthetic keyring on the explicit second save.
    let progress = Path::new(&directory).join("first-save-phase");
    std::fs::write(&progress, "cancel").unwrap();
    assert_eq!(
        store.save_api_key(&profile.id, synthetic_value),
        Err(CREDENTIAL_STORE_ACCESS_DENIED.into())
    );
    assert_no_default();
    // A cancelled D-Bus prompt can complete before its GTK window closes.
    // Wait for the smoke driver to observe that closure before requesting
    // the next prompt; otherwise it may submit the previous dialog again.
    let retry_ready = Path::new(&directory).join("first-save-retry-ready");
    let wait_started = std::time::Instant::now();
    while !retry_ready.exists() {
        assert!(
            wait_started.elapsed() < std::time::Duration::from_secs(30),
            "smoke driver did not acknowledge the cancelled prompt closing"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    std::fs::write(&progress, "accept").unwrap();
    store.save_api_key(&profile.id, synthetic_value).unwrap();
    assert_eq!(store.credential_state(&profile), CredentialState::Present);
    assert!(service.get_default_collection().is_ok());

    // A new test process has no Mimi credential cache. This proves a
    // durable backend read across app-process lifetimes, not Stop/Start.
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "settings_store::tests::native_credentials::linux_secret_service_first_save_and_restart",
            "--exact",
            "--ignored",
        ])
        .env("MIMI_TEST_SECRET_SERVICE_FIRST_SAVE", "read")
        .status()
        .unwrap();
    assert!(result.success());
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires an isolated D-Bus session and unlocked test Secret Service"]
fn linux_secret_service_roundtrip() {
    assert_eq!(
        std::env::var("MIMI_TEST_SECRET_SERVICE").as_deref(),
        Ok("1")
    );
    let account = format!("linux-ci-{}", uuid::Uuid::new_v4());
    let service = "app.yuxino.mimi.test.secret-service";
    let store = KeyringSecretStore;
    assert_eq!(store.load(service, &account).unwrap(), None);
    store
        .save(service, &account, "non-secret-test-value")
        .unwrap();
    assert!(store.load(service, &account).unwrap().as_deref() == Some("non-secret-test-value"));
    store.save(service, &account, "updated-test-value").unwrap();
    assert!(store.load(service, &account).unwrap().as_deref() == Some("updated-test-value"));
    store.delete(service, &account).unwrap();
    assert_eq!(store.load(service, &account).unwrap(), None);
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires a private non-activating D-Bus session; starts synthetic Secret Service"]
fn linux_secret_service_recovers_without_restart() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    assert_eq!(
        std::env::var("MIMI_TEST_SECRET_SERVICE_RECOVERY").as_deref(),
        Ok("1")
    );
    let directory = std::env::var("MIMI_TEST_PRIVATE_KEYRING_DIRECTORY").unwrap();
    assert!(std::env::var("DBUS_SESSION_BUS_ADDRESS")
        .unwrap()
        .contains(&directory));
    // Reproduce the dependency's permanently cached first failure too.
    assert!(keyring::Entry::store_status().is_err());
    let store = SettingsStore::in_memory_with_scope(
        Box::new(KeyringSecretStore),
        false,
        "app.yuxino.mimi.test.secret-service-recovery",
    );
    let profile = store.active_profile().unwrap();
    assert_eq!(store.credential_diagnostic(&profile), "serviceUnavailable");
    assert_eq!(
        store.credential_state(&profile),
        CredentialState::Unavailable
    );
    assert_eq!(
        store.save_api_key(&profile.id, "synthetic-recovery-value"),
        Err(CREDENTIAL_SERVICE_UNAVAILABLE.to_string())
    );

    struct Daemon(std::process::Child);
    impl Drop for Daemon {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut daemon = Daemon(
        Command::new("gnome-keyring-daemon")
            .args([
                "--foreground",
                "--unlock",
                "--components=secrets",
                "--control-directory",
            ])
            .arg(Path::new(&directory).join("control"))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    daemon
        .0
        .stdin
        .take()
        .unwrap()
        .write_all(b"mimi-ci-synthetic-recovery")
        .unwrap();
    let mut unlocked = false;
    for _ in 0..40 {
        let result = Command::new("gdbus")
            .args([
                "call",
                "--session",
                "--dest",
                "org.freedesktop.secrets",
                "--object-path",
                "/org/freedesktop/secrets/collection/login",
                "--method",
                "org.freedesktop.DBus.Properties.Get",
                "org.freedesktop.Secret.Collection",
                "Locked",
            ])
            .output()
            .unwrap();
        if result.status.success() && String::from_utf8_lossy(&result.stdout).contains("false") {
            unlocked = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    assert!(unlocked, "synthetic Secret Service did not unlock");
    assert!(keyring::Entry::store_status().is_err());
    assert_eq!(store.credential_diagnostic(&profile), "missing");
    assert_eq!(store.credential_state(&profile), CredentialState::Missing);
    store
        .save_api_key(&profile.id, "synthetic-recovery-value")
        .unwrap();
    assert_eq!(store.credential_diagnostic(&profile), "present");
    assert!(
        KeyringSecretStore
            .load(
                store.profile_keychain_service,
                &credential_account(&profile)
            )
            .unwrap()
            .as_deref()
            == Some("synthetic-recovery-value")
    );
    store.delete_api_key(&profile.id).unwrap();
    assert_eq!(store.credential_diagnostic(&profile), "missing");
}

#[test]
fn local_windows_credential_is_not_rewritten() {
    let reads = Cell::new(0);
    let rewrites = Cell::new(0);

    ensure_local_credential_persistence(
        "secret",
        || {
            reads.set(reads.get() + 1);
            Ok(persistence_attributes("Local"))
        },
        |_| {
            rewrites.set(rewrites.get() + 1);
            Ok(())
        },
        || {},
    )
    .unwrap();

    assert_eq!(reads.get(), 1);
    assert_eq!(rewrites.get(), 0);
}

#[test]
fn enterprise_windows_credential_is_rewritten_and_verified_once() {
    let reads = Cell::new(0);
    let rewrites = Cell::new(0);

    ensure_local_credential_persistence(
        "secret",
        || {
            let read = reads.get();
            reads.set(read + 1);
            Ok(persistence_attributes(if read == 0 {
                "Enterprise"
            } else {
                "Local"
            }))
        },
        |value| {
            assert_eq!(value, "secret");
            rewrites.set(rewrites.get() + 1);
            Ok(())
        },
        || {},
    )
    .unwrap();

    assert_eq!(reads.get(), 2);
    assert_eq!(rewrites.get(), 1);
}

#[test]
fn windows_credential_migration_fails_closed_when_local_state_is_not_verified() {
    let reads = Cell::new(0);
    let rewrites = Cell::new(0);

    let result = ensure_local_credential_persistence(
        "secret",
        || {
            reads.set(reads.get() + 1);
            Ok(persistence_attributes("Enterprise"))
        },
        |_| {
            rewrites.set(rewrites.get() + 1);
            Ok(())
        },
        || {},
    );

    assert_eq!(result, Err(SecretStoreError::Unavailable));
    assert_eq!(reads.get(), 4);
    assert_eq!(rewrites.get(), 1);
}

#[test]
fn windows_credential_verification_retries_a_transient_attribute_failure() {
    let reads = Cell::new(0);
    let pauses = Cell::new(0);

    ensure_local_credential_persistence(
        "secret",
        || {
            let read = reads.get();
            reads.set(read + 1);
            match read {
                0 => Ok(persistence_attributes("Enterprise")),
                1 => Err(SecretStoreError::Unavailable),
                _ => Ok(persistence_attributes("Local")),
            }
        },
        |_| Ok(()),
        || pauses.set(pauses.get() + 1),
    )
    .unwrap();

    assert_eq!(reads.get(), 3);
    assert_eq!(pauses.get(), 1);
}

#[test]
fn local_windows_credential_retries_an_initial_attribute_failure_without_rewrite() {
    let reads = Cell::new(0);
    let rewrites = Cell::new(0);

    ensure_local_credential_persistence(
        "secret",
        || {
            let read = reads.get();
            reads.set(read + 1);
            if read == 0 {
                Err(SecretStoreError::Unavailable)
            } else {
                Ok(persistence_attributes("Local"))
            }
        },
        |_| {
            rewrites.set(rewrites.get() + 1);
            Ok(())
        },
        || {},
    )
    .unwrap();

    assert_eq!(reads.get(), 2);
    assert_eq!(rewrites.get(), 0);
}

#[cfg(target_os = "windows")]
#[test]
fn windows_credential_manager_imports_enterprise_into_file_then_retires_native_item() {
    struct Cleanup {
        service: String,
        account: String,
    }

    impl Drop for Cleanup {
        fn drop(&mut self) {
            if let Ok(entry) = keyring_core::Entry::new(&self.service, &self.account) {
                let _ = entry.delete_credential();
            }
        }
    }

    assert!(keyring::Entry::store_status().is_ok());
    let unique = uuid::Uuid::new_v4().simple().to_string();
    let service = format!("app.yuxino.mimi.test.{unique}");
    let profile = ServiceProfile::new(
        format!("credential-file-migration-{unique}"),
        "Credential migration test",
        ProviderKind::OpenAIRealtime,
    )
    .unwrap();
    let account = credential_account(&profile);
    let directory = tempfile::tempdir().unwrap();
    let catalog = ProfileCatalog {
        schema_version: PROFILE_CATALOG_SCHEMA_VERSION,
        active_profile_id: profile.id.clone(),
        profiles: vec![profile],
    };
    std::fs::write(
        directory.path().join(PROFILE_CATALOG_FILE),
        serde_json::to_vec(&catalog).unwrap(),
    )
    .unwrap();
    let _cleanup = Cleanup {
        service: service.clone(),
        account: account.clone(),
    };
    let enterprise = keyring_core::Entry::new(&service, &account).unwrap();
    enterprise.set_password("migration-secret").unwrap();
    assert_eq!(
        enterprise.get_attributes().unwrap()["persistence"],
        "Enterprise"
    );

    assert_eq!(
        KeyringSecretStore.load(&service, &account).unwrap(),
        Some("migration-secret".to_string())
    );

    // Legacy reads must not rewrite the native item or its persistence.
    let observed = keyring_core::Entry::new(&service, &account).unwrap();
    assert_eq!(observed.get_password().unwrap(), "migration-secret");
    assert_eq!(
        observed.get_attributes().unwrap()["persistence"],
        "Enterprise"
    );

    let store = file_credentials::FileCredentialStore::for_app(directory.path(), &service, false);
    assert_eq!(store.pending_imports().unwrap(), 1);
    store.migrate_legacy().unwrap();
    assert_eq!(store.pending_imports().unwrap(), 0);
    let saved: serde_json::Value = serde_json::from_slice(
        &std::fs::read(directory.path().join("credentials/credentials.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(saved["entries"][&service][&account], "migration-secret");
    assert!(matches!(
        observed.get_password(),
        Err(keyring_core::Error::NoEntry)
    ));
    drop(store);

    let restarted =
        file_credentials::FileCredentialStore::for_app(directory.path(), &service, false);
    restarted.migrate_legacy().unwrap();
    assert_eq!(
        restarted.load(&service, &account).unwrap().as_deref(),
        Some("migration-secret")
    );
    restarted
        .save(&service, &account, "updated-secret")
        .unwrap();
    assert_eq!(
        restarted.load(&service, &account).unwrap().as_deref(),
        Some("updated-secret")
    );
    assert!(matches!(
        observed.get_password(),
        Err(keyring_core::Error::NoEntry)
    ));
}

#[test]
fn linux_storage_access_errors_are_classified_without_exposing_payloads() {
    let private_detail = "synthetic-private-native-detail";
    let access = keyring_core::Error::NoStorageAccess(Box::new(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        private_detail,
    )));
    assert_eq!(
        secret_service_operation_error(access),
        SecretStoreError::AccessDenied
    );
    let failure =
        keyring_core::Error::PlatformFailure(Box::new(std::io::Error::other(private_detail)));
    for error in [
        failure,
        keyring_core::Error::BadEncoding(private_detail.as_bytes().to_vec()),
        keyring_core::Error::NoDefaultStore,
    ] {
        let safe = secret_service_operation_error(error);
        assert_eq!(safe, SecretStoreError::Unavailable);
        assert!(!safe.public_error().contains(private_detail));
    }
}
