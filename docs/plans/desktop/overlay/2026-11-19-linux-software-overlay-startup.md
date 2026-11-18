# Linux software-rendered overlay startup

## Decision

Keep WebKit's default acceleration policy for the transparent subtitle WebView.
Remove the X11/software-GL override and its otherwise unused direct dependency.
Keep the separate settings/control windows and existing native input behavior.

## Evidence and boundary

At release source `b83d99ef`, installed Linux deb startup failed its existing
30-second readiness gate. Diagnostic revision `0d352449` reproduced it on
Ubuntu 22.04 / WebKitGTK 2.50.4: settings was ready after about two seconds,
both windows were visible and the synthetic session was listening, but the
overlay readiness marker was absent. The test's gates and timeout were unchanged.

Revision `2ad15243` omitted only the `HardwareAccelerationPolicy::Never` setter
from that path. The same package fixture passed three independent deb launches
and three AppImage launches, including move/resize, lifecycle and shortcut checks.
This establishes the regression boundary; it does not establish a specific
WebKit internal failure or a live-caption latency improvement.

Mimi's normally constructed WebViews have separate settings, although their
context can be shared. Moving the
override to other windows, changing a global environment switch, reloading a
running session or loosening readiness is unnecessary for this repair.

## Verification

Repeat both package formats on the final helper/dependency removal. Preserve
the content-free session, marker and window diagnostics if startup fails.
Keep each launch independent and fail at the first failure; never extend the
timeout or remove the paint/IPC prerequisites to obtain a pass.
