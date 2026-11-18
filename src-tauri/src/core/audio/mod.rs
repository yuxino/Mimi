//! Audio responsibilities.

pub mod audio_input;
pub mod pending_pcm;

#[cfg(any(target_os = "windows", test))]
pub mod audio_source;
pub mod echo_cancellation;
#[cfg(any(target_os = "macos", target_os = "windows", test))]
pub mod pcm16;
pub mod system_audio_target;
