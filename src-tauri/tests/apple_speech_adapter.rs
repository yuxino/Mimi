//! Exercise the shared native adapter with the desktop package's real Swift
//! linkage. Cargo does not execute a dependency crate's unit tests; the portable
//! shared checks cannot link this macOS-only bridge by themselves.
#![cfg(all(target_os = "macos", target_arch = "aarch64"))]

// Retain the desktop package's native-library linkage for callback teardown.
extern crate mimi_lib;

use mimi_runtime::apple_speech::{
    AppleSpeechCapabilities, AppleSpeechError, AppleSpeechEvent, AppleSpeechLocale,
};

// Compile the maintained source, including its existing unit tests. Do not
// restore the retired desktop copy or duplicate the callback implementation.
#[allow(dead_code)]
#[path = "../../shared/mimi-runtime/src/apple_speech/native.rs"]
mod native;
