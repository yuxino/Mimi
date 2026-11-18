//! AppKit presentation and normal-quit integration.

pub(crate) mod dock;
#[cfg(target_os = "macos")]
pub(crate) mod native_quit;
