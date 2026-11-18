//! Legacy native-store adapter, used only for one-time import and compatibility tests.

use super::{SecretStore, SecretStoreError};
#[cfg(any(target_os = "windows", test))]
use std::collections::HashMap;

#[cfg(any(target_os = "linux", test))]
pub(super) fn secret_service_operation_error(error: keyring_core::Error) -> SecretStoreError {
    // NoStorageAccess covers locked storage, dismissed prompts, and unavailable
    // results. Do not assert that the collection is locked or expose its payload.
    match error {
        keyring_core::Error::NoStorageAccess(_) => SecretStoreError::AccessDenied,
        _ => SecretStoreError::Unavailable,
    }
}

fn keyring_operation_error(error: keyring_core::Error) -> SecretStoreError {
    #[cfg(target_os = "linux")]
    {
        secret_service_operation_error(error)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = error;
        SecretStoreError::Unavailable
    }
}
pub(super) struct KeyringSecretStore;

#[cfg(target_os = "windows")]
// `windows-native-keyring-store` maps `local` to
// `CRED_PERSIST_LOCAL_MACHINE` in Windows Credential Manager.
const WINDOWS_CREDENTIAL_PERSISTENCE: &str = "local";

fn credential_entry(service: &str, account: &str) -> Result<keyring_core::Entry, SecretStoreError> {
    #[cfg(target_os = "windows")]
    {
        // `keyring` owns one-time selection of the native platform store. Its
        // compatibility wrapper does not expose store modifiers, so initialise
        // it there and construct the actual entry through keyring-core.
        if keyring::Entry::store_status().is_err() {
            return Err(SecretStoreError::Unavailable);
        }
        let modifiers = HashMap::from([("persistence", WINDOWS_CREDENTIAL_PERSISTENCE)]);
        keyring_core::Entry::new_with_modifiers(service, account, &modifiers)
            .map_err(|_| SecretStoreError::Unavailable)
    }

    #[cfg(target_os = "linux")]
    {
        // keyring's compatibility Entry caches its first store initialization
        // in a LazyLock, including failure. A desktop Secret Service can appear
        // later, so use the same backend directly with a fresh encrypted session.
        use keyring_core::api::CredentialStoreApi;
        // An initialization failure means no encrypted service session could
        // be established; it does not prove that a provider is uninstalled.
        let store = zbus_secret_service_keyring_store::Store::new()
            .map_err(|_| SecretStoreError::ServiceUnavailable)?;
        store
            .build(service, account, None)
            .map_err(keyring_operation_error)
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        keyring::Entry::new(service, account)
            .map(|entry| entry.inner)
            .map_err(|_| SecretStoreError::Unavailable)
    }
}

#[cfg(target_os = "linux")]
fn save_linux_password(entry: &keyring_core::Entry, value: &str) -> Result<(), SecretStoreError> {
    let error = match entry.set_password(value) {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    // The backend cannot create its default collection: an absent alias is
    // reported as NoStorageAccess(NoResult). Only this failed explicit save
    // may request collection creation, never a settings read or diagnostic.
    if !matches!(
        &error,
        keyring_core::Error::NoStorageAccess(source)
            if matches!(source.downcast_ref::<secret_service::Error>(), Some(secret_service::Error::NoResult))
    ) {
        return Err(keyring_operation_error(error));
    }
    let service =
        secret_service::blocking::SecretService::connect(secret_service::EncryptionType::Dh)
            .map_err(|_| SecretStoreError::ServiceUnavailable)?;
    match service.get_default_collection() {
        // NoResult can also arise elsewhere in the backend. Do not turn a
        // failed unlock/read into a new collection or a second prompt.
        Ok(_) => return Err(keyring_operation_error(error)),
        Err(secret_service::Error::NoResult) => {
            // The provider owns the native authorization/password prompt and
            // encrypted storage policy. Cancellation remains an access error.
            service
                .create_collection("Default", "default")
                .map_err(|error| {
                    keyring_operation_error(
                        zbus_secret_service_keyring_store::errors::decode_error(error),
                    )
                })?;
        }
        Err(error) => {
            return Err(keyring_operation_error(
                zbus_secret_service_keyring_store::errors::decode_error(error),
            ));
        }
    }
    entry.set_password(value).map_err(keyring_operation_error)
}

#[cfg(any(target_os = "windows", test))]
pub(super) fn ensure_local_credential_persistence(
    password: &str,
    mut read_attributes: impl FnMut() -> Result<HashMap<String, String>, SecretStoreError>,
    rewrite: impl FnOnce(&str) -> Result<(), SecretStoreError>,
    mut retry_pause: impl FnMut(),
) -> Result<(), SecretStoreError> {
    const VERIFICATION_ATTEMPTS: usize = 3;
    let is_local = |attributes: &HashMap<String, String>| {
        attributes
            .get("persistence")
            .is_some_and(|value| value.eq_ignore_ascii_case("local"))
    };
    let mut initial_attributes = None;
    for attempt in 0..VERIFICATION_ATTEMPTS {
        if let Ok(attributes) = read_attributes() {
            initial_attributes = Some(attributes);
            break;
        }
        if attempt + 1 < VERIFICATION_ATTEMPTS {
            retry_pause();
        }
    }
    let Some(initial_attributes) = initial_attributes else {
        return Err(SecretStoreError::Unavailable);
    };
    if is_local(&initial_attributes) {
        return Ok(());
    }

    // Entries written by earlier Mimi builds used Windows' Enterprise
    // persistence. Rewriting the same target with a local-spec entry keeps the
    // secret value while stopping future credential roaming.
    rewrite(password)?;
    // Windows Credential Manager can briefly expose stale attributes after a
    // persistence rewrite. Verify a bounded number of times instead of
    // caching that transient state as unavailable until restart.
    for attempt in 0..VERIFICATION_ATTEMPTS {
        if read_attributes().is_ok_and(|attributes| is_local(&attributes)) {
            return Ok(());
        }
        if attempt + 1 < VERIFICATION_ATTEMPTS {
            retry_pause();
        }
    }
    Err(SecretStoreError::Unavailable)
}

#[cfg(target_os = "windows")]
fn enforce_local_credential_persistence(
    entry: &keyring_core::Entry,
    password: &str,
) -> Result<(), SecretStoreError> {
    ensure_local_credential_persistence(
        password,
        || {
            entry
                .get_attributes()
                .map_err(|_| SecretStoreError::Unavailable)
        },
        |value| {
            entry
                .set_password(value)
                .map_err(|_| SecretStoreError::Unavailable)
        },
        || std::thread::sleep(std::time::Duration::from_millis(25)),
    )
}

impl SecretStore for KeyringSecretStore {
    #[cfg(target_os = "macos")]
    fn contains(&self, service: &str, account: &str) -> Result<bool, SecretStoreError> {
        use security_framework::item::{ItemClass, ItemSearchOptions};
        use security_framework::os::macos::keychain::{SecKeychain, SecPreferencesDomain};
        let keychain = SecKeychain::default_for_domain(SecPreferencesDomain::User)
            .map_err(|_| SecretStoreError::Unavailable)?;
        // Same User keychain and exact account as keyring's legacy backend.
        // Return attributes only; never password bytes or altered access rules.
        match ItemSearchOptions::new()
            .keychains(&[keychain])
            .class(ItemClass::generic_password())
            .service(service)
            .account(account)
            .load_attributes(true)
            .search()
        {
            Ok(items) => Ok(!items.is_empty()),
            Err(error) if error.code() == -25300 => Ok(false),
            Err(_) => Err(SecretStoreError::Unavailable),
        }
    }

    fn load(&self, service: &str, account: &str) -> Result<Option<String>, SecretStoreError> {
        let entry = credential_entry(service, account)?;
        match entry.get_password() {
            Ok(password) => Ok(Some(password)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(error) => Err(keyring_operation_error(error)),
        }
    }

    fn save(&self, service: &str, account: &str, value: &str) -> Result<(), SecretStoreError> {
        let entry = credential_entry(service, account)?;
        entry.set_password(value).map_err(keyring_operation_error)?;
        #[cfg(target_os = "windows")]
        enforce_local_credential_persistence(&entry, value)?;
        Ok(())
    }

    fn save_from_user_action(
        &self,
        service: &str,
        account: &str,
        value: &str,
    ) -> Result<(), SecretStoreError> {
        #[cfg(target_os = "linux")]
        {
            save_linux_password(&credential_entry(service, account)?, value)
        }
        #[cfg(not(target_os = "linux"))]
        {
            self.save(service, account, value)
        }
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), SecretStoreError> {
        let entry = credential_entry(service, account)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(error) => Err(keyring_operation_error(error)),
        }
    }
}
