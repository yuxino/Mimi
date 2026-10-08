//! Fixed local-model inventory. Model IDs never become arbitrary URLs or paths.

use super::models::SourceLanguage;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LocalSpeechModel {
    #[default]
    QwenSmall,
    QwenStandard,
    SenseVoice,
    QwenOnnx,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFile {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelManifest {
    pub id: LocalSpeechModel,
    pub repository: String,
    pub revision: String,
    #[serde(rename = "sourceLanguages")]
    pub source_languages: Vec<String>,
    pub files: Vec<ModelFile>,
}

impl LocalSpeechModel {
    pub const ALL: [Self; 4] = [
        Self::SenseVoice,
        Self::QwenOnnx,
        Self::QwenSmall,
        Self::QwenStandard,
    ];

    pub const fn is_onnx(self) -> bool {
        matches!(self, Self::SenseVoice | Self::QwenOnnx)
    }

    pub const fn directory(self) -> &'static str {
        match self {
            Self::QwenSmall => "qwen-small",
            Self::QwenStandard => "qwen-standard",
            Self::SenseVoice => "sense-voice",
            Self::QwenOnnx => "qwen-onnx",
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::QwenSmall => "Qwen3-ASR 0.6B",
            Self::QwenStandard => "Qwen3-ASR 1.7B",
            Self::SenseVoice => "SenseVoiceSmall · ONNX int8",
            Self::QwenOnnx => "Qwen3-ASR 0.6B · ONNX int8",
        }
    }

    pub fn manifest(self) -> &'static ModelManifest {
        static CATALOG: OnceLock<Vec<ModelManifest>> = OnceLock::new();
        CATALOG
            .get_or_init(|| {
                serde_json::from_str(include_str!("local_speech_catalog.json"))
                    .expect("fixed model catalog")
            })
            .iter()
            .find(|item| item.id == self)
            .expect("catalog contains every local model")
    }

    pub fn download_bytes(self) -> u64 {
        self.manifest().files.iter().map(|file| file.bytes).sum()
    }

    pub fn source_languages(self) -> Vec<SourceLanguage> {
        let codes = &self.manifest().source_languages;
        SourceLanguage::ALL
            .iter()
            .copied()
            .filter(|language| codes.iter().any(|code| code == language.raw_value()))
            .collect()
    }
}

/// Fixed manifests may contain tokenizer subdirectories, never traversal.
pub fn safe_model_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.split('/').all(|component| {
            !component.is_empty()
                && !component.starts_with('.')
                && component
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn catalog_is_complete_pinned_and_path_safe() {
        for model in LocalSpeechModel::ALL {
            let manifest = model.manifest();
            assert!(
                manifest.repository.starts_with("mlx-community/")
                    || manifest.repository.starts_with("csukuangfj/")
                    || manifest.repository.starts_with("csukuangfj2/")
            );
            assert_eq!(manifest.revision.len(), 40);
            assert!(manifest.revision.bytes().all(|b| b.is_ascii_hexdigit()));
            assert!(model.download_bytes() > 200_000_000);
            let mut names = BTreeSet::new();
            for file in &manifest.files {
                assert!(names.insert(&file.name));
                assert!(safe_model_file_name(&file.name));
                assert!(file.bytes > 0);
                assert_eq!(file.sha256.len(), 64);
                assert!(file.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
            }
        }
        assert!(serde_json::from_str::<LocalSpeechModel>("\"../outside\"").is_err());
    }

    #[test]
    fn language_inventory_does_not_claim_other_models_languages() {
        for model in LocalSpeechModel::ALL {
            let languages = model.source_languages();
            for expected in [
                SourceLanguage::Automatic,
                SourceLanguage::Chinese,
                SourceLanguage::English,
                SourceLanguage::Japanese,
            ] {
                assert!(languages.contains(&expected));
            }
            assert!(!languages.contains(&SourceLanguage::Norwegian));
        }
        assert!(LocalSpeechModel::QwenSmall
            .source_languages()
            .contains(&SourceLanguage::Vietnamese));
    }

    #[test]
    fn onnx_inventory_and_file_paths_preserve_supported_models() {
        assert!(LocalSpeechModel::QwenOnnx
            .source_languages()
            .contains(&SourceLanguage::English));
        assert!(!LocalSpeechModel::SenseVoice
            .source_languages()
            .contains(&SourceLanguage::French));
        for name in [
            "",
            "../model",
            "/model",
            "tokenizer/../model",
            "tokenizer//model",
            "C:\\model",
            "tokenizer/.hidden",
        ] {
            assert!(!safe_model_file_name(name));
        }
        assert!(safe_model_file_name("tokenizer/vocab.json"));
        assert_eq!(
            serde_json::to_string(&LocalSpeechModel::QwenSmall).unwrap(),
            "\"qwenSmall\""
        );
        assert_eq!(LocalSpeechModel::QwenSmall.directory(), "qwen-small");
    }
}
