//! Help command behavior and capability regressions.

use super::*;

#[test]
fn alibaba_setup_pages_reject_arbitrary_urls() {
    for (value, expected) in [
        (
            "guide",
            "https://help.aliyun.com/zh/model-studio/qwen3-5-livetranslate-flash-realtime",
        ),
        (
            "apiKey",
            "https://bailian.console.aliyun.com/cn-beijing/model/settings/api-key",
        ),
        (
            "workspace",
            "https://bailian.console.aliyun.com/cn-beijing/settings/workspace",
        ),
    ] {
        let page: AlibabaSetupPage = serde_json::from_value(serde_json::json!(value)).unwrap();
        assert_eq!(page.url(), expected);
    }
    for value in ["https://example.com", "file:///tmp/test", "ApiKey", ""] {
        assert!(serde_json::from_value::<AlibabaSetupPage>(serde_json::json!(value)).is_err());
    }
}

#[test]
fn tencent_setup_pages_reject_arbitrary_urls() {
    for (value, expected) in [
        ("account", "https://console.cloud.tencent.com/developer"),
        ("apiKey", "https://console.cloud.tencent.com/cam/capi"),
        ("asr", "https://console.cloud.tencent.com/asr"),
    ] {
        let page: TencentSetupPage = serde_json::from_value(serde_json::json!(value)).unwrap();
        assert_eq!(page.url(), expected);
    }
    for value in ["https://example.com", "file:///tmp/test", "Account", ""] {
        assert!(serde_json::from_value::<TencentSetupPage>(serde_json::json!(value)).is_err());
    }
}

#[test]
fn text_setup_pages_resolve_only_catalog_destinations() {
    assert_eq!(
        text_translation_setup_url("openai", TextTranslationSetupPage::ApiKey),
        Some("https://platform.openai.com/api-keys")
    );
    let catalog: serde_json::Value =
        serde_json::from_str(include_str!("../../../../shared/provider-copy.json")).unwrap();
    for preset in catalog["compatiblePresets"].as_array().unwrap() {
        let id = preset["id"].as_str().unwrap();
        assert_eq!(
            text_translation_setup_url(id, TextTranslationSetupPage::Guide),
            preset["guideUrl"].as_str()
        );
        assert_eq!(
            text_translation_setup_url(id, TextTranslationSetupPage::ApiKey),
            preset["apiKeyUrl"].as_str().filter(|url| !url.is_empty())
        );
    }
    for id in [
        "unknown",
        "https://example.com",
        "file:///tmp/test",
        "openai/../",
        "",
    ] {
        assert_eq!(
            text_translation_setup_url(id, TextTranslationSetupPage::Guide),
            None
        );
    }
    for id in ["ollama", "lmStudio", "indexTranslate"] {
        assert_eq!(
            text_translation_setup_url(id, TextTranslationSetupPage::ApiKey),
            None
        );
    }
    for page in ["https://example.com", "models", "ApiKey", ""] {
        assert!(
            serde_json::from_value::<TextTranslationSetupPage>(serde_json::json!(page)).is_err()
        );
    }
}

#[test]
fn service_setup_pages_keep_speech_and_text_products_separate() {
    for (id, expected) in [
        ("openAIRealtime", "https://platform.openai.com/api-keys"),
        (
            "volcanoEngine",
            "https://console.volcengine.com/speech/new/setting/apikeys?projectName=default",
        ),
        (
            "baiduTranslate",
            "https://console.bce.baidu.com/ai-engine/machinetranslation/overview/index",
        ),
        ("tencentCloud", "https://console.cloud.tencent.com/cam/capi"),
    ] {
        assert_eq!(
            catalog_setup_url("serviceSetups", id, TextTranslationSetupPage::ApiKey),
            Some(expected)
        );
    }
    for id in [
        "unknown",
        "https://example.com",
        "openAIRealtime/../",
        "appleSpeech",
        "customOpenAIASR",
        "",
    ] {
        assert_eq!(
            catalog_setup_url("serviceSetups", id, TextTranslationSetupPage::ApiKey),
            None
        );
    }
}
