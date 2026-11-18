//! Native desktop shell adapters. Capture stays in `audio`, window interaction
//! stays in `windows`, and portable policies stay in the shared crates.

#[cfg(target_os = "linux")]
pub(crate) mod linux;
#[cfg(any(target_os = "macos", test))]
pub(crate) mod macos;
pub(crate) mod menu;
#[cfg(any(target_os = "windows", test))]
pub(crate) mod windows;
