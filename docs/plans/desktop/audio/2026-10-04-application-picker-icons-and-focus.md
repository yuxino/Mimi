# Application picker icons and floating-panel search

The user requested application icons in the floating panel's audio picker and
reported that its search field would not accept typing. Keep the shared picker
consistent in settings and the floating panel, without changing capture routing,
selected targets or recording permissions.

## Local icons

Add optional `iconDataUrl` presentation metadata to `AudioApplication`. On macOS,
use the existing NSWorkspace application list and NSRunningApplication icon. Do
not enumerate screen contents or use ScreenCaptureKit for names or icons. Keep
AppKit objects on the application's main thread and preserve the current graphics
context even if an individual icon fails to render.

Sort, deduplicate and retain the existing maximum of 512 choices before rendering
icons. Redraw each icon into a fresh 32×32 RGBA bitmap and encode PNG with the
already available AppKit bindings and base64 dependency. Limit one PNG to 8 KiB
and all encoded icon URLs in one snapshot to 1 MiB. A failed, missing or oversized
icon leaves a selectable text row with the shared fallback icon. Windows returns
no native icon in this change; Linux retains its unsupported application picker.

There is no new disk storage, persistent icon cache, dependency, network request
or permission. Each WebView keeps one bounded in-memory icon map across control
panel remounts (128 icons, 12 KiB per data URL, 1 MiB total). On macOS, restore the
selected icon silently on mount or an external target change through NSWorkspace.
Do not cache selectable lists or change the target. Background failure or a closed
application falls back without an error; manual refresh still reports failure.
Generation checks prevent an older request from replacing newer icons. Icons,
application names and identifiers stay out of diagnostics, recordings and saved
capture preferences; preferences still store only the existing selected target.

## Floating-panel typing

The reported focus failure is specific to the macOS floating NSPanel; settings
search already accepts input. Let a user interaction make the control panel key
by disabling `becomesKeyOnlyIfNeeded` for that panel. Preserve its nonactivating
style and `canBecomeMain = false`. User testing showed that this policy alone
still let keystrokes go to the media application. On an explicit user request
to expand controls, give the actual WKWebView first-responder status, then call
AppKit `makeKeyWindow` on that visible panel, after rechecking that it remains
expanded. Wry wraps the WebView in a parent content view; that wrapper is not the
keyboard responder. Setting only key-window status also failed user testing. Do not call Tauri `set_focus` or activate
NSApplication. Passive subtitle/status updates do not request keyboard input.
When controls collapse, use window ordering to release key status and restore
only the passive island. Do not change the subtitle overlay's keyboard policy.
Class-selector tests establish declared policy, not runtime keyboard delivery.
During IME composition, Escape must not dismiss the dropdown or panel.

## Verification

Cover optional camelCase serialization, missing icons, per-icon and snapshot byte
limits, valid small PNG output and restored graphics context. Keep the native
class policy check independent from opening a real window. The final signed-app
check must inspect icons and type in both settings and floating-panel search,
including empty results, Escape, closing/reopening and normal subtitle controls.
Offscreen image and policy tests do not replace that native interaction check.

Verification correction on 2026-10-04: computer-use automation typed in the
search field, but the user still reproduced keys going to the foreground app.
Automation can change application activation; that result does not establish
nonactivating keyboard delivery. Keep this release blocked until actual user
interaction works while another application remains active.

The next signed development restart was verified twice by the user, including
clicking another application's text field first, then returning to the floating
search. The temporary content-free native observer recorded local key events.
All those events also reported Mimi active; this does not establish delivery
while Mimi remains inactive or prove the panel-initialization hypothesis.
The observer was then removed completely and the signed development app rebuilt
and restarted. The user repeated the same other-application → floating-search
path and confirmed normal input again. The user also confirmed that reopening
the panel no longer loses the selected application icon. Full-screen Space
continuity and inactive-app delivery remain separate validation limits.
