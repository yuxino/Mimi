//! Explicit local-program adapters. User files are borrowed, never managed/deleted.
use crate::core::local_program::{LocalProgramConfiguration, LocalProgramEngine};
use crate::core::local_speech::LocalSpeechModel;
use crate::core::models::SourceLanguage;
use crate::local_models::{self, ModelLease};
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub enum LocalWorkerRuntime {
    Managed(LocalSpeechModel),
    Program(LocalProgramConfiguration),
}

pub struct WorkerLaunch {
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    pub lease: Option<ModelLease>,
}

impl LocalWorkerRuntime {
    pub fn prepare(&self, source: SourceLanguage) -> Result<WorkerLaunch, &'static str> {
        match self {
            Self::Managed(model) => {
                let lease = local_models::manager()?.acquire(*model)?;
                if model.is_onnx() {
                    if !model.source_languages().contains(&source) {
                        return Err("local_model_runtime_failed");
                    }
                    return Ok(WorkerLaunch {
                        executable: lease.helper.clone(),
                        arguments: vec![
                            model.directory().into(),
                            lease.directory.to_string_lossy().into_owned(),
                            source.raw_value().into(),
                        ],
                        lease: Some(lease),
                    });
                }
                let metal = lease
                    .helper
                    .parent()
                    .ok_or("local_models_unavailable")?
                    .join("../Resources/mlx.metallib");
                Ok(WorkerLaunch {
                    executable: lease.helper.clone(),
                    arguments: vec![
                        model.directory().into(),
                        lease.directory.to_string_lossy().into_owned(),
                        source.raw_value().into(),
                        metal.to_string_lossy().into_owned(),
                    ],
                    lease: Some(lease),
                })
            }
            Self::Program(config) => {
                validate_files(config)?;
                Ok(WorkerLaunch {
                    executable: config.executable.clone().into(),
                    arguments: config.worker_arguments(source.raw_value()),
                    lease: None,
                })
            }
        }
    }

    pub fn accepts_language(&self, language: &str) -> bool {
        match self {
            Self::Managed(model) => model
                .source_languages()
                .iter()
                .any(|source| source.raw_value() == language),
            Self::Program(_) => SourceLanguage::ALL
                .iter()
                .any(|source| source.raw_value() == language),
        }
    }
}

pub fn validate_files(config: &LocalProgramConfiguration) -> Result<(), &'static str> {
    let config = config.validated()?;
    let executable =
        std::fs::metadata(&config.executable).map_err(|_| "local_program_executable_missing")?;
    if !executable.is_file() {
        return Err("local_program_executable_missing");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if executable.permissions().mode() & 0o111 == 0 {
            return Err("local_program_not_executable");
        }
    }
    let model = std::fs::metadata(&config.model_path).map_err(|_| "local_program_model_missing")?;
    if (config.engine == LocalProgramEngine::WhisperCpp && !model.is_file())
        || (!model.is_file() && !model.is_dir())
    {
        return Err("local_program_model_missing");
    }
    Ok(())
}

pub fn command(executable: &Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(executable);
    command.kill_on_drop(true);
    #[cfg(target_os = "windows")]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW; stderr never opens a console.
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_files_are_sanitized_and_do_not_change_user_files() {
        let root = tempfile::tempdir().unwrap();
        let model = root.path().join("model.bin");
        std::fs::write(&model, b"owned by user").unwrap();
        let config = LocalProgramConfiguration {
            engine: LocalProgramEngine::WhisperCpp,
            executable: root.path().join("missing").to_string_lossy().into_owned(),
            model_path: model.to_string_lossy().into_owned(),
            arguments: vec![],
        };
        assert_eq!(
            validate_files(&config),
            Err("local_program_executable_missing")
        );
        assert_eq!(std::fs::read(model).unwrap(), b"owned by user");
    }
}
