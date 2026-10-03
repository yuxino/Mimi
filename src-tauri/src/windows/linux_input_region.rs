//! GTK input regions shared by native X11 and Wayland overlays.
//!
//! Tao 0.35.3's ignore-cursor path uses a 1×1 rectangle. An entirely empty
//! Cairo region is required to pass every point through. Use the GTK widget
//! API so the application shape composes with GTK's own input shape. GTK
//! clears that shape on unrealize, so lifecycle callbacks restore the latest
//! intent on realize and map; this is not a compositor-specific workaround.

use gtk::prelude::*;

#[derive(Debug, Default)]
pub(super) struct InputRegionState {
    requested: Option<bool>,
    applied: Option<bool>,
}

impl InputRegionState {
    pub(super) fn request(&mut self, enabled: bool) {
        self.requested = Some(enabled);
    }

    pub(super) fn invalidate(&mut self) {
        // A recreated surface must inherit the latest user intent, including
        // an unlock requested while it was hidden or awaiting dispatch.
        self.applied = None;
    }

    pub(super) fn reconcile(&mut self, force: bool, apply: impl FnOnce(bool)) {
        let Some(requested) = self.requested else {
            return;
        };
        if force || self.applied != Some(requested) {
            apply(requested);
            self.applied = Some(requested);
        }
    }
}

fn input_region(enabled: bool) -> Option<gtk::cairo::Region> {
    enabled.then(gtk::cairo::Region::create)
}

pub(super) fn apply(window: &impl IsA<gtk::Window>, enabled: bool) {
    let window: &gtk::Window = window.as_ref();
    // None restores GTK's default input shape (including any CSD constraints).
    // An empty region is different from None: it accepts no pointer input.
    window.input_shape_combine_region(input_region(enabled).as_ref());
}

pub(super) fn restore_on_surface_change(
    window: &impl IsA<gtk::Window>,
    restore: impl Fn(&gtk::Window) + 'static,
) {
    let window: &gtk::Window = window.as_ref();
    let restore = std::rc::Rc::new(restore);
    let on_realize = std::rc::Rc::clone(&restore);
    window.connect_realize(move |window| on_realize(window));
    window.connect_map_event(move |window, _| {
        restore(window);
        gtk::glib::Propagation::Proceed
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locked_region_is_fully_empty_including_the_origin() {
        let region = input_region(true).unwrap();
        assert!(region.is_empty());
        assert_eq!(region.num_rectangles(), 0);
        assert!(!region.contains_point(0, 0));
        assert!(!region.contains_point(1, 1));
        assert!(!region.contains_point(500, 100));
    }

    #[test]
    fn unlocked_region_restores_default_input_instead_of_an_empty_shape() {
        assert!(input_region(false).is_none());
    }

    #[test]
    fn unchanged_snapshots_do_not_reapply_native_state() {
        let mut state = InputRegionState::default();
        let mut applied = Vec::new();
        state.request(true);
        state.reconcile(false, |value| applied.push(value));
        state.request(true);
        state.reconcile(false, |value| applied.push(value));
        assert_eq!(applied, [true]);
    }

    #[test]
    fn queued_lock_cannot_overwrite_a_newer_unlock() {
        let mut state = InputRegionState::default();
        let mut applied = Vec::new();
        state.request(true);
        state.request(false);
        // Both queued main-loop callbacks consult the latest intent.
        state.reconcile(false, |value| applied.push(value));
        state.reconcile(false, |value| applied.push(value));
        assert_eq!(applied, [false]);
    }

    #[test]
    fn remap_reapplies_the_latest_lock_or_unlock() {
        let mut state = InputRegionState::default();
        let mut applied = Vec::new();
        state.request(true);
        state.reconcile(false, |value| applied.push(value));
        state.reconcile(true, |value| applied.push(value));
        state.request(false);
        state.reconcile(true, |value| applied.push(value));
        assert_eq!(applied, [true, true, false]);
    }

    #[test]
    fn recreate_preserves_intent_but_invalidates_native_cache() {
        let mut state = InputRegionState::default();
        let mut applied = Vec::new();
        state.request(true);
        state.reconcile(false, |value| applied.push(value));
        state.invalidate();
        state.reconcile(false, |value| applied.push(value));
        state.request(false);
        state.invalidate();
        state.reconcile(false, |value| applied.push(value));
        assert_eq!(applied, [true, true, false]);
    }

    // Runs only inside scripts/linux-wayland-input-smoke.sh. This checks the
    // same GTK helper and lifecycle hooks as Mimi, not WebKit/provider state.
    #[test]
    #[ignore = "requires an explicitly isolated native Wayland display"]
    fn native_input_region_lifecycle() {
        use std::cell::{Cell, RefCell};
        use std::rc::Rc;
        use std::time::Duration;

        assert_eq!(std::env::var("MIMI_NATIVE_INPUT_SMOKE").as_deref(), Ok("1"));
        assert_eq!(std::env::var("GDK_BACKEND").as_deref(), Ok("wayland"));
        assert!(std::env::var_os("DISPLAY").is_none());
        gtk::init().expect("the isolated Wayland display must be available");
        let display = gtk::gdk::Display::default().expect("GDK display");
        assert_eq!(display.type_().name(), "GdkWaylandDisplay");

        fn drain() {
            for _ in 0..20 {
                while gtk::events_pending() {
                    gtk::main_iteration_do(false);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        let state = Rc::new(RefCell::new(InputRegionState::default()));
        let restores = Rc::new(Cell::new(0));
        let make_window = || {
            let window = gtk::Window::new(gtk::WindowType::Toplevel);
            window.set_title("Mimi isolated input-region fixture");
            window.set_decorated(false);
            window.set_default_size(100, 80);
            let state = Rc::clone(&state);
            let restores = Rc::clone(&restores);
            restore_on_surface_change(&window, move |window| {
                restores.set(restores.get() + 1);
                state
                    .borrow_mut()
                    .reconcile(true, |enabled| apply(window, enabled));
            });
            window
        };
        let window = make_window();
        state.borrow_mut().request(true);
        eprintln!("MIMI_INPUT_PHASE locked-before-map");
        window.show_all();
        drain();
        assert!(window.is_mapped());
        assert!(restores.get() >= 2);
        eprintln!("MIMI_INPUT_PHASE_END locked-before-map");

        eprintln!("MIMI_INPUT_PHASE unlocked");
        state.borrow_mut().request(false);
        state
            .borrow_mut()
            .reconcile(false, |enabled| apply(&window, enabled));
        window.queue_draw();
        drain();
        eprintln!("MIMI_INPUT_PHASE_END unlocked");

        eprintln!("MIMI_INPUT_PHASE relocked");
        state.borrow_mut().request(true);
        state
            .borrow_mut()
            .reconcile(false, |enabled| apply(&window, enabled));
        window.queue_draw();
        drain();
        eprintln!("MIMI_INPUT_PHASE_END relocked");

        window.hide();
        drain();
        let before = restores.get();
        eprintln!("MIMI_INPUT_PHASE hidden-unlock-remap");
        state.borrow_mut().request(false);
        // No apply call: the real map hook must consume the hidden request.
        window.show_all();
        drain();
        assert!(restores.get() > before);
        eprintln!("MIMI_INPUT_PHASE_END hidden-unlock-remap");

        window.hide();
        window.unrealize();
        drain();
        let before = restores.get();
        eprintln!("MIMI_INPUT_PHASE unrealize-relock-realize");
        state.borrow_mut().request(true);
        window.show_all();
        drain();
        assert!(restores.get() >= before + 2);
        eprintln!("MIMI_INPUT_PHASE_END unrealize-relock-realize");

        window.hide();
        drain();
        eprintln!("MIMI_INPUT_PHASE unchanged-lock-remap");
        // No request/invalidate/apply: a true cache must not suppress map.
        window.show_all();
        drain();
        eprintln!("MIMI_INPUT_PHASE_END unchanged-lock-remap");

        window.hide();
        window.unrealize();
        drain();
        eprintln!("MIMI_INPUT_PHASE unchanged-lock-unrealize");
        // GTK cleared the widget shape, but requested/applied both remain true.
        // Only the forced lifecycle restoration can restore the empty shape.
        window.show_all();
        drain();
        eprintln!("MIMI_INPUT_PHASE_END unchanged-lock-unrealize");

        window.close();
        drain();
        state.borrow_mut().invalidate();
        let recreated = make_window();
        eprintln!("MIMI_INPUT_PHASE recreated-locked");
        recreated.show_all();
        drain();
        assert!(recreated.is_mapped());
        eprintln!("MIMI_INPUT_PHASE_END recreated-locked");

        eprintln!("MIMI_INPUT_PHASE recreated-unlocked");
        state.borrow_mut().request(false);
        state
            .borrow_mut()
            .reconcile(false, |enabled| apply(&recreated, enabled));
        recreated.queue_draw();
        drain();
        eprintln!("MIMI_INPUT_PHASE_END recreated-unlocked");
        eprintln!("MIMI_INPUT_PHASE complete");
        recreated.close();
        drain();
    }

    #[test]
    fn absent_intent_does_not_change_a_window() {
        let mut state = InputRegionState::default();
        state.reconcile(true, |_| panic!("there is no requested input state"));
    }
}
