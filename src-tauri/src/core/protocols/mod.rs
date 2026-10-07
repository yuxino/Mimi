//! Wire protocols for mimi's built-in translation providers.

pub mod audio3;
pub mod azure_openai_realtime;
pub mod baidu_translate;
pub mod custom_speech;
pub mod gemini_live;
pub mod live_translate;
pub mod openai_compatible;
pub mod openai_realtime;
pub mod qwen_mt;
pub mod tencent_cloud;
pub mod volcano_engine;
pub mod xai_realtime;

pub mod deepl;
pub mod deepl_languages;
pub mod deeplx;

#[cfg(test)]
mod translation_contract_tests;
