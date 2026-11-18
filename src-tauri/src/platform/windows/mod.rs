//! Windows instance ownership and native updater cleanup.

#[cfg(target_os = "windows")]
pub(crate) mod startup;
pub(crate) mod updater_cleanup;
