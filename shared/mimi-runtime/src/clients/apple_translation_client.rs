//! Local text translation inside the same bounded final/preview pipeline.

use crate::apple_translation_support::{self, source_code, target_code};
use crate::core::models::{SourceLanguage, TargetLanguage};
use crate::core::protocols::qwen_mt::QwenMTClientError;

#[cfg(test)]
type SyntheticTranslation = std::sync::Arc<
    dyn Fn(&str, SourceLanguage, TargetLanguage) -> Result<String, QwenMTClientError> + Send + Sync,
>;

#[derive(Clone)]
pub struct AppleTranslationClient {
    source: SourceLanguage,
    target: TargetLanguage,
    #[cfg(test)]
    synthetic_translation: Option<SyntheticTranslation>,
}

impl AppleTranslationClient {
    pub fn new(source: SourceLanguage, target: TargetLanguage) -> Result<Self, QwenMTClientError> {
        source_code(source).map_err(QwenMTClientError::Apple)?;
        target_code(target).map_err(QwenMTClientError::Apple)?;
        Ok(Self {
            source,
            target,
            #[cfg(test)]
            synthetic_translation: None,
        })
    }

    #[cfg(test)]
    pub(crate) fn with_synthetic_translation(
        mut self,
        translate: impl Fn(&str, SourceLanguage, TargetLanguage) -> Result<String, QwenMTClientError>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        self.synthetic_translation = Some(std::sync::Arc::new(translate));
        self
    }

    pub async fn check_ready(&self) -> Result<(), QwenMTClientError> {
        apple_translation_support::require_installed(self.source, self.target)
            .await
            .map_err(|error| {
                QwenMTClientError::Apple(match error.as_str() {
                    "apple_translation_assets_missing" => "apple_translation_assets_missing",
                    "apple_translation_language_unsupported" => {
                        "apple_translation_language_unsupported"
                    }
                    "apple_translation_unavailable" => "apple_translation_unavailable",
                    "apple_translation_timeout" => "apple_translation_timeout",
                    _ => "apple_translation_failed",
                })
            })
    }

    pub async fn translate(
        &self,
        text: &str,
        source: Option<SourceLanguage>,
    ) -> Result<String, QwenMTClientError> {
        if let Some(text) = self.passthrough(text, source)? {
            return Ok(text);
        }
        let selected_source = source.unwrap_or(self.source);
        let source = source_code(selected_source).map_err(QwenMTClientError::Apple)?;
        let target = target_code(self.target).map_err(QwenMTClientError::Apple)?;
        #[cfg(test)]
        if let Some(translate) = &self.synthetic_translation {
            return translate(text, selected_source, self.target);
        }
        crate::apple_translation::translate(source, target, text)
            .await
            .map_err(|error| {
                QwenMTClientError::Apple(apple_translation_support::error_label(&error))
            })
    }

    /// Apple's API rejects identical languages. Explicit selections can still
    /// preserve the input when an ASR omits its optional reported language.
    fn passthrough(
        &self,
        text: &str,
        source: Option<SourceLanguage>,
    ) -> Result<Option<String>, QwenMTClientError> {
        if text.trim().is_empty() || text.len() > crate::apple_translation::MAX_TEXT_BYTES {
            return Err(QwenMTClientError::Apple("apple_translation_invalid_input"));
        }
        let source = source.unwrap_or(self.source);
        source_code(source).map_err(QwenMTClientError::Apple)?;
        Ok(self
            .target
            .matches_reported_asr(Some(source.raw_value()))
            .then(|| text.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn explicit_same_language_preserves_text_without_native_work() {
        let client =
            AppleTranslationClient::new(SourceLanguage::English, TargetLanguage::English).unwrap();
        assert_eq!(
            client.translate("  Hello.  ", None).await.unwrap(),
            "  Hello.  "
        );
        assert_eq!(
            client
                .passthrough("Hello.", Some(SourceLanguage::Japanese))
                .unwrap(),
            None
        );
        assert_eq!(
            client.passthrough("Hello.", Some(SourceLanguage::Automatic)),
            Err(QwenMTClientError::Apple(
                "apple_translation_language_unsupported"
            ))
        );
    }

    #[test]
    fn passthrough_keeps_input_bounds_and_chinese_scripts() {
        let client = AppleTranslationClient::new(
            SourceLanguage::Chinese,
            TargetLanguage::TraditionalChinese,
        )
        .unwrap();
        assert_eq!(client.passthrough("你好。", None).unwrap(), None);
        for text in [
            " \n ".to_owned(),
            "x".repeat(crate::apple_translation::MAX_TEXT_BYTES + 1),
        ] {
            assert_eq!(
                client.passthrough(&text, None),
                Err(QwenMTClientError::Apple("apple_translation_invalid_input"))
            );
        }
    }
}
