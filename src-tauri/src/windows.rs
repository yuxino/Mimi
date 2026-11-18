//! Native window entry points and platform-driver registration.
//! Overlay geometry, transient controls, OS presentation, monitor coordinates
//! and the settings window keep their implementation with their resource owner.

#[cfg(target_os = "macos")]
use crate::settings_store::SettingsStore;
#[cfg(target_os = "macos")]
use std::sync::Arc;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use tauri::AppHandle;

mod controls;
mod native;
mod overlay;
mod settings_window;
mod work_area;

pub use controls::{
    OverlayControlGeometry, OverlayControlHost, OverlayControlMode, OverlayControlState,
    OverlayControlWindowManager,
};
pub use native::OverlayPresentationState;
pub use overlay::{OverlayState, OverlayWindowManager};
pub use settings_window::ensure_settings_window;

#[cfg(test)]
mod test_support;

/// True when this is a dev build: a debug build (`tauri::is_dev`) or the
/// bundled dev wrapper (bundle id `app.yuxino.mimi.dev`, which is a release
/// build so macOS renders its Dock icon with the standard mask).
pub fn is_dev_build() -> bool {
    #[cfg(target_os = "macos")]
    {
        use objc2::msg_send;
        use objc2::runtime::{AnyClass, AnyObject};
        use std::ffi::{c_char, CStr};
        if tauri::is_dev() {
            return true;
        }
        unsafe {
            let Some(bundle_class) = AnyClass::get(c"NSBundle") else {
                return false;
            };
            let main_bundle: *mut AnyObject = msg_send![bundle_class, mainBundle];
            if main_bundle.is_null() {
                return false;
            }
            let identifier: *mut AnyObject = msg_send![main_bundle, bundleIdentifier];
            if identifier.is_null() {
                return false;
            }
            let ptr: *const c_char = msg_send![identifier, UTF8String];
            if ptr.is_null() {
                return false;
            }
            CStr::from_ptr(ptr).to_str() == Ok("app.yuxino.mimi.dev")
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        tauri::is_dev()
    }
}

/// Window and tooltip title for the current build: dev builds get a "(dev)"
/// marker so they can be told apart from the installed release app at a
/// glance.
pub fn dev_title(base: &str) -> String {
    if is_dev_build() {
        format!("{base} (dev)")
    } else {
        base.to_string()
    }
}

#[cfg(target_os = "linux")]
mod linux_input_region;

#[cfg(target_os = "macos")]
mod macos_control_dismiss;

#[cfg(target_os = "macos")]
mod macos_pointer;

#[cfg(any(target_os = "windows", test))]
mod windows_pointer;

#[cfg(target_os = "windows")]
pub use windows_pointer::WindowsPointerTracker;

#[cfg(target_os = "windows")]
pub fn install_windows_pointer_tracking(app: &AppHandle) -> WindowsPointerTracker {
    windows_pointer::install(app)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn remove_overlay_pointer_tracking(app: &AppHandle, label: &str) {
    #[cfg(target_os = "macos")]
    macos_pointer::remove(app, label);
    #[cfg(target_os = "windows")]
    windows_pointer::remove(app, label);
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn remove_all_overlay_pointer_tracking(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    macos_pointer::remove_all(app);
    #[cfg(target_os = "windows")]
    windows_pointer::remove_all(app);
}

#[cfg(target_os = "macos")]
pub async fn set_overlay_pointer_cursor(
    app: &AppHandle,
    point: crate::core::ui::overlay_pointer::OverlayPointerPosition,
    pointing: bool,
) -> Result<bool, String> {
    macos_pointer::set_pointer_cursor(app, point, pointing).await
}

#[cfg(target_os = "windows")]
mod windows_workspace;

#[cfg(target_os = "windows")]
pub use windows_workspace::WindowsWorkspaceFollower;

#[cfg(target_os = "windows")]
pub fn install_windows_workspace_follower(app: &AppHandle) -> WindowsWorkspaceFollower {
    windows_workspace::install(app)
}

#[cfg(target_os = "macos")]
mod macos_space;

pub mod resize;

#[cfg(target_os = "macos")]
pub fn install_active_space_observer(
    app: &AppHandle,
    overlay: &Arc<std::sync::Mutex<OverlayState>>,
    settings: &Arc<SettingsStore>,
) {
    macos_space::install(app, overlay, settings);
    // Seed the runtime presentation frame as well as observing later changes;
    // Mimi may start while another application is already full-screen.
    OverlayWindowManager::follow_active_space(app, overlay, settings);
}
