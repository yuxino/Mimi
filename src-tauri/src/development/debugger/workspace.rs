//! Evidence workspace selection, quota checks, and private file I/O primitives.

use super::DebuggerState;
use crate::core::development::development_evidence_workspace::{
    valid_workspace_name, EvidenceWorkspace, EXTRA_WORKSPACE_LIMIT,
};
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};

// Both closures are deliberately evaluated only after the exact dev gate.
// Production must neither read this selector nor create an evidence directory.
pub(super) fn initialize_evidence_storage(
    available: bool,
    app_directory: impl FnOnce() -> Option<PathBuf>,
    selector: impl FnOnce() -> Option<std::ffi::OsString>,
) -> Result<Option<(PathBuf, PathBuf, EvidenceWorkspace)>, &'static str> {
    if !available {
        return Ok(None);
    }
    let value = selector();
    let workspace = EvidenceWorkspace::parse(
        value
            .as_ref()
            .map(|value| {
                value
                    .to_str()
                    .ok_or("development_evidence_workspace_invalid")
            })
            .transpose()?,
    )?;
    let app_directory = app_directory().ok_or("development_evidence_storage_failed")?;
    let root = select_evidence_root(&app_directory, &workspace)?;
    Ok(Some((app_directory, root, workspace)))
}

fn require_directory(path: &Path) -> Result<(), &'static str> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "development_evidence_storage_failed")?;
    if !metadata.is_dir() {
        return Err("development_evidence_workspace_invalid");
    }
    Ok(())
}

pub(super) fn workspace_count(container: &Path) -> Result<usize, &'static str> {
    let mut count = 0;
    for entry in fs::read_dir(container).map_err(|_| "development_evidence_storage_failed")? {
        let entry = entry.map_err(|_| "development_evidence_storage_failed")?;
        let file_type = entry
            .file_type()
            .map_err(|_| "development_evidence_storage_failed")?;
        let name = entry.file_name();
        if name == ".DS_Store" {
            // Finder metadata is not an evidence catalog. Keep this exception
            // narrow and bounded; links and oversized files remain invalid.
            if !file_type.is_file()
                || entry
                    .metadata()
                    .map_err(|_| "development_evidence_storage_failed")?
                    .len()
                    > 64 * 1024
            {
                return Err("development_evidence_workspace_invalid");
            }
            continue;
        }
        if !file_type.is_dir() || !name.to_str().is_some_and(valid_workspace_name) {
            return Err("development_evidence_workspace_invalid");
        }
        count += 1;
        if count > EXTRA_WORKSPACE_LIMIT {
            return Err("development_evidence_workspace_limit");
        }
    }
    Ok(count)
}

fn existing_workspace_count(container: &Path) -> Result<usize, &'static str> {
    match fs::symlink_metadata(container) {
        Ok(_) => {
            require_directory(container)?;
            workspace_count(container)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(_) => Err("development_evidence_storage_failed"),
    }
}

pub(super) fn workspace_directory(app_directory: &Path, workspace: &EvidenceWorkspace) -> PathBuf {
    match workspace {
        EvidenceWorkspace::Default => app_directory.join("development-evidence"),
        EvidenceWorkspace::Named(name) => app_directory
            .join("development-evidence-workspaces")
            .join(name),
    }
}

pub(super) fn select_evidence_root(
    app_directory: &Path,
    workspace: &EvidenceWorkspace,
) -> Result<PathBuf, &'static str> {
    private_directory(app_directory).map_err(|error| {
        if error.kind() == std::io::ErrorKind::InvalidInput {
            "development_evidence_workspace_invalid"
        } else {
            "development_evidence_storage_failed"
        }
    })?;
    let root = workspace_directory(app_directory, workspace);
    let container = app_directory.join("development-evidence-workspaces");
    existing_workspace_count(&container)?;
    if matches!(workspace, EvidenceWorkspace::Named(_)) {
        private_directory(&container).map_err(|error| {
            if error.kind() == std::io::ErrorKind::InvalidInput {
                "development_evidence_workspace_invalid"
            } else {
                "development_evidence_storage_failed"
            }
        })?;
        let count = workspace_count(&container)?;
        match fs::symlink_metadata(&root) {
            Ok(_) => require_directory(&root)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if count >= EXTRA_WORKSPACE_LIMIT {
                    return Err("development_evidence_workspace_limit");
                }
                let mut builder = fs::DirBuilder::new();
                builder.recursive(false);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;
                    builder.mode(0o700);
                }
                let created = match builder.create(&root) {
                    Ok(()) => true,
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
                    Err(_) => return Err("development_evidence_storage_failed"),
                };
                // Recheck after creation: competing initializations cannot retain
                // a ninth extra catalog. Only our newly created empty directory
                // is eligible for rollback; existing evidence is never removed.
                if let Err(error) = workspace_count(&container) {
                    if created {
                        let _ = fs::remove_dir(&root);
                    }
                    return Err(error);
                }
                require_directory(&root)?;
            }
            Err(_) => return Err("development_evidence_storage_failed"),
        }
    }
    private_directory(&root).map_err(|error| {
        if error.kind() == std::io::ErrorKind::InvalidInput {
            "development_evidence_workspace_invalid"
        } else {
            "development_evidence_storage_failed"
        }
    })?;
    Ok(root)
}

pub(super) fn evidence_root(debugger: &DebuggerState) -> Result<PathBuf, &'static str> {
    if let Some(error) = debugger.storage_error {
        return Err(error);
    }
    let app_directory = debugger
        .app_directory
        .as_deref()
        .ok_or("development_evidence_storage_failed")?;
    require_directory(app_directory)?;
    let container = app_directory.join("development-evidence-workspaces");
    if matches!(debugger.workspace, EvidenceWorkspace::Named(_)) {
        require_directory(&container)?;
    }
    existing_workspace_count(&container)?;
    let root = debugger
        .root
        .as_ref()
        .ok_or("development_evidence_storage_failed")?;
    require_directory(root)?;
    Ok(root.clone())
}

pub(super) fn private_directory(path: &Path) -> std::io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.is_dir() => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "private evidence directory must be a regular directory",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    fs::create_dir_all(path)?;
    if !fs::symlink_metadata(path)?.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "private evidence directory must be a regular directory",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub(super) fn private_file(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

pub(super) fn open_case_file(
    directory: &Path,
    relative: &str,
    maximum: u64,
) -> Result<File, &'static str> {
    let path = directory.join(relative);
    let before = fs::symlink_metadata(&path).map_err(|_| "development_case_read_failed")?;
    if !before.is_file() || before.len() > maximum {
        return Err("development_case_invalid");
    }
    let canonical_directory = directory
        .canonicalize()
        .map_err(|_| "development_case_invalid")?;
    if !path
        .canonicalize()
        .map_err(|_| "development_case_invalid")?
        .starts_with(canonical_directory)
    {
        return Err("development_case_invalid");
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        #[cfg(target_os = "macos")]
        options.custom_flags(0x0100 | 0x0004); // O_NOFOLLOW | O_NONBLOCK.
        #[cfg(not(target_os = "macos"))]
        options.custom_flags(0x20000 | 0x0800);
    }
    let file = options
        .open(path)
        .map_err(|_| "development_case_read_failed")?;
    let after = file
        .metadata()
        .map_err(|_| "development_case_read_failed")?;
    if !after.is_file() || after.len() > maximum {
        return Err("development_case_invalid");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.ino() != after.ino() || before.dev() != after.dev() {
            return Err("development_case_invalid");
        }
    }
    Ok(file)
}

pub(super) fn copy_case_file(
    directory: &Path,
    relative: &str,
    destination: &Path,
    maximum: u64,
) -> Result<bool, &'static str> {
    match fs::symlink_metadata(directory.join(relative)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err("development_export_failed"),
        Ok(_) => {}
    }
    let file = open_case_file(directory, relative, maximum)?;
    let mut destination = private_file(destination).map_err(|_| "development_export_failed")?;
    let bytes = std::io::copy(&mut file.take(maximum + 1), &mut destination)
        .map_err(|_| "development_export_failed")?;
    if bytes > maximum {
        return Err("development_export_failed");
    }
    Ok(true)
}

pub(super) fn stored_bytes(root: &Path) -> std::io::Result<u64> {
    if !root.exists() {
        return Ok(0);
    }
    let mut bytes = 0_u64;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            bytes = bytes.saturating_add(stored_bytes(&entry.path())?);
        } else if entry.file_type()?.is_file() {
            bytes = bytes.saturating_add(entry.metadata()?.len());
        }
    }
    Ok(bytes)
}
