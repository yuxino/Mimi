//! Ordinary AppKit termination must use Mimi's asynchronous save boundary.
//!
//! The delegate is weak on NSApplication. Keep both this wrapper and Tao's
//! original delegate alive on the main thread, and forward callbacks with the
//! original receiver so its private state is never read from our wrapper.

use crate::commands::{self, AppState, SettingsNavigationTarget};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, ProtocolObject, Sel};
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSApplication, NSApplicationDelegate, NSApplicationTerminateReply};
use objc2_foundation::NSObjectProtocol;
use std::{cell::RefCell, sync::Arc};
use tauri::{AppHandle, Manager};

struct QuitDelegateIvars {
    app: AppHandle,
    original: Retained<ProtocolObject<dyn NSApplicationDelegate>>,
}

fn responds_to(original: &AnyObject, selector: Sel, superclass_responds: bool) -> bool {
    // SAFETY: All Objective-C objects implement NSObject's selector query.
    superclass_responds || unsafe { msg_send![original, respondsToSelector: selector] }
}

fn forwarding_target(original: &AnyObject, selector: Sel) -> Option<Retained<AnyObject>> {
    if responds_to(original, selector, false) {
        // SAFETY: The registration holds a strong reference to this object;
        // retaining it for the forwarding return preserves that exact receiver.
        unsafe { Retained::retain((original as *const AnyObject).cast_mut()) }
    } else {
        None
    }
}

fn owns_delegate(current: Option<&AnyObject>, owner: &AnyObject) -> bool {
    current.is_some_and(|current| std::ptr::eq(current, owner))
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements. NSApplication delegate
    // callbacks, installation and restoration are confined to the main thread.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MimiNormalQuitDelegate"]
    #[ivars = QuitDelegateIvars]
    struct MimiNormalQuitDelegate;

    impl MimiNormalQuitDelegate {
        #[unsafe(method(respondsToSelector:))]
        fn responds(&self, selector: Sel) -> bool {
            // SAFETY: This is NSObject's public selector query, with its exact
            // signature. Query the superclass without recursing into ourselves.
            let superclass_responds = unsafe {
                msg_send![super(self), respondsToSelector: selector]
            };
            responds_to(self.ivars().original.as_ref(), selector, superclass_responds)
        }

        #[unsafe(method_id(forwardingTargetForSelector:))]
        fn forward(&self, selector: Sel) -> Option<Retained<AnyObject>> {
            forwarding_target(self.ivars().original.as_ref(), selector)
        }
    }

    unsafe impl NSObjectProtocol for MimiNormalQuitDelegate {}

    unsafe impl NSApplicationDelegate for MimiNormalQuitDelegate {
        #[unsafe(method(applicationShouldTerminate:))]
        fn should_terminate(&self, _sender: &NSApplication) -> NSApplicationTerminateReply {
            let app = self.ivars().app.clone();
            if let Some(state) = app.try_state::<AppState>() {
                let session = Arc::clone(&state.session);
                tauri::async_runtime::spawn(async move {
                    if commands::quit_application(app.clone(), session).await.is_err() {
                        tracing::warn!("native quit failed label=history_save_failed");
                        // Keep the app and pending archive alive. The existing
                        // Export page exposes historySaveError and retry/export.
                        let _ = commands::app_show_settings(
                            app,
                            Some(SettingsNavigationTarget::Export),
                        );
                    }
                });
            } else {
                tracing::warn!("native quit rejected label=app_state_unavailable");
            }
            // Cancel AppKit's direct termination. The shared command calls
            // Tauri's exit only after stop/finalization succeeds; no Later
            // reply/run-loop ordering or early applicationWillTerminate occurs.
            NSApplicationTerminateReply::TerminateCancel
        }
    }
);

impl MimiNormalQuitDelegate {
    fn new(
        app: AppHandle,
        original: Retained<ProtocolObject<dyn NSApplicationDelegate>>,
        main: MainThreadMarker,
    ) -> Retained<Self> {
        let this = Self::alloc(main).set_ivars(QuitDelegateIvars { app, original });
        // SAFETY: NSObject's init has the declared signature.
        unsafe { msg_send![super(this), init] }
    }
}

struct QuitDelegateRegistration {
    application: Retained<NSApplication>,
    owner: Retained<MimiNormalQuitDelegate>,
}

impl Drop for QuitDelegateRegistration {
    fn drop(&mut self) {
        let current = self.application.delegate();
        if owns_delegate(current.as_deref().map(AsRef::as_ref), self.owner.as_ref()) {
            // A later plugin's delegate must never be overwritten on teardown.
            self.application
                .setDelegate(Some(&self.owner.ivars().original));
        }
    }
}

thread_local! {
    static REGISTRATION: RefCell<Option<QuitDelegateRegistration>> = const { RefCell::new(None) };
}

pub(crate) fn install(app: &AppHandle) -> Result<(), &'static str> {
    let main = MainThreadMarker::new().ok_or("native_quit_main_thread_unavailable")?;
    REGISTRATION.with(|slot| {
        if slot.borrow().is_some() {
            return Ok(());
        }
        let application = NSApplication::sharedApplication(main);
        let original = application
            .delegate()
            .ok_or("native_quit_delegate_unavailable")?;
        let owner = MimiNormalQuitDelegate::new(app.clone(), original, main);
        application.setDelegate(Some(ProtocolObject::from_ref(&*owner)));
        *slot.borrow_mut() = Some(QuitDelegateRegistration { application, owner });
        Ok(())
    })
}

pub(crate) fn remove() {
    // RunEvent::Exit is delivered by the AppKit event loop on its main thread.
    if MainThreadMarker::new().is_some() {
        REGISTRATION.with(|slot| {
            let registration = slot.borrow_mut().take();
            drop(registration);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2::{AnyThread, ClassType};
    use std::cell::Cell;

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "MimiQuitForwardingTestOriginal"]
        #[ivars = Cell<usize>]
        struct Original;

        impl Original {
            #[unsafe(method(mimiOriginalCallback:))]
            fn callback(&self, value: usize) -> usize {
                self.ivars().set(self.ivars().get() + value);
                self.ivars().get()
            }
        }
        unsafe impl NSObjectProtocol for Original {}
    );

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "MimiQuitForwardingTestWrapper"]
        #[ivars = Retained<Original>]
        struct Forwarder;

        impl Forwarder {
            #[unsafe(method(respondsToSelector:))]
            fn responds(&self, selector: Sel) -> bool {
                let superclass = unsafe { msg_send![super(self), respondsToSelector: selector] };
                responds_to(self.ivars().as_ref(), selector, superclass)
            }
            #[unsafe(method_id(forwardingTargetForSelector:))]
            fn forward(&self, selector: Sel) -> Option<Retained<AnyObject>> {
                forwarding_target(self.ivars().as_ref(), selector)
            }
        }
        unsafe impl NSObjectProtocol for Forwarder {}
    );

    fn original() -> Retained<Original> {
        let this = Original::alloc().set_ivars(Cell::new(0));
        unsafe { msg_send![super(this), init] }
    }

    #[test]
    fn forwarded_optional_callback_uses_original_receiver_and_retains_it() {
        let original = original();
        let this = Forwarder::alloc().set_ivars(original.clone());
        let wrapper: Retained<Forwarder> = unsafe { msg_send![super(this), init] };
        drop(original);
        assert!(wrapper.respondsToSelector(objc2::sel!(mimiOriginalCallback:)));
        // The wrapper has a different ivar layout. Actual dynamic forwarding
        // must send the callback to Original, never read that layout from self.
        let first: usize = unsafe { msg_send![&wrapper, mimiOriginalCallback: 7usize] };
        let second: usize = unsafe { msg_send![&wrapper, mimiOriginalCallback: 2usize] };
        assert_eq!((first, second), (7, 9));
        assert!(wrapper.respondsToSelector(objc2::sel!(description)));
        assert_eq!(
            MimiNormalQuitDelegate::class().name().to_bytes(),
            b"MimiNormalQuitDelegate"
        );
    }

    #[test]
    fn unknown_callbacks_are_not_advertised_or_forwarded() {
        let original = original();
        let selector = objc2::sel!(mimiUnknownCallback);
        assert!(!responds_to(original.as_ref(), selector, false));
        assert!(forwarding_target(original.as_ref(), selector).is_none());
    }

    #[test]
    fn restoration_requires_exact_owner_identity() {
        let owner = original();
        let later_delegate = original();
        assert!(owns_delegate(Some(owner.as_ref()), owner.as_ref()));
        assert!(!owns_delegate(
            Some(later_delegate.as_ref()),
            owner.as_ref()
        ));
        assert!(!owns_delegate(None, owner.as_ref()));
    }

    #[test]
    fn taos_delegate_ivar_dock_path_is_not_exposed_by_mimi() {
        // Tao's runtime Dock visibility API reads auxState from NSApp.delegate.
        // Mimi uses direct AppKit activation policy instead. Do not expose the
        // unsafe-for-a-wrapper Tao route through Rust or a renderer capability.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let forbidden_call = format!(".{}(", "set_dock_visibility");
        fn check_sources(path: &std::path::Path, forbidden: &str) {
            for entry in std::fs::read_dir(path).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    check_sources(&path, forbidden);
                } else if path.extension().is_some_and(|extension| extension == "rs") {
                    assert!(!std::fs::read_to_string(path).unwrap().contains(forbidden));
                }
            }
        }
        check_sources(&root.join("src"), &forbidden_call);
        for entry in std::fs::read_dir(root.join("capabilities")).unwrap() {
            let path = entry.unwrap().path();
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                let capability: serde_json::Value =
                    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
                for permission in capability["permissions"].as_array().unwrap() {
                    let identifier = permission
                        .as_str()
                        .or_else(|| permission["identifier"].as_str())
                        .unwrap();
                    assert!(!matches!(
                        identifier,
                        "core:app:allow-set-dock-visibility" | "core:app:*" | "core:*" | "*"
                    ));
                }
            }
        }
    }
}
