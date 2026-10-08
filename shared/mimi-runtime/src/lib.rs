//! One provider runtime used directly by desktop and through Android JNI.
//! Native capture, credentials storage and product rendering remain adapters.

pub mod apple_speech;
pub mod apple_speech_support;
pub mod apple_translation;
pub mod apple_translation_support;
pub mod audio;
pub mod clients;
pub mod core;
#[cfg(any(test, feature = "development-debugger"))]
pub mod development_audio;
#[cfg(not(any(test, feature = "development-debugger")))]
#[path = "development_audio_disabled.rs"]
pub mod development_audio;
#[cfg(any(test, feature = "development-debugger"))]
pub mod development_content;
#[cfg(not(any(test, feature = "development-debugger")))]
#[path = "development_content_disabled.rs"]
pub mod development_content;
