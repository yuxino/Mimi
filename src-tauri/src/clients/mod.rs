//! Native and network clients for live recognition and translation.

pub mod apple_speech_client;
pub mod apple_translation_client;

pub mod audio3_client;
pub mod azure_openai_realtime_client;
pub mod baidu_translate_client;
pub mod custom_speech_client;
pub mod gemini_live_client;
pub mod high_quality_client;
pub mod live_translate_client;
pub mod openai_compatible_client;
pub mod openai_realtime_client;
pub mod provider_events;
pub mod provider_network;
pub mod qwen_mt_client;
pub mod recognition_client;
pub mod tencent_cloud_client;
pub mod translation_client;
pub mod volcano_engine_client;
pub mod xai_realtime_client;

pub mod connection_diagnostics;

pub mod deepl_client;
pub mod deeplx_client;
