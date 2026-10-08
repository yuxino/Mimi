//! UI-independent models, configuration, protocols, subtitle assembly, and
//! pipeline diagnostics.

pub mod audio_input;
pub mod committer;
pub mod configuration;
pub mod credentials;
#[cfg(test)]
pub mod development_build_permissions;
pub mod development_debug;
#[cfg(any(test, feature = "development-debugger"))]
pub mod development_evidence_workspace;
pub mod diagnostics;
#[cfg(any(target_os = "macos", test))]
pub mod dock_presentation;
pub mod echo_cancellation;
pub mod local_speech;
pub mod models;
pub mod network_proxy;
pub mod openai_transcript_committer;
pub mod overlay_layout;
#[cfg(any(target_os = "macos", test))]
pub mod overlay_pointer;
#[cfg(any(target_os = "macos", target_os = "windows", test))]
pub mod pcm16;
pub mod pending_pcm;
pub mod preview_pacing;
pub mod protocols;
pub mod provider;
pub mod session;
pub mod session_archive;
pub mod subtitle_font;
pub mod subtitle_reducer;

#[cfg(any(target_os = "windows", test))]
pub mod audio_source;

pub mod support_diagnostics;

pub mod system_audio_target;
