//! Retire only completed Mimi updater staging directories on Windows startup.
//! Tauri intentionally keeps these directories because its installer outlives
//! the old process. A newer running version supplies the completion boundary.
use std::fs;
use std::io;
use std::path::Path;

#[derive(Default, Debug, PartialEq)]
pub(crate) struct CleanupResult {
    pub removed: usize,
    pub failed: usize,
}

fn stable_version(value: &str) -> Option<[u32; 3]> {
    let parts: Vec<_> = value.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let mut version = [0; 3];
    for (index, part) in parts.into_iter().enumerate() {
        if part.is_empty()
            || !part.bytes().all(|byte| byte.is_ascii_digit())
            || (part.len() > 1 && part.starts_with('0'))
        {
            return None;
        }
        version[index] = part.parse().ok()?;
    }
    Some(version)
}

fn candidate_version(name: &str, current: [u32; 3]) -> Option<&str> {
    let (version, suffix) = name.strip_prefix("mimi-")?.split_once("-updater-")?;
    if suffix.len() != 6 || !suffix.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return None;
    }
    (stable_version(version)? <= current).then_some(version)
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Includes junctions and other reparse points, not only symbolic links.
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn remove_staging(directory: &Path, version: &str) -> io::Result<bool> {
    let metadata = fs::symlink_metadata(directory)?;
    if !metadata.is_dir() || is_link(&metadata) {
        return Ok(false);
    }
    let allowed = [
        format!("mimi-{version}-installer.exe"),
        format!("mimi-{version}-installer.msi"),
        // Older ZIP updater assets contained the original installer filename.
        format!("mimi_{version}_x64-setup.exe"),
        format!("mimi_{version}_x64.msi"),
        format!("mimi_{version}_x64_en-US.msi"),
    ];
    let mut files = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if !metadata.is_file()
            || is_link(&metadata)
            || !allowed
                .iter()
                .any(|name| entry.file_name() == name.as_str())
        {
            return Ok(false);
        }
        files.push(entry.path());
    }
    // Never recursively delete: unknown files appearing during cleanup stay.
    // In-use installer files fail deletion and are retried on a later launch.
    for file in files {
        fs::remove_file(file)?;
    }
    fs::remove_dir(directory)?;
    Ok(true)
}

pub(crate) fn cleanup(root: &Path, running_version: &str) -> CleanupResult {
    let mut result = CleanupResult::default();
    let Some(current) = stable_version(running_version) else {
        return result;
    };
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return result,
        Err(_) => {
            result.failed += 1;
            return result;
        }
    };
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let name = entry.file_name();
        let Some(version) = name
            .to_str()
            .and_then(|name| candidate_version(name, current))
        else {
            continue;
        };
        match remove_staging(&entry.path(), version) {
            Ok(true) => result.removed += 1,
            Ok(false) => {}
            Err(_) => result.failed += 1,
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn staging(root: &Path, name: &str, file: &str) -> std::path::PathBuf {
        let directory = root.join(name);
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join(file), b"synthetic installer").unwrap();
        directory
    }

    #[test]
    fn removes_completed_versions_and_empty_staging_but_preserves_future_updates() {
        let root = tempfile::tempdir().unwrap();
        let old = staging(
            root.path(),
            "mimi-1.5.24-updater-Ab1234",
            "mimi-1.5.24-installer.exe",
        );
        let current = staging(
            root.path(),
            "mimi-1.6.0-updater-Ab1234",
            "mimi-1.6.0-installer.msi",
        );
        let future = staging(
            root.path(),
            "mimi-1.6.1-updater-Ab1234",
            "mimi-1.6.1-installer.exe",
        );
        let empty = root.path().join("mimi-1.4.0-updater-Ab1234");
        fs::create_dir(&empty).unwrap();
        assert_eq!(
            cleanup(root.path(), "1.6.0"),
            CleanupResult {
                removed: 3,
                failed: 0
            }
        );
        assert!(!old.exists() && !current.exists() && !empty.exists());
        assert!(future.join("mimi-1.6.1-installer.exe").exists());
        assert_eq!(cleanup(root.path(), "1.6.0"), CleanupResult::default());
    }

    #[test]
    fn preserves_other_apps_malformed_versions_unknown_files_and_nested_directories() {
        let root = tempfile::tempdir().unwrap();
        let cases = [
            ("kiri-1.5.24-updater-Ab1234", "mimi-1.5.24-installer.exe"),
            (
                "mimi-1.5.24-updater-not-a-random-suffix",
                "mimi-1.5.24-installer.exe",
            ),
            (
                "mimi-1.5.24-dev-updater-Ab1234",
                "mimi-1.5.24-installer.exe",
            ),
            ("mimi-01.5.24-updater-Ab1234", "mimi-1.5.24-installer.exe"),
            ("mimi-1.5.23-updater-Ab1234", "personal.txt"),
            ("mimi-1.5.22-updater-Ab1234", "mimi-1.5.24-installer.exe"),
        ];
        for (directory, file) in cases {
            staging(root.path(), directory, file);
        }
        let mixed = staging(
            root.path(),
            "mimi-1.5.21-updater-Ab1234",
            "mimi-1.5.21-installer.exe",
        );
        fs::create_dir(mixed.join("unknown")).unwrap();
        assert_eq!(cleanup(root.path(), "1.6.0"), CleanupResult::default());
        for (directory, file) in cases {
            assert!(root.path().join(directory).join(file).exists());
        }
        assert!(mixed.join("mimi-1.5.21-installer.exe").exists());
    }

    #[test]
    fn supports_legacy_zip_installers_without_claiming_other_archive_files() {
        let root = tempfile::tempdir().unwrap();
        staging(
            root.path(),
            "mimi-1.5.22-updater-Ab1234",
            "mimi_1.5.22_x64-setup.exe",
        );
        staging(
            root.path(),
            "mimi-1.5.23-updater-Ab1234",
            "mimi_1.5.23_x64_en-US.msi",
        );
        assert_eq!(cleanup(root.path(), "1.6.0").removed, 2);
    }

    #[test]
    fn never_uses_development_or_invalid_running_versions_as_completion_proof() {
        let root = tempfile::tempdir().unwrap();
        let directory = staging(
            root.path(),
            "mimi-1.5.24-updater-Ab1234",
            "mimi-1.5.24-installer.exe",
        );
        for version in [
            "1.6.0-dev",
            "1.6.0+build",
            "1.6",
            "garbage",
            "999999999999.0.0",
        ] {
            assert_eq!(cleanup(root.path(), version), CleanupResult::default());
        }
        assert!(directory.exists());
        assert_eq!(
            cleanup(&root.path().join("missing"), "1.6.0"),
            CleanupResult::default()
        );
    }

    #[cfg(windows)]
    #[test]
    fn defers_a_locked_installer_until_the_next_launch() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = tempfile::tempdir().unwrap();
        let directory = staging(
            root.path(),
            "mimi-1.5.24-updater-Ab1234",
            "mimi-1.5.24-installer.exe",
        );
        let handle = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(directory.join("mimi-1.5.24-installer.exe"))
            .unwrap();
        assert_eq!(
            cleanup(root.path(), "1.6.0"),
            CleanupResult {
                removed: 0,
                failed: 1
            }
        );
        assert!(directory.exists());
        drop(handle);
        assert_eq!(
            cleanup(root.path(), "1.6.0"),
            CleanupResult {
                removed: 1,
                failed: 0
            }
        );
        assert!(!directory.exists());
    }

    #[cfg(unix)]
    #[test]
    fn never_follows_directory_or_installer_symlinks() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("mimi-1.5.24-installer.exe"), b"keep").unwrap();
        symlink(
            outside.path(),
            root.path().join("mimi-1.5.24-updater-Ab1234"),
        )
        .unwrap();
        let directory = root.path().join("mimi-1.5.23-updater-Ab1234");
        fs::create_dir(&directory).unwrap();
        symlink(
            outside.path().join("mimi-1.5.24-installer.exe"),
            directory.join("mimi-1.5.23-installer.exe"),
        )
        .unwrap();
        assert_eq!(cleanup(root.path(), "1.6.0"), CleanupResult::default());
        assert_eq!(
            fs::read(outside.path().join("mimi-1.5.24-installer.exe")).unwrap(),
            b"keep"
        );
    }
}
