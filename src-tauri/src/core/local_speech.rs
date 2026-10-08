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
    pub const ALL: [Self; 2] = [Self::QwenSmall, Self::QwenStandard];

    pub const fn directory(self) -> &'static str {
        match self {
            Self::QwenSmall => "qwen-small",
            Self::QwenStandard => "qwen-standard",
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::QwenSmall => "Qwen3-ASR 0.6B",
            Self::QwenStandard => "Qwen3-ASR 1.7B",
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn catalog_is_complete_pinned_and_path_safe() {
        for model in LocalSpeechModel::ALL {
            let manifest = model.manifest();
            assert!(manifest.repository.starts_with("mlx-community/"));
            assert_eq!(manifest.revision.len(), 40);
            assert!(manifest.revision.bytes().all(|b| b.is_ascii_hexdigit()));
            assert!(model.download_bytes() > 600_000_000);
            let mut names = BTreeSet::new();
            for file in &manifest.files {
                assert!(names.insert(&file.name));
                assert!(!file.name.starts_with('.'));
                assert!(file
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)));
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
}
