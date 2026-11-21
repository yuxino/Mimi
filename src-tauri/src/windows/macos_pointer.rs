//! Hover for the nonactivating subtitle and language panels.
//!
//! Each visible panel owns one AppKit tracking area and relays view-local
//! coordinates to its own renderer. InVisibleRect follows resizing; the
//! language panel's geometry transitions also reconcile the current hit
//! without waiting for another movement after a panel shrinks.

use crate::core::ui::overlay_pointer::{
    css_position, OverlayPointerPosition, PointerDelivery, PointerMotionGate,
};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{define_class, msg_send, AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSCursor, NSEvent, NSTrackingArea, NSTrackingAreaOptions, NSView, NSWindow};
use objc2_foundation::{NSObjectProtocol, NSPoint};
use std::{
    cell::{Cell, RefCell},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Instant,
};
use tauri::{AppHandle, Emitter, EventTarget, Manager, WebviewWindow};

const EVENT: &str = "overlay-pointer-motion";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Surface {
    Overlay,
    Control,
}

impl Surface {
    fn from_label(label: &str) -> Option<Self> {
        match label {
            "overlay" => Some(Self::Overlay),
            "overlay-control" => Some(Self::Control),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Overlay => "overlay",
            Self::Control => "overlay-control",
        }
    }
}

struct PointerOwnerIvars {
    app: AppHandle,
    surface: Surface,
    view: Retained<NSView>,
    inside: Cell<bool>,
    gate: RefCell<PointerMotionGate>,
    flush_scheduled: Cell<bool>,
    pointing_cursor: Cell<bool>,
    generation: Arc<AtomicU64>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements. Tracking callbacks,
    // view ownership, and registration/removal remain on AppKit's main thread.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MimiOverlayPointerOwner"]
    #[ivars = PointerOwnerIvars]
    struct MimiOverlayPointerOwner;

    impl MimiOverlayPointerOwner {
        #[unsafe(method(mouseEntered:))]
        fn entered(&self, event: &NSEvent) {
            if self.ivars().surface == Surface::Control {
                crate::language_probe::native(&self.ivars().app, "control-entered", self.ivars().surface.label(), serde_json::json!({"eventInWindow": [event.locationInWindow().x, event.locationInWindow().y], "eventTimestamp": event.timestamp(), "eventWindowNumber": event.windowNumber(), "insideBefore": self.ivars().inside.get()}));
            }
            self.clear(false);
            self.motion(event);
        }

        #[unsafe(method(mouseMoved:))]
        fn moved(&self, event: &NSEvent) {
            if self.ivars().surface == Surface::Control {
                crate::language_probe::native(&self.ivars().app, "control-moved", self.ivars().surface.label(), serde_json::json!({"eventInWindow": [event.locationInWindow().x, event.locationInWindow().y], "eventTimestamp": event.timestamp(), "eventWindowNumber": event.windowNumber(), "insideBefore": self.ivars().inside.get()}));
            }
            self.motion(event);
        }

        #[unsafe(method(mouseExited:))]
        fn exited(&self, _event: &NSEvent) {
            if self.ivars().surface == Surface::Control {
                crate::language_probe::native(&self.ivars().app, "control-exited", self.ivars().surface.label(), serde_json::json!({"eventTimestamp": _event.timestamp(), "eventWindowNumber": _event.windowNumber(), "insideBefore": self.ivars().inside.get()}));
            }
            self.clear(true);
        }
    }

    unsafe impl NSObjectProtocol for MimiOverlayPointerOwner {}
);

impl MimiOverlayPointerOwner {
    fn new(
        app: AppHandle,
        surface: Surface,
        view: Retained<NSView>,
        mtm: MainThreadMarker,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(PointerOwnerIvars {
            app,
            surface,
            view,
            inside: Cell::new(false),
            gate: RefCell::new(PointerMotionGate::default()),
            flush_scheduled: Cell::new(false),
            pointing_cursor: Cell::new(false),
            generation: Arc::new(AtomicU64::new(0)),
        });
        unsafe { msg_send![super(this), init] }
    }

    fn motion(&self, event: &NSEvent) {
        self.update_position(event.locationInWindow(), false);
    }

    fn update_position(&self, point_in_window: NSPoint, report_current: bool) {
        let ivars = self.ivars();
        let view = &ivars.view;
        // Tracking must never make a locked, hidden surface appear clickable.
        if !self.tracking_allowed() {
            self.clear(report_current);
            return;
        }
        let point = view.convertPoint_fromView(point_in_window, None);
        let bounds = view.bounds();
        let Some(point) = css_position(
            (point.x, point.y),
            (bounds.origin.x, bounds.origin.y),
            (bounds.size.width, bounds.size.height),
            view.isFlipped(),
        ) else {
            self.clear(report_current);
            return;
        };
        let was_inside = ivars.inside.replace(true);
        // Only the subtitle renderer hit-tests cursor positions. The child
        // reports presence transitions, so motion inside it adds no IPC.
        if ivars.surface == Surface::Control {
            if !was_inside || report_current {
                self.emit(Some(point));
            }
            return;
        }
        if ivars.pointing_cursor.get() {
            // WebKit/Tao may reset a cursor while processing the same move.
            // Reassert this owner's intent on real movement, never a timer.
            NSCursor::pointingHandCursor().set();
        }
        let delivery = ivars.gate.borrow_mut().motion(point, Instant::now());
        self.deliver(delivery);
    }

    fn tracking_allowed(&self) -> bool {
        let view = &self.ivars().view;
        !view.isHiddenOrHasHiddenAncestor()
            && view
                .window()
                .is_some_and(|window| window.isVisible() && !window.ignoresMouseEvents())
    }

    fn emit(&self, point: Option<OverlayPointerPosition>) {
        let ivars = self.ivars();
        let emitted = ivars.app.emit_to(
            EventTarget::webview_window(ivars.surface.label()),
            EVENT,
            point,
        );
        if ivars.surface == Surface::Control {
            crate::language_probe::native(&self.ivars().app, "control-emit", self.ivars().surface.label(), serde_json::json!({"inside": point.is_some(), "accepted": emitted.is_ok(), "viewHidden": ivars.view.isHiddenOrHasHiddenAncestor()}));
        }
    }

    fn deliver(&self, delivery: PointerDelivery) {
        let ivars = self.ivars();
        match delivery {
            PointerDelivery::Position(point) => self.emit(Some(point)),
            PointerDelivery::Delay(delay) if !ivars.flush_scheduled.replace(true) => {
                let app = ivars.app.clone();
                let generation = Arc::clone(&ivars.generation);
                let token = generation.load(Ordering::Relaxed);
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(delay).await;
                    if generation.load(Ordering::Relaxed) != token {
                        return;
                    }
                    let _ = app.run_on_main_thread(move || {
                        let Some(owner) = owner(Surface::Overlay) else {
                            return;
                        };
                        if !Arc::ptr_eq(&owner.ivars().generation, &generation)
                            || generation.load(Ordering::Relaxed) != token
                        {
                            return;
                        }
                        // Visibility/passthrough may have changed while
                        // this single delayed flush was waiting.
                        if !owner.tracking_allowed() {
                            owner.clear(false);
                            return;
                        }
                        owner.ivars().flush_scheduled.set(false);
                        let delivery = owner.ivars().gate.borrow_mut().flush(Instant::now());
                        owner.deliver(delivery);
                    });
                });
            }
            PointerDelivery::Delay(_) | PointerDelivery::None => {}
        }
    }

    fn clear(&self, report_outside: bool) {
        let was_inside = self.reset();
        if self.ivars().surface == Surface::Control {
            crate::language_probe::native(&self.ivars().app, "control-clear", self.ivars().surface.label(), serde_json::json!({"wasInside": was_inside, "reportOutside": report_outside}));
        }
        if was_inside || report_outside {
            self.emit(None);
        }
    }

    fn reset(&self) -> bool {
        self.restore_cursor();
        let ivars = self.ivars();
        ivars.generation.fetch_add(1, Ordering::Relaxed);
        ivars.flush_scheduled.set(false);
        ivars.gate.borrow_mut().clear();
        ivars.inside.replace(false)
    }

    fn set_pointer_cursor(&self, point: OverlayPointerPosition, pointing: bool) -> bool {
        if !self.tracking_allowed() {
            self.clear(false);
            return false;
        }
        if !self.ivars().gate.borrow().accepts_cursor_reply(point) {
            return false;
        }
        if pointing {
            self.ivars().pointing_cursor.set(true);
            // ActiveAlways does not receive cursorUpdate. Set the public
            // AppKit cursor directly, without making the panel key/active.
            NSCursor::pointingHandCursor().set();
        } else {
            self.restore_cursor();
        }
        true
    }

    fn restore_cursor(&self) {
        // Never change another application's cursor unless this owner set
        // the hand first. Leave, lock, hide, and drop all take this path.
        if self.ivars().pointing_cursor.replace(false) {
            NSCursor::arrowCursor().set();
        }
    }
}

struct PointerRegistration {
    view: Retained<NSView>,
    area: Retained<NSTrackingArea>,
    owner: Retained<MimiOverlayPointerOwner>,
}

impl Drop for PointerRegistration {
    fn drop(&mut self) {
        // AppKit does not retain the area's owner. Keep it alive until after
        // removing the area; replacing/destroying the overlay releases both.
        self.view.removeTrackingArea(&self.area);
        self.owner.reset();
    }
}

thread_local! {
    static REGISTRATIONS: RefCell<[Option<PointerRegistration>; 2]> = const { RefCell::new([None, None]) };
}

fn owner(surface: Surface) -> Option<Retained<MimiOverlayPointerOwner>> {
    REGISTRATIONS.with_borrow(|slots| {
        slots[surface as usize]
            .as_ref()
            .map(|registration| registration.owner.clone())
    })
}

pub(super) fn install(window: &WebviewWindow) {
    let Some(surface) = Surface::from_label(window.label()) else {
        return;
    };
    let app = window.app_handle().clone();
    crate::language_probe::native(&app, "install-request", surface.label(), serde_json::json!({}));
    let _ = window.with_webview(move |platform| {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        // Tauri supplies a live WKWebView (an NSView subclass) in this
        // main-thread callback. Retaining it makes registration cleanup safe.
        let Some(view) = (unsafe { Retained::retain(platform.inner().cast::<NSView>()) }) else {
            return;
        };
        crate::language_probe::native(&app, "install-view", surface.label(), serde_json::json!({
            "viewHasWindow": view.window().is_some(), "viewWindowNumber": view.window().map(|w| w.windowNumber()), "viewHidden": view.isHiddenOrHasHiddenAncestor(),
            "ownerPresent": owner(surface).is_some()
        }));
        if !view.window().is_some_and(|window| window.isVisible()) {
            crate::language_probe::native(&app, "install-not-visible", surface.label(), serde_json::json!({}));
            return;
        }
        if owner(surface)
            .is_some_and(|owner| Retained::as_ptr(&owner.ivars().view) == Retained::as_ptr(&view))
        {
            crate::language_probe::native(&app, "install-same-owner", surface.label(), serde_json::json!({}));
            return;
        }
        drop(take_registration(surface));
        let owner = MimiOverlayPointerOwner::new(app.clone(), surface, view.clone(), mtm);
        let owner_object: &AnyObject = owner.as_ref();
        let options = NSTrackingAreaOptions::ActiveAlways
            | NSTrackingAreaOptions::MouseEnteredAndExited
            | NSTrackingAreaOptions::MouseMoved
            | NSTrackingAreaOptions::InVisibleRect;
        let area = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(),
                view.bounds(),
                options,
                Some(owner_object),
                None,
            )
        };
        view.addTrackingArea(&area);
        REGISTRATIONS.with_borrow_mut(|slots| {
            slots[surface as usize] = Some(PointerRegistration { view, area, owner });
        });
        crate::language_probe::native(&app, "install-success", surface.label(), serde_json::json!({}));
    });
}

fn on_main(app: &AppHandle, operation: impl FnOnce(MainThreadMarker) + Send + 'static) {
    if let Some(mtm) = MainThreadMarker::new() {
        operation(mtm);
        return;
    }
    let _ = app.run_on_main_thread(move || {
        if let Some(mtm) = MainThreadMarker::new() {
            operation(mtm);
        }
    });
}

pub(super) fn remove(app: &AppHandle, label: &str) {
    if let Some(surface) = Surface::from_label(label) {
        on_main(app, move |_| remove_on_main(surface));
    }
}

fn take_registration(surface: Surface) -> Option<PointerRegistration> {
    REGISTRATIONS.with_borrow_mut(|slots| slots[surface as usize].take())
}

fn remove_on_main(surface: Surface) {
    if let Some(registration) = take_registration(surface) {
        registration.owner.clear(true);
    }
}

pub(super) fn remove_all(app: &AppHandle) {
    on_main(app, |_| {
        remove_on_main(Surface::Overlay);
        remove_on_main(Surface::Control);
    });
}

pub(super) fn clear_overlay(app: &AppHandle) {
    on_main(app, |_| {
        if let Some(owner) = owner(Surface::Overlay) {
            owner.clear(true);
        }
    });
}

pub(super) fn refresh_after_window_update(app: &AppHandle) {
    // Tauri first delivers window changes to its dispatcher; Tao then queues
    // the AppKit frame setters. Sample on the same queue after those setters
    // and after any enclosing with_webview callback has returned.
    on_main(app, |_| {
        dispatch2::DispatchQueue::main().exec_async(|| {
            if let Some(mtm) = MainThreadMarker::new() {
                refresh_on_main(mtm);
            }
        });
    });
}

fn refresh_on_main(mtm: MainThreadMarker) {
    if let Some(owner) = owner(Surface::Control) {
        crate::language_probe::native(&owner.ivars().app, "native-refresh", "overlay-control", serde_json::json!({
            "ownerInside": owner.ivars().inside.get()
        }));
    }
    let screen_point = NSEvent::mouseLocation();
    let frontmost = NSWindow::windowNumberAtPoint_belowWindowWithWindowNumber(screen_point, 0, mtm);
    for surface in [Surface::Overlay, Surface::Control] {
        let Some(owner) = owner(surface) else {
            continue;
        };
        let window = owner
            .ivars()
            .view
            .window()
            .filter(|window| window.windowNumber() == frontmost);
        if let Some(window) = window {
            owner.update_position(window.convertPointFromScreen(screen_point), true);
        } else {
            // A stationary pointer may now be over another window after a
            // resize. Report outside even if only the DOM observed entry.
            owner.clear(true);
        }
    }
}

pub(super) async fn set_pointer_cursor(
    app: &AppHandle,
    point: OverlayPointerPosition,
    pointing: bool,
) -> Result<bool, String> {
    if MainThreadMarker::new().is_some() {
        return Ok(set_pointer_cursor_on_main(point, pointing));
    }
    let (reply, accepted) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let _ = reply.send(set_pointer_cursor_on_main(point, pointing));
    })
    .map_err(|_| "overlay_pointer_cursor_unavailable".to_string())?;
    accepted
        .await
        .map_err(|_| "overlay_pointer_cursor_unavailable".to_string())
}

fn set_pointer_cursor_on_main(point: OverlayPointerPosition, pointing: bool) -> bool {
    owner(Surface::Overlay).is_some_and(|owner| owner.set_pointer_cursor(point, pointing))
}
