//! Provider identity, route modes, and text translation selection.
//!
//! Profiles contain display metadata only. Credentials deliberately live in
//! a private local file and must never be serialized with a profile.

mod capabilities;
mod profile;
#[cfg(test)]
mod tests;

pub use capabilities::{
    realtime_asr_capabilities, xai_asr_capabilities, ProviderCapabilities, ProviderPreferences,
};
pub use profile::{
    CustomSpeechLanguagesPatch, ProfileLanguagePreset, ProfileLanguagePresetPatch, ServiceProfile,
    ServiceProfileError, TextTranslationName,
};

use serde::{Deserialize, Serialize};
use std::fmt;

pub const DEFAULT_ALIBABA_PROFILE_ID: &str = "alibaba-default";
pub const DEFAULT_XAI_TEXT_MODEL: &str = "grok-4.3";
pub const XAI_TEXT_ENDPOINT: &str = "https://api.x.ai/v1/chat/completions";

pub fn default_xai_text_model() -> String {
    DEFAULT_XAI_TEXT_MODEL.to_owned()
}

/// Missing metadata preserves the existing integrated translation route.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpeechTranslationMode {
    #[default]
    #[serde(rename = "simultaneous")]
    Simultaneous,
    #[serde(rename = "asrMt")]
    AsrMt,
}

/// Missing metadata preserves the historical turn-based Voice route.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum XAISpeechMode {
    #[default]
    #[serde(rename = "voice")]
    Voice,
    #[serde(rename = "asrMt")]
    AsrMt,
}

/// Experimental integrated audio routes; absent metadata keeps the ASR + MT chain.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlibabaRealtimeMode {
    #[default]
    // Retire the short-lived Omni experiment without breaking its saved catalog.
    #[serde(rename = "disabled", alias = "omni")]
    Disabled,
    #[serde(rename = "liveTranslate")]
    LiveTranslate,
}

pub fn alibaba_workspace_id_is_valid(workspace: &str) -> bool {
    !workspace.is_empty()
        && workspace.len() <= 63
        && workspace
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        && workspace
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && workspace
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
}

impl AlibabaRealtimeMode {
    pub const fn model_name(self) -> Option<&'static str> {
        match self {
            Self::Disabled => None,
            Self::LiveTranslate => Some("qwen3.8-livetranslate-flash-realtime"),
        }
    }
}

/// A stable identifier used in settings JSON and provider-scoped keychain
/// account names. The serde values are part of the frontend contract.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderKind {
    #[default]
    #[serde(rename = "alibabaCloud")]
    AlibabaCloud,
    #[serde(rename = "openAIRealtime")]
    OpenAIRealtime,
    #[serde(rename = "googleGeminiLive")]
    GoogleGeminiLive,
    #[serde(rename = "azureOpenAIRealtime")]
    AzureOpenAIRealtime,
    #[serde(rename = "volcanoEngine")]
    VolcanoEngine,
    #[serde(rename = "tencentCloud")]
    TencentCloud,
    #[serde(rename = "baiduTranslate")]
    BaiduTranslate,
    #[serde(rename = "xAIRealtime")]
    XAIRealtime,
    #[serde(rename = "deepLX")]
    DeepLX,
    #[serde(rename = "customDashScopeASR")]
    CustomDashScopeASR,
    #[serde(rename = "customOpenAIASR")]
    CustomOpenAIASR,
    #[serde(rename = "appleSpeech")]
    AppleSpeech,
}

impl ProviderKind {
    pub const fn wire_value(self) -> &'static str {
        match self {
            Self::AlibabaCloud => "alibabaCloud",
            Self::OpenAIRealtime => "openAIRealtime",
            Self::GoogleGeminiLive => "googleGeminiLive",
            Self::AzureOpenAIRealtime => "azureOpenAIRealtime",
            Self::VolcanoEngine => "volcanoEngine",
            Self::TencentCloud => "tencentCloud",
            Self::BaiduTranslate => "baiduTranslate",
            Self::XAIRealtime => "xAIRealtime",
            Self::DeepLX => "deepLX",
            Self::CustomDashScopeASR => "customDashScopeASR",
            Self::CustomOpenAIASR => "customOpenAIASR",
            Self::AppleSpeech => "appleSpeech",
        }
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::AlibabaCloud => "Alibaba Cloud",
            Self::OpenAIRealtime => "OpenAI Realtime",
            Self::GoogleGeminiLive => "Google Gemini",
            Self::AzureOpenAIRealtime => "Azure OpenAI",
            Self::VolcanoEngine => "Volcano Engine",
            Self::TencentCloud => "Tencent Cloud",
            Self::BaiduTranslate => "Baidu Translate",
            Self::XAIRealtime => "xAI Grok",
            Self::DeepLX => "DeepLX (Audio 3.0 ASR)",
            Self::CustomDashScopeASR => "Custom DashScope ASR",
            Self::CustomOpenAIASR => "Custom OpenAI ASR",
            Self::AppleSpeech => "Apple Speech",
        }
    }

    pub const fn supports_speech_translation_mode(self) -> bool {
        matches!(
            self,
            Self::OpenAIRealtime | Self::GoogleGeminiLive | Self::AzureOpenAIRealtime
        )
    }

    pub const fn is_custom_speech(self) -> bool {
        matches!(self, Self::CustomDashScopeASR | Self::CustomOpenAIASR)
    }

    pub const fn is_standalone_asr(self) -> bool {
        self.is_custom_speech() || matches!(self, Self::AppleSpeech)
    }

    pub const fn supports_text_translation(self) -> bool {
        matches!(self, Self::AlibabaCloud | Self::DeepLX) || self.is_standalone_asr()
    }

    pub const fn uses_api_key_only(self) -> bool {
        matches!(
            self,
            Self::AlibabaCloud
                | Self::OpenAIRealtime
                | Self::GoogleGeminiLive
                | Self::VolcanoEngine
                | Self::XAIRealtime
        )
    }
}

impl fmt::Display for ProviderKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.display_name())
    }
}

/// Independent text translation for supported speech-recognition chains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TextTranslation {
    #[serde(rename = "followService")]
    FollowService,
    #[serde(rename = "xAI")]
    XAI,
    #[serde(rename = "deepL")]
    DeepL,
    #[serde(rename = "deepLX")]
    DeepLX,
    #[serde(rename = "openAICompatible")]
    OpenAICompatible,
    #[serde(rename = "chatMock")]
    ChatMock,
    #[serde(rename = "apple")]
    Apple,
}

impl TextTranslation {
    pub const fn uses_chat_completions(self) -> bool {
        matches!(self, Self::OpenAICompatible | Self::ChatMock | Self::XAI)
    }
}
