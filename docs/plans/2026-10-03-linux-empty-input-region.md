# Linux overlay input-region lifecycle

Scope: the fully empty input-region and lifecycle portion of #92. This does
not implement compositor-independent placement, always-on-top, workspace
pinning, GlobalShortcuts Portal, or layer-shell.

## Decision

Tao 0.35.3 implements ignored cursor events with a 1×1 Cairo rectangle. On
Linux, use GTK's widget input-shape API with a genuinely empty Cairo region.
Pass `None` when unlocking so GTK restores its normal input shape, including
any client-side-decoration constraints. Do not patch or fork Tao and do not
change the macOS/Windows paths.

Keep requested state separate from applied state. Record the newest request
before queueing work, resolve the current native window on GTK's main thread,
and then reconcile the latest request. A delayed lock must not overwrite a
newer unlock. No presentation-state mutex is held while dispatching to GTK.
A failed dispatch/native lookup does not update the applied cache.

GTK combines the widget shape with its own input region, but clears the
application shape on unrealize. Restore the latest request on both realize
and map. Recreating an overlay invalidates the applied cache while preserving
intent. Linux records locking before showing the overlay so its first map
cannot intentionally restore a stale unlocked state. The GTK setter returns
no compositor acknowledgment; applying it is not proof of pointer delivery.

## Regression coverage

- Cairo region is empty, including (0,0); unlock is `None`, not an empty region
- Duplicate snapshots are deduplicated after application
- A queued lock followed by unlock reconciles only the newest request
- Remap forces reapplication; recreate invalidates cache without losing intent
- An explicitly isolated native GTK fixture exercises first map, unlock,
  relock, hidden unlock/remap, unrealize/realize, unchanged-locked remap and
  re-realization, and full window recreation
- A Wayland protocol checker requires actual `set_input_region` requests
  followed by `wl_surface.commit` in all nine phases. Its own synthetic
  regressions reject Tao's 1×1 region, missing commits, missing phases, and
  an unlock that leaves input disabled

`./scripts/linux-wayland-input-smoke.sh` creates a private session bus and
Weston headless display with software rendering, private HOME/XDG directories,
and no X11 fallback. It does not attach to an existing desktop, enable
accessibility, request input permissions, use providers/credentials, or change
system settings. The ignored Rust fixture refuses ordinary desktop execution
unless that opt-in and the native Wayland backend are present. Standard Rust
checks still run the display-independent tests.

The native fixture uses Mimi's actual GTK region helper/lifecycle callbacks,
not a replacement Mimi binary. It validates protocol handling only: it does
not prove full packaged-WebView behavior, input delivery to a window beneath
Mimi, or GNOME/KDE/wlroots acceptance. Those remain separate #92 checks, and
#92 must stay open. Existing X11 package smoke coverage remains separate.
