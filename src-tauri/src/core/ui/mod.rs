//! Ui responsibilities.

#[cfg(any(target_os = "macos", test))]
pub mod dock_presentation;
pub mod overlay_layout;
#[cfg(any(target_os = "macos", test))]
pub mod overlay_pointer;
pub mod subtitle_font;
