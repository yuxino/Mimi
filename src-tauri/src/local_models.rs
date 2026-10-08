//! Private, explicitly managed model installations. No discovery path downloads.

use crate::core::local_speech::LocalSpeechModel;
use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use tokio::io::AsyncWriteExt;

static MANAGER: OnceLock<Arc<LocalModelManager>> = OnceLock::new();

pub fn initialize(root: PathBuf, resources: PathBuf, ui_test: bool) -> Result<(), &'static str> {
    let helper = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|dir| dir.join("mimi-local-speech")));
    let mut manager = LocalModelManager::new(root, helper, ui_test);
    manager.onnx_helper = Some(resources.join("local-speech-onnx").join(if cfg!(windows) {
        "mimi-local-onnx.exe"
    } else {
        "mimi-local-onnx"
    }));
    MANAGER
        .set(Arc::new(manager))
        .map_err(|_| "local_models_unavailable")
}

pub fn manager() -> Result<&'static Arc<LocalModelManager>, &'static str> {
    MANAGER.get().ok_or("local_models_unavailable")
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelPhase {
    #[default]
    Missing,
    Downloading,
    Verifying,
    Cancelling,
    Installed,
    Deleting,
    Error,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelStatus {
    pub id: LocalSpeechModel,
    pub name: &'static str,
    pub download_bytes: u64,
    pub downloaded_bytes: u64,
    pub phase: ModelPhase,
    pub installed: bool,
    pub in_use: bool,
    pub error: Option<&'static str>,
    pub available: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelsSnapshot {
    pub available: bool,
    pub models: Vec<LocalModelStatus>,
}

#[derive(Default)]
struct Operation {
    phase: ModelPhase,
    downloaded: u64,
    cancel: Option<tokio::sync::watch::Sender<bool>>,
    leases: usize,
    error: Option<&'static str>,
}

pub struct LocalModelManager {
    root: PathBuf,
    helper: Option<PathBuf>,
    onnx_helper: Option<PathBuf>,
    available: bool,
    ui_test: bool,
    operations: Mutex<BTreeMap<LocalSpeechModel, Operation>>,
}

impl LocalModelManager {
    fn new(root: PathBuf, helper: Option<PathBuf>, ui_test: bool) -> Self {
        let available = cfg!(all(target_os = "macos", target_arch = "aarch64"))
            && helper.as_ref().is_some_and(|path| path.is_file())
            && std::process::Command::new("/usr/bin/sw_vers")
                .arg("-productVersion")
                .output()
                .ok()
                .and_then(|out| String::from_utf8(out.stdout).ok())
                .and_then(|version| version.trim().split('.').next()?.parse::<u32>().ok())
                .is_some_and(|version| version >= 14);
        Self {
            root,
            helper,
            onnx_helper: None,
            available,
            ui_test,
            operations: Mutex::new(BTreeMap::new()),
        }
    }

    fn directory(&self, model: LocalSpeechModel) -> PathBuf {
        self.root.join(model.directory())
    }

    fn available_for(&self, model: LocalSpeechModel) -> bool {
        if model.is_onnx() {
            self.onnx_helper.as_ref().is_some_and(|path| {
                let Ok(metadata) = std::fs::metadata(path) else {
                    return false;
                };
                if !metadata.is_file() {
                    return false;
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if metadata.permissions().mode() & 0o111 == 0 {
                        return false;
                    }
                }
                let Some(directory) = path.parent() else {
                    return false;
                };
                onnx_libraries()
                    .iter()
                    .all(|name| directory.join(name).is_file())
            })
        } else {
            self.available
        }
    }

    fn helper_for(&self, model: LocalSpeechModel) -> Option<PathBuf> {
        if model.is_onnx() {
            self.onnx_helper.clone()
        } else {
            self.helper.clone()
        }
    }

    fn partial(&self, model: LocalSpeechModel) -> PathBuf {
        self.root.join(format!(".{}-download", model.directory()))
    }

    fn installed(&self, model: LocalSpeechModel) -> bool {
        let directory = self.directory(model);
        if !safe_directory(&self.root) || !safe_directory(&directory) {
            return false;
        }
        let receipt_path = directory.join("installed-revision");
        if !std::fs::symlink_metadata(&receipt_path).is_ok_and(|meta| {
            meta.is_file()
                && !meta.file_type().is_symlink()
                && meta.len() == model.manifest().revision.len() as u64
        }) {
            return false;
        }
        let receipt = std::fs::read_to_string(receipt_path);
        receipt.is_ok_and(|revision| revision == model.manifest().revision)
            && model.manifest().files.iter().all(|file| {
                let mut ancestor = directory.clone();
                let components: Vec<_> = file.name.split('/').collect();
                for component in &components[..components.len().saturating_sub(1)] {
                    ancestor.push(component);
                    if !safe_directory(&ancestor) {
                        return false;
                    }
                }
                std::fs::symlink_metadata(directory.join(&file.name)).is_ok_and(|meta| {
                    meta.is_file() && !meta.file_type().is_symlink() && meta.len() == file.bytes
                })
            })
    }

    pub fn snapshot(&self) -> LocalModelsSnapshot {
        let operations = self.operations.lock().unwrap();
        LocalModelsSnapshot {
            available: self.ui_test
                || LocalSpeechModel::ALL
                    .into_iter()
                    .any(|model| self.available_for(model)),
            models: LocalSpeechModel::ALL
                .into_iter()
                .map(|model| {
                    let installed = self.installed(model);
                    let operation = operations.get(&model);
                    let phase = operation.map_or(
                        if installed {
                            ModelPhase::Installed
                        } else {
                            ModelPhase::Missing
                        },
                        |op| {
                            if matches!(op.phase, ModelPhase::Missing | ModelPhase::Installed) {
                                if installed {
                                    ModelPhase::Installed
                                } else {
                                    ModelPhase::Missing
                                }
                            } else {
                                op.phase
                            }
                        },
                    );
                    LocalModelStatus {
                        id: model,
                        name: model.name(),
                        download_bytes: model.download_bytes(),
                        downloaded_bytes: operation.map_or(0, |op| op.downloaded),
                        phase,
                        installed,
                        in_use: operation.is_some_and(|op| op.leases > 0),
                        error: operation.and_then(|op| op.error),
                        available: self.ui_test || self.available_for(model),
                    }
                })
                .collect(),
        }
    }

    pub fn download(self: &Arc<Self>, model: LocalSpeechModel) -> Result<(), &'static str> {
        if self.ui_test || !self.available_for(model) {
            return Err("local_models_unavailable");
        }
        let mut operations = self.operations.lock().unwrap();
        let operation = operations.entry(model).or_default();
        if operation.cancel.is_some()
            || operation.leases > 0
            || matches!(operation.phase, ModelPhase::Deleting)
        {
            return Err("local_model_busy");
        }
        if self.installed(model) {
            return Ok(());
        }
        let (cancel, receiver) = tokio::sync::watch::channel(false);
        operation.phase = ModelPhase::Downloading;
        operation.downloaded = 0;
        operation.error = None;
        operation.cancel = Some(cancel);
        let manager = self.clone();
        tokio::spawn(async move {
            let downloader = manager.clone();
            let result =
                tokio::spawn(async move { downloader.download_files(model, receiver).await })
                    .await
                    .unwrap_or(Err("local_model_download_failed"));
            // Cleanup only this model's fixed staging directory. Never inspect
            // third-party caches or follow a directory supplied by the renderer.
            let _ = remove_managed_directory(&manager.partial(model)).await;
            let mut operations = manager.operations.lock().unwrap();
            let operation = operations.entry(model).or_default();
            operation.cancel = None;
            match result {
                Ok(()) => {
                    operation.phase = ModelPhase::Installed;
                    operation.error = None;
                }
                Err("local_model_cancelled") => {
                    operation.phase = ModelPhase::Missing;
                    operation.error = None;
                    operation.downloaded = 0;
                }
                Err(label) => {
                    operation.phase = ModelPhase::Error;
                    operation.error = Some(label);
                }
            }
        });
        Ok(())
    }

    pub fn cancel(&self, model: LocalSpeechModel) -> Result<(), &'static str> {
        let mut operations = self.operations.lock().unwrap();
        if let Some(operation) = operations.get_mut(&model) {
            if let Some(cancel) = &operation.cancel {
                operation.phase = ModelPhase::Cancelling;
                cancel.send_replace(true);
            }
        }
        Ok(())
    }

    pub async fn delete(&self, model: LocalSpeechModel) -> Result<(), &'static str> {
        if self.ui_test {
            return Err("local_models_unavailable");
        }
        {
            let mut operations = self.operations.lock().unwrap();
            let operation = operations.entry(model).or_default();
            if operation.leases > 0
                || operation.cancel.is_some()
                || matches!(operation.phase, ModelPhase::Deleting)
            {
                return Err("local_model_busy");
            }
            operation.phase = ModelPhase::Deleting;
            operation.error = None;
        }
        let result = if safe_directory(&self.root) {
            match remove_managed_directory(&self.directory(model)).await {
                Ok(()) => remove_managed_directory(&self.partial(model)).await,
                failure => failure,
            }
        } else if !self.root.exists() {
            Ok(())
        } else {
            Err("local_model_storage_failed")
        };
        let mut operations = self.operations.lock().unwrap();
        let operation = operations.entry(model).or_default();
        operation.phase = if result.is_ok() {
            ModelPhase::Missing
        } else {
            ModelPhase::Error
        };
        operation.downloaded = 0;
        operation.error = result.err();
        result
    }

    pub fn acquire(self: &Arc<Self>, model: LocalSpeechModel) -> Result<ModelLease, &'static str> {
        if self.ui_test || !self.available_for(model) {
            return Err("local_models_unavailable");
        }
        let mut operations = self.operations.lock().unwrap();
        let operation = operations.entry(model).or_default();
        if operation.cancel.is_some() || matches!(operation.phase, ModelPhase::Deleting) {
            return Err("local_model_busy");
        }
        if !self.installed(model) {
            return Err("local_model_missing");
        }
        let helper = self.helper_for(model).ok_or("local_models_unavailable")?;
        operation.leases += 1;
        Ok(ModelLease {
            manager: self.clone(),
            model,
            directory: self.directory(model),
            helper,
        })
    }

    async fn download_files(
        &self,
        model: LocalSpeechModel,
        mut cancel: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), &'static str> {
        create_private_directory(&self.root).await?;
        let partial = self.partial(model);
        remove_managed_directory(&partial).await?;
        create_private_directory(&partial).await?;
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(20))
            .read_timeout(std::time::Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.url().scheme() != "https" || attempt.previous().len() >= 10 {
                    attempt.error("download_redirect_rejected")
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .map_err(|_| "local_model_download_failed")?;
        let manifest = model.manifest();
        let mut total = 0u64;
        for file in &manifest.files {
            if !crate::core::local_speech::safe_model_file_name(&file.name) {
                return Err("local_model_integrity_failed");
            }
            let file_path = partial.join(&file.name);
            if let Some(parent) = file_path.parent() {
                create_private_directory(parent).await?;
            }
            if *cancel.borrow() {
                return Err("local_model_cancelled");
            }
            let url = format!(
                "https://huggingface.co/{}/resolve/{}/{}",
                manifest.repository, manifest.revision, file.name
            );
            let response = tokio::select! {
                _ = cancel.changed() => return Err("local_model_cancelled"),
                result = client.get(url).send() => result.map_err(|_| "local_model_download_failed")?,
            }.error_for_status().map_err(|_| "local_model_download_failed")?;
            if response
                .content_length()
                .is_some_and(|length| length != file.bytes)
            {
                return Err("local_model_integrity_failed");
            }
            let mut output = tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(file_path)
                .await
                .map_err(|_| "local_model_storage_failed")?;
            let mut stream = response.bytes_stream();
            let mut hash = Sha256::new();
            let mut count = 0u64;
            loop {
                let chunk = tokio::select! {
                    _ = cancel.changed() => return Err("local_model_cancelled"),
                    next = stream.next() => next,
                };
                let Some(chunk) = chunk else { break };
                let chunk = chunk.map_err(|_| "local_model_download_failed")?;
                count = count.saturating_add(chunk.len() as u64);
                if count > file.bytes {
                    return Err("local_model_integrity_failed");
                }
                output
                    .write_all(&chunk)
                    .await
                    .map_err(|_| "local_model_storage_failed")?;
                hash.update(&chunk);
                let mut operations = self.operations.lock().unwrap();
                operations.entry(model).or_default().downloaded = total + count;
            }
            if count != file.bytes || format!("{:x}", hash.finalize()) != file.sha256 {
                return Err("local_model_integrity_failed");
            }
            output
                .sync_all()
                .await
                .map_err(|_| "local_model_storage_failed")?;
            total += count;
        }
        if *cancel.borrow() {
            return Err("local_model_cancelled");
        }
        self.operations
            .lock()
            .unwrap()
            .entry(model)
            .or_default()
            .phase = ModelPhase::Verifying;
        // Publishing occurs only after every file was verified and synced.
        let mut receipt = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(partial.join("installed-revision"))
            .await
            .map_err(|_| "local_model_storage_failed")?;
        receipt
            .write_all(manifest.revision.as_bytes())
            .await
            .map_err(|_| "local_model_storage_failed")?;
        receipt
            .sync_all()
            .await
            .map_err(|_| "local_model_storage_failed")?;
        // Serialize the publication point with cancellation. No await while
        // holding the operations lock, and no installation after Cancel wins.
        let mut operations = self.operations.lock().unwrap();
        if *cancel.borrow() {
            return Err("local_model_cancelled");
        }
        let destination = self.directory(model);
        if destination.exists() {
            if !safe_directory(&destination) {
                return Err("local_model_storage_failed");
            }
            std::fs::remove_dir_all(&destination).map_err(|_| "local_model_storage_failed")?;
        }
        std::fs::rename(&partial, destination).map_err(|_| "local_model_storage_failed")?;
        let operation = operations.entry(model).or_default();
        operation.cancel = None;
        operation.phase = ModelPhase::Installed;
        Ok(())
    }
}

pub struct ModelLease {
    manager: Arc<LocalModelManager>,
    model: LocalSpeechModel,
    pub directory: PathBuf,
    pub helper: PathBuf,
}

impl Drop for ModelLease {
    fn drop(&mut self) {
        let mut operations = self.manager.operations.lock().unwrap();
        let operation = operations.entry(self.model).or_default();
        operation.leases = operation.leases.saturating_sub(1);
    }
}

fn safe_directory(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| {
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return false;
            }
        }
        meta.is_dir() && !meta.file_type().is_symlink()
    })
}

fn onnx_libraries() -> [&'static str; 2] {
    if cfg!(windows) {
        ["sherpa-onnx-c-api.dll", "onnxruntime.dll"]
    } else if cfg!(target_os = "macos") {
        ["libsherpa-onnx-c-api.dylib", "libonnxruntime.dylib"]
    } else {
        ["libsherpa-onnx-c-api.so", "libonnxruntime.so"]
    }
}

async fn create_private_directory(path: &Path) -> Result<(), &'static str> {
    if path.exists() && !safe_directory(path) {
        return Err("local_model_storage_failed");
    }
    let mut builder = tokio::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    builder.mode(0o700);
    builder
        .create(path)
        .await
        .map_err(|_| "local_model_storage_failed")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .await
            .map_err(|_| "local_model_storage_failed")?;
    }
    #[cfg(windows)]
    crate::settings_store::protect_local_model_directory(path)
        .map_err(|_| "local_model_storage_failed")?;
    Ok(())
}

async fn remove_managed_directory(path: &Path) -> Result<(), &'static str> {
    match tokio::fs::symlink_metadata(path).await {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {
            tokio::fs::remove_dir_all(path)
                .await
                .map_err(|_| "local_model_storage_failed")
        }
        _ => Err("local_model_storage_failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn deletion_is_bounded_and_refuses_leased_models() {
        let root = tempfile::tempdir().unwrap();
        let manager = LocalModelManager::new(root.path().join("models"), None, false);
        create_private_directory(&manager.root).await.unwrap();
        let model = LocalSpeechModel::QwenSmall;
        create_private_directory(&manager.directory(model))
            .await
            .unwrap();
        tokio::fs::write(manager.directory(model).join("partial"), b"partial")
            .await
            .unwrap();
        let unrelated = manager.root.join("user-model");
        create_private_directory(&unrelated).await.unwrap();
        manager
            .operations
            .lock()
            .unwrap()
            .entry(model)
            .or_default()
            .leases = 1;
        assert_eq!(manager.delete(model).await, Err("local_model_busy"));
        assert!(manager.directory(model).exists());
        manager
            .operations
            .lock()
            .unwrap()
            .entry(model)
            .or_default()
            .leases = 0;
        manager.delete(model).await.unwrap();
        assert!(!manager.directory(model).exists());
        assert!(unrelated.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn deletion_refuses_symlinked_installations() {
        let root = tempfile::tempdir().unwrap();
        let manager = LocalModelManager::new(root.path().join("models"), None, false);
        create_private_directory(&manager.root).await.unwrap();
        let outside = root.path().join("outside");
        create_private_directory(&outside).await.unwrap();
        std::os::unix::fs::symlink(&outside, manager.directory(LocalSpeechModel::QwenStandard))
            .unwrap();
        assert_eq!(
            manager.delete(LocalSpeechModel::QwenStandard).await,
            Err("local_model_storage_failed")
        );
        assert!(outside.exists());
    }

    // Explicit maintainer acceptance only: ordinary cargo test never downloads.
    #[tokio::test]
    #[ignore = "downloads public model weights; requires explicit acceptance directory"]
    async fn accept_real_model_download() {
        let root = PathBuf::from(
            std::env::var("MIMI_MODEL_ACCEPTANCE_DIR").expect("explicit model directory"),
        );
        let model: LocalSpeechModel = serde_json::from_str(&format!(
            "\"{}\"",
            std::env::var("MIMI_MODEL_ACCEPTANCE_ID").unwrap_or("qwenSmall".into())
        ))
        .unwrap();
        let helper = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("binaries/mimi-local-speech-aarch64-apple-darwin");
        let mut manager = LocalModelManager::new(root, Some(helper), false);
        if model.is_onnx() {
            manager.onnx_helper = Some(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("binaries/local-speech-onnx")
                    .join(if cfg!(windows) {
                        "mimi-local-onnx.exe"
                    } else {
                        "mimi-local-onnx"
                    }),
            );
        }
        let manager = Arc::new(manager);
        if !manager.installed(model) {
            manager.download(model).unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            manager.cancel(model).unwrap();
            loop {
                let pending = {
                    manager
                        .operations
                        .lock()
                        .unwrap()
                        .get(&model)
                        .is_some_and(|op| op.cancel.is_some())
                };
                if !pending {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            assert!(!manager.partial(model).exists());
            manager.download(model).unwrap();
            loop {
                let status = manager
                    .snapshot()
                    .models
                    .into_iter()
                    .find(|item| item.id == model)
                    .unwrap();
                if status.installed {
                    break;
                }
                assert!(status.error.is_none(), "download error: {:?}", status.error);
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
        let lease = manager.acquire(model).unwrap();
        assert_eq!(manager.delete(model).await, Err("local_model_busy"));
        drop(lease);
        assert!(manager.installed(model));
    }

    #[test]
    fn queries_do_not_create_storage_or_start_downloads() {
        let root = tempfile::tempdir().unwrap();
        let manager = LocalModelManager::new(root.path().join("models"), None, false);
        let snapshot = manager.snapshot();
        assert!(snapshot
            .models
            .iter()
            .all(|model| !model.installed && model.downloaded_bytes == 0));
        assert!(!manager.root.exists());
    }

    #[tokio::test]
    async fn onnx_availability_is_independent_of_mlx_and_leases_release() {
        let temporary = tempfile::tempdir().unwrap();
        let helper = temporary.path().join("worker");
        std::fs::write(&helper, b"fixture, never executed").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let mut manager = LocalModelManager::new(temporary.path().join("models"), None, false);
        manager.onnx_helper = Some(helper);
        assert!(!manager.available_for(LocalSpeechModel::QwenSmall));
        assert!(!manager.available_for(LocalSpeechModel::SenseVoice));
        for name in onnx_libraries() {
            std::fs::write(temporary.path().join(name), b"fixture, never loaded").unwrap();
        }
        assert!(manager.available_for(LocalSpeechModel::SenseVoice));
        let manager = Arc::new(manager);
        let model = LocalSpeechModel::QwenOnnx;
        assert!(matches!(manager.acquire(model), Err("local_model_missing")));
        create_private_directory(&manager.root).await.unwrap();
        create_private_directory(&manager.directory(model))
            .await
            .unwrap();
        for file in &model.manifest().files {
            let path = manager.directory(model).join(&file.name);
            create_private_directory(path.parent().unwrap())
                .await
                .unwrap();
            std::fs::File::create(path)
                .unwrap()
                .set_len(file.bytes)
                .unwrap();
        }
        std::fs::write(
            manager.directory(model).join("installed-revision"),
            model.manifest().revision.as_bytes(),
        )
        .unwrap();
        assert!(manager.installed(model));
        let lease = manager.acquire(model).unwrap();
        assert_eq!(manager.delete(model).await, Err("local_model_busy"));
        drop(lease);
        manager.delete(model).await.unwrap();
        assert!(!manager.installed(model));
        assert!(manager.onnx_helper.as_ref().unwrap().is_file());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn nested_tokenizer_symlink_is_not_an_installation() {
        let temporary = tempfile::tempdir().unwrap();
        let manager = LocalModelManager::new(temporary.path().join("models"), None, false);
        let model = LocalSpeechModel::QwenOnnx;
        create_private_directory(&manager.root).await.unwrap();
        create_private_directory(&manager.directory(model))
            .await
            .unwrap();
        let outside = temporary.path().join("user-owned");
        create_private_directory(&outside).await.unwrap();
        std::fs::write(outside.join("vocab.json"), b"user file").unwrap();
        std::os::unix::fs::symlink(&outside, manager.directory(model).join("tokenizer")).unwrap();
        std::fs::write(
            manager.directory(model).join("installed-revision"),
            model.manifest().revision.as_bytes(),
        )
        .unwrap();
        assert!(!manager.installed(model));
        manager.delete(model).await.unwrap();
        assert_eq!(
            std::fs::read(outside.join("vocab.json")).unwrap(),
            b"user file"
        );
    }
}
