//! Public-AppKit observation and screen lookup for macOS Space transitions.

use super::overlay::{OverlayState, OverlayWindowManager};
use super::work_area::LogicalWorkArea;
use crate::settings_store::SettingsStore;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{define_class, msg_send, sel, AnyThread, DefinedClass, MainThreadMarker};
use objc2_app_kit::{
    NSApplication, NSScreen, NSWorkspace, NSWorkspaceActiveSpaceDidChangeNotification,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSObjectProtocol};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager};

const SPACE_TRANSITION_SETTLE_MS: u64 = 250;

struct ActiveSpaceObserverIvars {
    app: AppHandle,
    overlay: Arc<Mutex<OverlayState>>,
    settings: Arc<SettingsStore>,
    generation: Arc<AtomicU64>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements. The callback touches
    // AppKit only after OverlayWindowManager dispatches to the main thread.
    #[unsafe(super(NSObject))]
    #[name = "MimiActiveSpaceObserver"]
    #[ivars = ActiveSpaceObserverIvars]
    struct MimiActiveSpaceObserver;

    impl MimiActiveSpaceObserver {
        #[unsafe(method(mimiActiveSpaceDidChange:))]
        fn active_space_did_change(&self, _notification: &NSNotification) {
            let generation = self
                .ivars()
                .generation
                .fetch_add(1, Ordering::Relaxed)
                .wrapping_add(1);
            let generation_state = Arc::clone(&self.ivars().generation);
            let app = self.ivars().app.clone();
            let overlay = Arc::clone(&self.ivars().overlay);
            let settings = Arc::clone(&self.ivars().settings);
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(
                    SPACE_TRANSITION_SETTLE_MS,
                ))
                .await;
                if !is_current_generation(&generation_state, generation) {
                    return;
                }
                let app_for_main = app.clone();
                let generation_for_main = Arc::clone(&generation_state);
                let _ = app.run_on_main_thread(move || {
                    if !is_current_generation(&generation_for_main, generation) {
                        return;
                    }
                    OverlayWindowManager::follow_active_space_on_main(
                        &app_for_main,
                        &overlay,
                        &settings,
                    );
                });
            });
        }
    }

    unsafe impl NSObjectProtocol for MimiActiveSpaceObserver {}
);

impl MimiActiveSpaceObserver {
    fn new(
        app: AppHandle,
        overlay: Arc<Mutex<OverlayState>>,
        settings: Arc<SettingsStore>,
    ) -> Retained<Self> {
        let this = Self::alloc().set_ivars(ActiveSpaceObserverIvars {
            app,
            overlay,
            settings,
            generation: Arc::new(AtomicU64::new(0)),
        });
        unsafe { msg_send![super(this), init] }
    }
}

fn is_current_generation(state: &AtomicU64, candidate: u64) -> bool {
    state.load(Ordering::Relaxed) == candidate
}

struct ActiveSpaceObserverRegistration {
    center: Retained<NSNotificationCenter>,
    observer: Retained<MimiActiveSpaceObserver>,
}

impl Drop for ActiveSpaceObserverRegistration {
    fn drop(&mut self) {
        let observer: &AnyObject = self.observer.as_ref();
        unsafe { self.center.removeObserver(observer) };
    }
}

pub(super) fn install(
    app: &AppHandle,
    overlay: &Arc<Mutex<OverlayState>>,
    settings: &Arc<SettingsStore>,
) {
    let workspace = NSWorkspace::sharedWorkspace();
    let center = workspace.notificationCenter();
    let observer =
        MimiActiveSpaceObserver::new(app.clone(), Arc::clone(overlay), Arc::clone(settings));
    let observer_object: &AnyObject = observer.as_ref();
    let notification_name = unsafe { NSWorkspaceActiveSpaceDidChangeNotification };
    unsafe {
        center.addObserver_selector_name_object(
            observer_object,
            sel!(mimiActiveSpaceDidChange:),
            Some(notification_name),
            None,
        );
    }

    if !app.manage(ActiveSpaceObserverRegistration { center, observer }) {
        tracing::warn!("active Space observer unavailable label=already_registered");
    }
}

/// Converts AppKit's bottom-left global screen coordinates to the top-left
/// logical coordinate space used by Tao/Tauri window positions.
pub(super) fn main_screen_work_area() -> Option<LogicalWorkArea> {
    let mtm = MainThreadMarker::new()?;
    // NSScreen.main follows the key window. Settings and tray interaction can
    // temporarily make Mimi itself key; that is not evidence that the user's
    // media/full-screen target changed displays.
    if NSApplication::sharedApplication(mtm).keyWindow().is_some() {
        return None;
    }
    let workspace = NSWorkspace::sharedWorkspace();
    if workspace
        .frontmostApplication()
        .is_some_and(|application| application.processIdentifier() == std::process::id() as i32)
    {
        return None;
    }
    let main = NSScreen::mainScreen(mtm)?;
    let primary = NSScreen::screens(mtm).firstObject()?;
    let primary_top = primary.frame().origin.y + primary.frame().size.height;
    let visible = main.visibleFrame();
    Some(LogicalWorkArea {
        x: visible.origin.x,
        y: primary_top - visible.origin.y - visible.size.height,
        width: visible.size.width,
        height: visible.size.height,
        coordinate_scale: 1.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_space_generation_supersedes_an_older_delayed_callback() {
        let generation = AtomicU64::new(4);
        assert!(is_current_generation(&generation, 4));

        generation.store(5, Ordering::Relaxed);

        assert!(!is_current_generation(&generation, 4));
        assert!(is_current_generation(&generation, 5));
    }
}
