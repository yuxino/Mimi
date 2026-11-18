//! Documented recognition hints for the first-party ASR routes.
//! Catalog provenance and exact wire spellings are shared with both UIs.
use crate::core::models::SourceLanguage;
use crate::core::services::provider::ProviderKind;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::OnceLock;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Catalog {
    source_languages: Vec<String>,
    wire_by_code: BTreeMap<String, String>,
}

fn catalog(provider: ProviderKind) -> &'static Catalog {
    static CATALOGS: OnceLock<BTreeMap<String, Catalog>> = OnceLock::new();
    let catalogs = CATALOGS.get_or_init(|| {
        let all: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../provider-language-catalogs.json"
        )))
        .expect("checked provider catalogs");
        ["openAIASR", "googleGeminiASR"]
            .into_iter()
            .map(|name| {
                (
                    name.to_owned(),
                    serde_json::from_value(all[name].clone()).expect("checked ASR catalog"),
                )
            })
            .collect()
    });
    &catalogs[if provider == ProviderKind::GoogleGeminiLive {
        "googleGeminiASR"
    } else {
        "openAIASR"
    }]
}

pub fn supports_language(provider: ProviderKind, language: SourceLanguage) -> bool {
    catalog(provider)
        .source_languages
        .iter()
        .any(|code| code == language.raw_value())
}

pub fn language_hint(provider: ProviderKind, language: SourceLanguage) -> Option<&'static str> {
    catalog(provider)
        .wire_by_code
        .get(language.raw_value())
        .map(String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_advertised_explicit_hint_has_an_exact_wire_mapping() {
        for provider in [
            ProviderKind::OpenAIRealtime,
            ProviderKind::AzureOpenAIRealtime,
            ProviderKind::GoogleGeminiLive,
        ] {
            for source in SourceLanguage::ALL {
                if supports_language(provider, source) && source != SourceLanguage::Automatic {
                    assert!(
                        language_hint(provider, source).is_some(),
                        "{provider:?} {source:?}"
                    );
                }
            }
        }
        assert_eq!(
            language_hint(ProviderKind::OpenAIRealtime, SourceLanguage::Chinese),
            Some("zh-cn")
        );
        assert_eq!(
            language_hint(ProviderKind::GoogleGeminiLive, SourceLanguage::English),
            Some("en-US")
        );
    }
}
