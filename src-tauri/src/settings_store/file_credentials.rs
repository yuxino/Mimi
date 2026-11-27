//! Production credentials. Native stores are used only by one-time upgrade import.
use super::{
    credential_account, ProfileCatalog, SecretStore, SecretStoreError, SettingsStore,
    LEGACY_KEYCHAIN_ACCOUNT, LEGACY_KEYCHAIN_SERVICE, LEGACY_KEYCHAIN_SERVICE_V2,
    LEGACY_KEYCHAIN_SERVICE_V3, LEGACY_MIGRATION_TOMBSTONE_ACCOUNT,
    LEGACY_MIGRATION_TOMBSTONE_VALUE, PROFILE_CATALOG_FILE,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

const MAX_BYTES: u64 = 4 * 1024 * 1024;
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Slot {
    service: String,
    account: String,
    legacy_alibaba: bool,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Cleanup {
    service: String,
    account: String,
    destination_service: String,
    destination_account: String,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Document {
    schema_version: u32,
    entries: HashMap<String, HashMap<String, Option<String>>>,
    pending_import: Vec<Slot>,
    #[serde(default)]
    pending_cleanup: Vec<Cleanup>,
}

pub(super) struct FileCredentialStore {
    path: PathBuf,
    initial: Document,
    legacy_service: String,
    complete_path: PathBuf,
    initial_unavailable: bool,
    legacy: Box<dyn SecretStore>,
    access: Mutex<()>,
    #[cfg(all(test, feature = "development-debugger", target_os = "macos"))]
    read_only: bool,
}
impl FileCredentialStore {
    #[cfg(all(test, feature = "development-debugger", target_os = "macos"))]
    pub(super) fn for_read_only_manual_probe(directory: &Path) -> Result<Self, SecretStoreError> {
        Self::for_scoped_read_only_manual_probe(
            directory,
            super::DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE,
        )
    }

    #[cfg(all(test, feature = "development-debugger", target_os = "macos"))]
    pub(super) fn for_scoped_read_only_manual_probe(
        directory: &Path,
        service: &str,
    ) -> Result<Self, SecretStoreError> {
        let mut store = Self::for_app(directory, service, false);
        store.read_only = true;
        // Validate the existing document without creating files, repairing
        // permissions, or entering a native-store migration path.
        store.read()?;
        Ok(store)
    }

    pub(super) fn for_app(directory: &Path, service: &str, import_alibaba: bool) -> Self {
        let (profiles, initial_unavailable) = match fs::read(directory.join(PROFILE_CATALOG_FILE)) {
            Ok(bytes) => match serde_json::from_slice::<ProfileCatalog>(&bytes)
                .ok()
                .and_then(|catalog| catalog.validated().ok())
            {
                Some(catalog) => (catalog.profiles, false),
                None => (Vec::new(), true),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (
                if directory.join("preferences.json").is_file() {
                    ProfileCatalog::legacy_alibaba().profiles
                } else {
                    Vec::new()
                },
                false,
            ),
            Err(_) => (Vec::new(), true),
        };
        let mut slots = Vec::new();
        for profile in profiles {
            if super::is_legacy_env_profile_id(&profile.id)
                || profile.provider == crate::core::services::provider::ProviderKind::AppleSpeech
            {
                continue;
            }
            slots.push(Slot {
                service: service.into(),
                account: credential_account(&profile),
                legacy_alibaba: import_alibaba && super::is_default_alibaba(&profile),
            });
            if profile.provider.supports_text_translation() {
                slots.push(Slot {
                    service: service.into(),
                    account: SettingsStore::destination_account(&profile),
                    legacy_alibaba: false,
                });
            }
        }
        let mut store = Self::new(
            directory,
            service,
            slots,
            Box::new(super::KeyringSecretStore),
        );
        store.initial_unavailable = initial_unavailable;
        store
    }
    fn new(
        directory: &Path,
        service: &str,
        mut slots: Vec<Slot>,
        legacy: Box<dyn SecretStore>,
    ) -> Self {
        let complete_path = directory.join("credentials-file-mode-complete");
        // Losing a local file after a completed upgrade must not resurrect an
        // OS authorization path. This marker contains no credential material.
        if fs::symlink_metadata(&complete_path).is_ok() {
            slots.clear();
        }
        Self {
            legacy_service: service.into(),
            complete_path,
            initial_unavailable: false,
            path: directory.join("credentials/credentials.json"),
            initial: Document {
                schema_version: 1,
                entries: HashMap::new(),
                pending_import: slots,
                pending_cleanup: Vec::new(),
            },
            legacy,
            access: Mutex::new(()),
            #[cfg(all(test, feature = "development-debugger", target_os = "macos"))]
            read_only: false,
        }
    }
    fn read(&self) -> Result<Document, SecretStoreError> {
        let document = self.read_document()?;
        self.complete_verified_document(&document)?;
        Ok(document)
    }
    fn read_document(&self) -> Result<Document, SecretStoreError> {
        if !self.path.is_absolute() {
            return Err(SecretStoreError::Unavailable);
        }
        // Completion remains authoritative for this live store too, including
        // when a credential file disappears or an older copy is restored.
        let import_complete = fs::symlink_metadata(&self.complete_path).is_ok();
        let directory = self.path.parent().ok_or(SecretStoreError::Unavailable)?;
        #[cfg(all(test, feature = "development-debugger", target_os = "macos"))]
        if self.read_only {
            use std::os::unix::fs::MetadataExt;
            let marker = fs::symlink_metadata(&self.complete_path)
                .map_err(|_| SecretStoreError::Unavailable)?;
            let metadata =
                fs::symlink_metadata(directory).map_err(|_| SecretStoreError::Unavailable)?;
            if !marker.is_file()
                || marker.file_type().is_symlink()
                || !metadata.is_dir()
                || metadata.file_type().is_symlink()
                || metadata.uid() != current_uid()
                || metadata.mode() & 0o7777 != 0o700
            {
                return Err(SecretStoreError::Unavailable);
            }
        } else {
            private_directory(directory).map_err(|_| SecretStoreError::Unavailable)?;
        }
        #[cfg(not(all(test, feature = "development-debugger", target_os = "macos")))]
        private_directory(directory).map_err(|_| SecretStoreError::Unavailable)?;
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                #[cfg(all(test, feature = "development-debugger", target_os = "macos"))]
                if self.read_only {
                    return Err(SecretStoreError::Unavailable);
                }
                if self.initial_unavailable {
                    return Err(SecretStoreError::Unavailable);
                }
                let mut document = self.initial.clone();
                if import_complete {
                    document.pending_import.clear();
                }
                self.write(&document)?;
                return Ok(document);
            }
            Err(_) => return Err(SecretStoreError::Unavailable),
        };
        validate_file(&metadata)?;
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x00200000); // FILE_FLAG_OPEN_REPARSE_POINT
        }
        let file = options
            .open(&self.path)
            .map_err(|_| SecretStoreError::Unavailable)?;
        let opened = file.metadata().map_err(|_| SecretStoreError::Unavailable)?;
        validate_file(&opened)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.dev() != opened.dev() || metadata.ino() != opened.ino() {
                return Err(SecretStoreError::Unavailable);
            }
        }
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| SecretStoreError::Unavailable)?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(SecretStoreError::Unavailable);
        }
        let mut document: Document =
            serde_json::from_slice(&bytes).map_err(|_| SecretStoreError::Unavailable)?;
        if import_complete {
            document.pending_import.clear();
            document.pending_cleanup.clear();
        }
        let mut seen = HashSet::new();
        if document.schema_version != 1
            || document.pending_import.iter().any(|slot| {
                !self.valid_slot(slot)
                    || !seen.insert((&slot.service, &slot.account))
                    || document
                        .entries
                        .get(&slot.service)
                        .is_some_and(|entries| entries.contains_key(&slot.account))
            })
        {
            return Err(SecretStoreError::Unavailable);
        }
        let mut cleanup_seen = HashSet::new();
        if document.pending_cleanup.iter().any(|slot| {
            !cleanup_seen.insert((&slot.service, &slot.account))
                || !self.valid_cleanup(slot, &document)
        }) {
            return Err(SecretStoreError::Unavailable);
        }
        Ok(document)
    }
    fn complete_verified_document(&self, document: &Document) -> Result<(), SecretStoreError> {
        #[cfg(all(test, feature = "development-debugger", target_os = "macos"))]
        if self.read_only {
            return Ok(());
        }
        if !document.pending_import.is_empty() || !document.pending_cleanup.is_empty() {
            return Ok(());
        }
        if fs::symlink_metadata(&self.complete_path).is_err() {
            // Creation, replacement and import cancellation all reach this
            // boundary. Verify the durable document before closing OS import,
            // including a retry after a previous marker write failed.
            if self.read_document()? != *document {
                return Err(SecretStoreError::Unavailable);
            }
        }
        self.mark_complete()
    }
    fn valid_slot(&self, slot: &Slot) -> bool {
        if slot.service != self.legacy_service {
            return false;
        }
        #[cfg(test)]
        if slot.service == "test.service" {
            return !slot.account.is_empty();
        }
        let Some(account) = slot.account.strip_prefix("provider-profile:") else {
            return false;
        };
        let parts: Vec<_> = account.split(':').collect();
        parts.len() == 3
            && !parts[0].is_empty()
            && !parts[1].is_empty()
            && matches!(parts[2], "api-key" | "text-translation")
            && (!slot.legacy_alibaba || self.alibaba_destination(&slot.account))
    }
    fn alibaba_destination(&self, account: &str) -> bool {
        #[cfg(test)]
        if self.legacy_service == "test.service" && account == "key" {
            return true;
        }
        self.legacy_service == super::PROFILE_KEYCHAIN_SERVICE
            && account == credential_account(&super::ServiceProfile::alibaba_default())
    }
    fn valid_cleanup(&self, slot: &Cleanup, document: &Document) -> bool {
        let destination = Slot {
            service: slot.destination_service.clone(),
            account: slot.destination_account.clone(),
            legacy_alibaba: false,
        };
        if !self.valid_slot(&destination)
            || !document
                .entries
                .get(&destination.service)
                .is_some_and(|entries| entries.contains_key(&destination.account))
        {
            return false;
        }
        (slot.service == destination.service && slot.account == destination.account)
            || (self.alibaba_destination(&destination.account)
                && ((slot.service == self.legacy_service
                    && slot.account == LEGACY_MIGRATION_TOMBSTONE_ACCOUNT)
                    || (matches!(
                        slot.service.as_str(),
                        LEGACY_KEYCHAIN_SERVICE
                            | LEGACY_KEYCHAIN_SERVICE_V2
                            | LEGACY_KEYCHAIN_SERVICE_V3
                    ) && slot.account == LEGACY_KEYCHAIN_ACCOUNT)))
    }
    fn mark_complete(&self) -> Result<(), SecretStoreError> {
        #[cfg(all(test, feature = "development-debugger", target_os = "macos"))]
        if self.read_only {
            return Err(SecretStoreError::Unavailable);
        }
        if fs::symlink_metadata(&self.complete_path).is_ok() {
            return Ok(());
        }
        let parent = self.path.parent().ok_or(SecretStoreError::Unavailable)?;
        let mut file = tempfile::Builder::new()
            .prefix(".import-complete-")
            .tempfile_in(parent)
            .map_err(|_| SecretStoreError::Unavailable)?;
        private_file(file.path()).map_err(|_| SecretStoreError::Unavailable)?;
        file.write_all(b"1\n")
            .and_then(|_| file.as_file().sync_all())
            .map_err(|_| SecretStoreError::Unavailable)?;
        super::persistence::replace_file(file.path(), &self.complete_path)
            .map_err(|_| SecretStoreError::Unavailable)?;
        if let Some(parent) = self.complete_path.parent() {
            super::persistence::sync_directory(parent);
        }
        Ok(())
    }
    fn write(&self, document: &Document) -> Result<(), SecretStoreError> {
        #[cfg(all(test, feature = "development-debugger", target_os = "macos"))]
        if self.read_only {
            return Err(SecretStoreError::Unavailable);
        }
        let parent = self.path.parent().ok_or(SecretStoreError::Unavailable)?;
        let bytes = serde_json::to_vec(document).map_err(|_| SecretStoreError::Unavailable)?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(SecretStoreError::Unavailable);
        }
        let mut file = tempfile::Builder::new()
            .prefix(".credentials-")
            .tempfile_in(parent)
            .map_err(|_| SecretStoreError::Unavailable)?;
        private_file(file.path()).map_err(|_| SecretStoreError::Unavailable)?;
        file.write_all(&bytes)
            .and_then(|_| file.as_file().sync_all())
            .map_err(|_| SecretStoreError::Unavailable)?;
        super::persistence::replace_file(file.path(), &self.path)
            .map_err(|_| SecretStoreError::Unavailable)?;
        super::persistence::sync_directory(parent);
        Ok(())
    }
    fn store(
        &self,
        service: &str,
        account: &str,
        value: Option<String>,
    ) -> Result<(), SecretStoreError> {
        #[cfg(all(test, feature = "development-debugger", target_os = "macos"))]
        if self.read_only {
            return Err(SecretStoreError::Unavailable);
        }
        let _access = self
            .access
            .lock()
            .map_err(|_| SecretStoreError::Unavailable)?;
        let mut document = self.read()?;
        document
            .entries
            .entry(service.into())
            .or_default()
            .insert(account.into(), value);
        document
            .pending_import
            .retain(|slot| slot.service != service || slot.account != account);
        self.write(&document)?;
        let verified = self.read_document()?;
        if verified != document {
            return Err(SecretStoreError::Unavailable);
        }
        self.complete_verified_document(&verified)
    }
    fn imported_value(&self, slot: &Slot) -> Result<Option<String>, SecretStoreError> {
        let value = self.legacy.load(&slot.service, &slot.account)?;
        if let Some(value) = value {
            return if value.trim().is_empty() {
                Err(SecretStoreError::Unavailable)
            } else {
                Ok(Some(value))
            };
        }
        if !slot.legacy_alibaba {
            return Ok(None);
        }
        match self
            .legacy
            .load(&slot.service, LEGACY_MIGRATION_TOMBSTONE_ACCOUNT)?
        {
            Some(value) if value == LEGACY_MIGRATION_TOMBSTONE_VALUE => return Ok(None),
            Some(_) => return Err(SecretStoreError::Unavailable),
            None => {}
        }
        for service in [
            LEGACY_KEYCHAIN_SERVICE_V3,
            LEGACY_KEYCHAIN_SERVICE_V2,
            LEGACY_KEYCHAIN_SERVICE,
        ] {
            if let Some(value) = self.legacy.load(service, LEGACY_KEYCHAIN_ACCOUNT)? {
                if !value.trim().is_empty() {
                    return Ok(Some(value));
                }
            }
        }
        Ok(None)
    }
}
impl SecretStore for FileCredentialStore {
    #[cfg(all(test, feature = "development-debugger", target_os = "macos"))]
    fn is_read_only(&self) -> bool {
        self.read_only
    }

    fn uses_local_file(&self) -> bool {
        true
    }
    fn pending_imports(&self) -> Result<usize, SecretStoreError> {
        let _access = self
            .access
            .lock()
            .map_err(|_| SecretStoreError::Unavailable)?;
        self.read()
            .map(|document| document.pending_import.len() + document.pending_cleanup.len())
    }
    fn migrate_legacy(&self) -> Result<(), SecretStoreError> {
        #[cfg(all(test, feature = "development-debugger", target_os = "macos"))]
        if self.read_only {
            return Err(SecretStoreError::Unavailable);
        }
        // Hold only this synchronous storage guard; no async lock or preferences
        // guard survives native authorization. Successful slots are checkpointed.
        let _access = self
            .access
            .lock()
            .map_err(|_| SecretStoreError::Unavailable)?;
        let mut document = self.read()?;
        let mut failed = None;
        for slot in document.pending_import.clone() {
            match self.imported_value(&slot) {
                Ok(value) => {
                    document
                        .entries
                        .entry(slot.service.clone())
                        .or_default()
                        .insert(slot.account.clone(), value);
                    document.pending_import.retain(|entry| {
                        entry.service != slot.service || entry.account != slot.account
                    });
                    if document
                        .entries
                        .get(&slot.service)
                        .and_then(|entries| entries.get(&slot.account))
                        .is_some_and(Option::is_some)
                    {
                        document.pending_cleanup.push(Cleanup {
                            service: slot.service.clone(),
                            account: slot.account.clone(),
                            destination_service: slot.service.clone(),
                            destination_account: slot.account.clone(),
                        });
                        if slot.legacy_alibaba {
                            for service in [
                                LEGACY_KEYCHAIN_SERVICE_V3,
                                LEGACY_KEYCHAIN_SERVICE_V2,
                                LEGACY_KEYCHAIN_SERVICE,
                            ] {
                                document.pending_cleanup.push(Cleanup {
                                    service: service.into(),
                                    account: LEGACY_KEYCHAIN_ACCOUNT.into(),
                                    destination_service: slot.service.clone(),
                                    destination_account: slot.account.clone(),
                                });
                            }
                            document.pending_cleanup.push(Cleanup {
                                service: slot.service.clone(),
                                account: LEGACY_MIGRATION_TOMBSTONE_ACCOUNT.into(),
                                destination_service: slot.service.clone(),
                                destination_account: slot.account.clone(),
                            });
                        }
                    }
                    self.write(&document)?;
                    // Verify the durable destination before retiring OS items.
                    let verified = self.read()?;
                    if verified
                        .entries
                        .get(&slot.service)
                        .and_then(|entries| entries.get(&slot.account))
                        != document
                            .entries
                            .get(&slot.service)
                            .and_then(|entries| entries.get(&slot.account))
                    {
                        return Err(SecretStoreError::Unavailable);
                    }
                }
                Err(error) => {
                    failed = Some(error);
                }
            }
        }
        for slot in document.pending_cleanup.clone() {
            // read() verified the destination exists, including after restart,
            // and rejects cleanup outside this app's owned credential slots.
            match self.legacy.delete(&slot.service, &slot.account) {
                Ok(()) => {
                    document.pending_cleanup.retain(|entry| {
                        entry.service != slot.service || entry.account != slot.account
                    });
                    self.write(&document)?;
                }
                Err(error) => {
                    failed = Some(error);
                }
            }
        }
        if document.pending_import.is_empty() && document.pending_cleanup.is_empty() {
            self.mark_complete()?;
        }
        failed.map_or(Ok(()), Err)
    }
    fn load(&self, service: &str, account: &str) -> Result<Option<String>, SecretStoreError> {
        let _access = self
            .access
            .lock()
            .map_err(|_| SecretStoreError::Unavailable)?;
        let document = self.read()?;
        if document
            .pending_import
            .iter()
            .any(|slot| slot.service == service && slot.account == account)
        {
            return Err(SecretStoreError::MigrationRequired);
        }
        Ok(document
            .entries
            .get(service)
            .and_then(|entries| entries.get(account))
            .cloned()
            .flatten())
    }
    fn contains(&self, service: &str, account: &str) -> Result<bool, SecretStoreError> {
        self.load(service, account).map(|value| value.is_some())
    }
    fn save(&self, service: &str, account: &str, value: &str) -> Result<(), SecretStoreError> {
        self.store(service, account, Some(value.into()))
    }
    fn delete(&self, service: &str, account: &str) -> Result<(), SecretStoreError> {
        self.store(service, account, None)
    }
}
fn validate_file(metadata: &fs::Metadata) -> Result<(), SecretStoreError> {
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_BYTES {
        return Err(SecretStoreError::Unavailable);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != current_uid()
            || metadata.mode() & 0o7777 != 0o600
            || metadata.nlink() != 1
        {
            return Err(SecretStoreError::Unavailable);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(SecretStoreError::Unavailable);
        }
    }
    Ok(())
}
#[cfg(unix)]
fn current_uid() -> u32 {
    unsafe extern "C" {
        fn getuid() -> u32;
    }
    // SAFETY: getuid takes no pointers and returns the current process identity.
    unsafe { getuid() }
}
fn private_directory(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(std::io::ErrorKind::PermissionDenied.into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != current_uid() {
            return Err(std::io::ErrorKind::PermissionDenied.into());
        }
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(std::io::ErrorKind::PermissionDenied.into());
        }
        private_windows_acl(path, true)?;
    }
    Ok(())
}
fn private_file(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(windows)]
    private_windows_acl(path, false)?;
    Ok(())
}
#[cfg(windows)]
fn private_windows_acl(path: &Path, directory: bool) -> std::io::Result<()> {
    use std::{ffi::c_void, os::windows::ffi::OsStrExt};
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text: *const u16,
            revision: u32,
            descriptor: *mut *mut c_void,
            size: *mut u32,
        ) -> i32;
        fn SetFileSecurityW(path: *const u16, information: u32, descriptor: *mut c_void) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
    }
    // Protected DACL, full access for the owner only. Directory inheritance
    // protects newly created files before any credential bytes are written.
    let text = if directory {
        "D:P(A;OICI;FA;;;OW)"
    } else {
        "D:P(A;;FA;;;OW)"
    };
    let text: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut descriptor = std::ptr::null_mut();
    // SAFETY: valid NUL-terminated strings and out-pointer; descriptor is released
    // with its required allocator on every successful conversion path.
    unsafe {
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text.as_ptr(),
            1,
            &mut descriptor,
            std::ptr::null_mut(),
        ) == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        let result = SetFileSecurityW(path.as_ptr(), 0x80000004, descriptor);
        let error = if result == 0 {
            Some(std::io::Error::last_os_error())
        } else {
            None
        };
        LocalFree(descriptor);
        error.map_or(Ok(()), Err)
    }
}

#[cfg(test)]
mod tests;
