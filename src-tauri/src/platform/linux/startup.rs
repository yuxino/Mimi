//! Native threading before Tauri/GTK creates any display connection.

use std::sync::OnceLock;
use x11_dl::xlib::Xlib;

fn should_skip_x11_init_for_wayland(
    wayland_display: Option<&str>,
    x_display: Option<&str>,
    gdk_backend: Option<&str>,
) -> bool {
    let wayland_backend = gdk_backend.is_some_and(|value| {
        value
            .split(',')
            .map(str::trim)
            .find(|backend| !backend.is_empty())
            .is_some_and(|first| first.eq_ignore_ascii_case("wayland"))
    });
    let has_wayland = wayland_display.is_some_and(|value| !value.is_empty());
    let has_x11 = x_display.is_some_and(|value| !value.is_empty());
    wayland_backend || (has_wayland && !has_x11)
}

fn initialize_x11_threads_with(
    open_xlib: impl FnOnce() -> Result<(), &'static str>,
    pure_wayland: bool,
) -> Result<(), &'static str> {
    match open_xlib() {
        Ok(()) => Ok(()),
        Err("x11.library_unavailable") if pure_wayland => {
            eprintln!("mimi: skipping X11 thread initialization on pure Wayland without libX11");
            Ok(())
        }
        Err(error) => Err(error),
    }
}

pub fn initialize_x11_threads() -> Result<(), &'static str> {
    // Keep the library loaded for the process lifetime. x11-dl is already used
    // by Tao; opening it here does not create a display or force an X11 backend.
    static XLIB: OnceLock<Result<Xlib, &'static str>> = OnceLock::new();
    let pure_wayland = should_skip_x11_init_for_wayland(
        std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
        std::env::var("DISPLAY").ok().as_deref(),
        std::env::var("GDK_BACKEND").ok().as_deref(),
    );
    initialize_x11_threads_with(
        || {
            let xlib = XLIB
                .get_or_init(|| Xlib::open().map_err(|_| "x11.library_unavailable"))
                .as_ref()
                .map_err(|label| *label)?;
            // SAFETY: run() calls this before Tauri, GTK, or any worker is initialized.
            // XInitThreads must precede every other Xlib call, including display opens.
            if unsafe { (xlib.XInitThreads)() } == 0 {
                return Err("x11.thread_initialization_failed");
            }
            Ok(())
        },
        pure_wayland,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_wayland_skips_missing_xlib_without_weakening_x11_failures() {
        assert!(should_skip_x11_init_for_wayland(
            Some("wayland-0"),
            None,
            None
        ));
        assert!(should_skip_x11_init_for_wayland(
            Some("wayland-0"),
            Some(""),
            None
        ));
        assert!(should_skip_x11_init_for_wayland(
            None,
            Some(":0"),
            Some("wayland")
        ));
        assert!(!should_skip_x11_init_for_wayland(
            Some("wayland-0"),
            Some(":0"),
            None
        ));
        assert!(!should_skip_x11_init_for_wayland(
            None,
            Some(":0"),
            Some("x11")
        ));

        assert_eq!(
            initialize_x11_threads_with(|| Err("x11.library_unavailable"), true),
            Ok(())
        );
        assert_eq!(
            initialize_x11_threads_with(|| Err("x11.library_unavailable"), false),
            Err("x11.library_unavailable")
        );
        assert_eq!(
            initialize_x11_threads_with(|| Err("x11.thread_initialization_failed"), true),
            Err("x11.thread_initialization_failed")
        );
    }
}
