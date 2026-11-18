//! Initialize native threading before Tauri/GTK creates any display connection.

use std::sync::OnceLock;
use x11_dl::xlib::Xlib;

pub fn initialize_x11_threads() -> Result<(), &'static str> {
    // Keep the library loaded for the process lifetime. x11-dl is already used
    // by Tao; opening it here does not create a display or force an X11 backend.
    static XLIB: OnceLock<Result<Xlib, &'static str>> = OnceLock::new();
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
}
