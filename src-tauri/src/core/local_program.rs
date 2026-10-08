//! Non-secret configuration for explicitly selected user-owned local runtimes.
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LocalProgramEngine {
    #[default]
    WhisperCpp,
    MimiStdio,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalProgramConfiguration {
    pub engine: LocalProgramEngine,
    pub executable: String,
    pub model_path: String,
    #[serde(default)]
    pub arguments: Vec<String>,
}

impl std::fmt::Debug for LocalProgramConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalProgramConfiguration")
            .field("engine", &self.engine)
            .finish_non_exhaustive()
    }
}

impl LocalProgramConfiguration {
    pub fn validated(&self) -> Result<Self, &'static str> {
        let executable = self.executable.trim();
        let model_path = self.model_path.trim();
        if !absolute_path(executable) {
            return Err("local_program_executable_invalid");
        }
        if !absolute_path(model_path) {
            return Err("local_program_model_invalid");
        }
        if self.arguments.len() > 32
            || self
                .arguments
                .iter()
                .any(|arg| arg.len() > 4096 || arg.contains(['\0', '\n', '\r']))
            || self.arguments.iter().map(String::len).sum::<usize>() > 16_384
        {
            return Err("local_program_arguments_invalid");
        }
        // Mimi owns Whisper's input/output and language. Additional arguments
        // must not introduce other files, output destinations or prompts.
        if self.engine == LocalProgramEngine::WhisperCpp && !whisper_arguments(&self.arguments) {
            return Err("local_program_arguments_invalid");
        }
        Ok(Self {
            executable: executable.to_owned(),
            model_path: model_path.to_owned(),
            ..self.clone()
        })
    }

    pub fn accepts_language(&self, language: &str) -> bool {
        self.engine == LocalProgramEngine::MimiStdio
            || whisper_languages().iter().any(|code| code == language)
    }

    pub fn worker_arguments(&self, language: &str) -> Vec<String> {
        self.arguments
            .iter()
            .map(|arg| {
                arg.replace("{model}", &self.model_path)
                    .replace("{language}", language)
            })
            .collect()
    }
}

fn whisper_languages() -> &'static [String] {
    static LANGUAGES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    LANGUAGES.get_or_init(|| {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Catalog {
            whisper_cpp: Vec<String>,
        }
        serde_json::from_str::<Catalog>(include_str!(
            "../../../shared/local-program-languages.json"
        ))
        .expect("checked-in Whisper language catalog")
        .whisper_cpp
    })
}

// Only tuning flags are accepted; arbitrary positional input files, prompts,
// language/output overrides and early-exit flags break the recognition contract.
fn whisper_arguments(arguments: &[String]) -> bool {
    let mut args = arguments.iter();
    while let Some(option) = args.next() {
        match option.as_str() {
            "-ng" | "--no-gpu" | "-fa" | "--flash-attn" | "-nfa" | "--no-flash-attn" | "-nf"
            | "--no-fallback" => {}
            "-t" | "--threads" | "-p" | "--processors" | "-bs" | "--beam-size" | "-bo"
            | "--best-of" => {
                if !args
                    .next()
                    .and_then(|value| value.parse::<u16>().ok())
                    .is_some_and(|value| (1..=256).contains(&value))
                {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

fn absolute_path(value: &str) -> bool {
    value.len() <= 4096
        && !value.chars().any(char::is_control)
        && (value.starts_with('/')
            || value.starts_with("\\\\")
            || (value.as_bytes().get(1) == Some(&b':')
                && value
                    .as_bytes()
                    .get(2)
                    .is_some_and(|c| matches!(c, b'/' | b'\\'))
                && value.as_bytes()[0].is_ascii_alphabetic()))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> LocalProgramConfiguration {
        LocalProgramConfiguration {
            engine: LocalProgramEngine::WhisperCpp,
            executable: "/apps/whisper-cli".into(),
            model_path: "/models/model.bin".into(),
            arguments: vec![],
        }
    }
    #[test]
    fn validates_cross_platform_absolute_paths_without_exposing_them() {
        for path in [
            "/apps/own model",
            "C:\\models\\own model.bin",
            "\\\\server\\share\\model",
        ] {
            let mut value = config();
            value.model_path = path.into();
            assert!(value.validated().is_ok());
        }
        for path in ["", "whisper-cli", "~/model.bin", "/models/\nmodel"] {
            let mut value = config();
            value.model_path = path.into();
            assert_eq!(value.validated(), Err("local_program_model_invalid"));
        }
        assert!(!format!("{:?}", config()).contains("/apps"));
    }
    #[test]
    fn owned_whisper_options_and_unbounded_arguments_are_rejected() {
        for flag in [
            "--output-file=/outside",
            "-f",
            "-m",
            "--prompt",
            "--translate",
            "--output-json",
            "--vad-model",
            "--help",
            "/other/audio.wav",
            "--unknown",
        ] {
            let mut value = config();
            value.arguments.push(flag.into());
            assert_eq!(value.validated(), Err("local_program_arguments_invalid"));
        }
        let mut value = config();
        value.arguments = vec!["-t".into(), "4".into(), "-ng".into()];
        assert!(value.validated().is_ok());
        value.arguments = vec!["x".repeat(4097)];
        assert!(value.validated().is_err());
    }
    #[test]
    fn whisper_languages_exclude_non_whisper_protocol_codes() {
        let mut value = config();
        assert!(value.accepts_language("auto"));
        assert!(value.accepts_language("ja"));
        assert!(!value.accepts_language("zh_en"));
        assert!(!value.accepts_language("zxx"));
        value.engine = LocalProgramEngine::MimiStdio;
        assert!(value.accepts_language("zh_en"));
    }
    #[test]
    fn worker_paths_are_single_arguments_and_not_shell_commands() {
        let mut value = config();
        value.engine = LocalProgramEngine::MimiStdio;
        value.model_path = "/models/a $(no execution)".into();
        value.arguments = vec![
            "--model".into(),
            "{model}".into(),
            "--language={language}".into(),
        ];
        assert_eq!(
            value.worker_arguments("ja"),
            ["--model", "/models/a $(no execution)", "--language=ja"]
        );
    }
}
