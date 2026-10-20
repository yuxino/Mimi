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
        let mut store = Self::for_app(
            directory,
            super::DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE,
            false,
        );
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
                || profile.provider == crate::core::provider::ProviderKind::AppleSpeech
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
            #[cfg(target_os = "macos")]
            options.custom_flags(0x100); // O_NOFOLLOW
            #[cfg(not(target_os = "macos"))]
            options.custom_flags(0x20000);
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
        super::replace_file(file.path(), &self.complete_path)
            .map_err(|_| SecretStoreError::Unavailable)?;
        if let Some(parent) = self.complete_path.parent() {
            super::sync_directory(parent);
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
        super::replace_file(file.path(), &self.path).map_err(|_| SecretStoreError::Unavailable)?;
        super::sync_directory(parent);
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
mod tests {
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
            fn load(
                &self,
                service: &str,
                account: &str,
            ) -> Result<Option<String>, SecretStoreError> {
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
        provider: crate::core::provider::ProviderKind,
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

    fn complete_speech_request(key: &str) -> crate::core::credentials::ProviderCredentials {
        crate::core::credentials::ProviderCredentials::CustomSpeech {
            endpoint: "wss://speech.example/realtime".into(),
            model: "synthetic-model".into(),
            api_key: key.into(),
        }
    }

    #[test]
    fn complete_custom_speech_replacement_retires_only_its_pending_import() {
        use crate::core::{credentials::ProviderCredentials, provider::ProviderKind};
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
            let text: super::super::TextTranslationDestination =
                serde_json::from_str(&text).unwrap();
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
            pending_custom_settings(crate::core::provider::ProviderKind::CustomOpenAIASR);
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
            pending_custom_settings(crate::core::provider::ProviderKind::CustomOpenAIASR);
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
            pending_custom_settings(crate::core::provider::ProviderKind::CustomOpenAIASR);
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
        use crate::core::{
            credentials::ProviderCredentials,
            provider::{ProviderKind, TextTranslation},
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
        let store = SettingsStore::load_with_secret(
            directory.path().into(),
            false,
            Box::new(local),
            service,
        );
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
}
