//! Credential storage contract and content-free availability/error labels.

use super::CREDENTIAL_STORE_UNAVAILABLE;
#[cfg(any(target_os = "linux", test))]
use super::{CREDENTIAL_SERVICE_UNAVAILABLE, CREDENTIAL_STORE_ACCESS_DENIED};
use serde::Serialize;

/// Public, deliberately coarse credential state. OS/keyring error details are
/// never serialized to the frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CredentialState {
    Present,
    Missing,
    Unavailable,
}

impl CredentialState {
    pub fn combined(self, other: Self) -> Self {
        if self == Self::Unavailable || other == Self::Unavailable {
            Self::Unavailable
        } else if self == Self::Missing || other == Self::Missing {
            Self::Missing
        } else {
            Self::Present
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretStoreError {
    Unavailable,
    MigrationRequired,
    #[cfg(any(target_os = "linux", test))]
    ServiceUnavailable,
    #[cfg(any(target_os = "linux", test))]
    AccessDenied,
}

impl SecretStoreError {
    pub(super) fn public_error(self) -> String {
        match self {
            Self::Unavailable => CREDENTIAL_STORE_UNAVAILABLE,
            Self::MigrationRequired => CREDENTIAL_STORE_UNAVAILABLE,
            #[cfg(any(target_os = "linux", test))]
            Self::ServiceUnavailable => CREDENTIAL_SERVICE_UNAVAILABLE,
            #[cfg(any(target_os = "linux", test))]
            Self::AccessDenied => CREDENTIAL_STORE_ACCESS_DENIED,
        }
        .to_string()
    }
}
/// Keyed credentials. Production uses a private local file; the native adapter
/// exists only for one-time legacy import and focused compatibility tests.
pub trait SecretStore: Send + Sync {
    #[cfg(all(test, feature = "development-debugger", target_os = "macos"))]
    fn is_read_only(&self) -> bool {
        false
    }

    fn load(&self, service: &str, account: &str) -> Result<Option<String>, SecretStoreError>;
    /// Availability only; production reads the local credential document.
    fn contains(&self, service: &str, account: &str) -> Result<bool, SecretStoreError> {
        self.load(service, account).map(|value| value.is_some())
    }
    fn save(&self, service: &str, account: &str, value: &str) -> Result<(), SecretStoreError>;
    /// Normal Save is file-backed. The native override is retained only for
    /// legacy compatibility tests and must not be selected by production.
    fn save_from_user_action(
        &self,
        service: &str,
        account: &str,
        value: &str,
    ) -> Result<(), SecretStoreError> {
        self.save(service, account, value)
    }
    fn delete(&self, service: &str, account: &str) -> Result<(), SecretStoreError>;
    fn uses_local_file(&self) -> bool {
        false
    }
    fn pending_imports(&self) -> Result<usize, SecretStoreError> {
        Ok(0)
    }
    fn migrate_legacy(&self) -> Result<(), SecretStoreError> {
        Ok(())
    }
}
