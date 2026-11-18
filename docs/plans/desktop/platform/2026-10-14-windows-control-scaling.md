# Windows control scaling and opening focus

The control surface uses CSS measurements while native Windows bounds use
monitor-scaled pixels. Windows text scaling adds WebView zoom on top of the
monitor scale, so treating CSS pixels as native logical pixels clips the
island and panel. Divide the WebView pixel ratio by its native monitor scale,
then apply that zoom only to Windows control geometry and work-area fitting.
An unchanged CSS measurement must still update when the pixel ratio changes.

WebView2 can still expose the island viewport when panel autofocus runs.
Focusing with scrolling enabled at that point moves the header out of view.
On Windows, focus without scrolling, then reveal the focused control after the
latest height request is acknowledged and the viewport has actually expanded
beyond the island header. IPC completion alone does not prove the queued native
resize has reached the WebView. Ignore completions and observer callbacks from
an earlier measurement or a closed panel.

macOS and Linux keep their existing focus and content-size reporting behavior.
They ignore the additional pixel-ratio payload, including invalid values, and
continue using zoom 1. Linux GTK sizing and Wayland embedded-control geometry
remain unchanged.

Regressions cover monitor-DPI/text-zoom conversion, invalid Windows scales,
non-Windows dimension updates, delayed native viewport changes, DPI changes
without content-size changes, and delayed callbacks across panel remounts.
Native Windows verification is still required for the packaged WebView2 path.
