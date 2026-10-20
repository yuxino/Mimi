# Keep subtitle mutations out of embedded popup measurement

The embedded Wayland control panel shares OverlayWindow's document with the
subtitle canvas. Its body-wide MutationObserver currently schedules popup
measurement for subtitle text, latency and lane style changes. Rectangle-key
deduplication prevents redundant native hit-slot IPC only after reading the
menu and notification rectangles. Chromium at the preceding memo boundary
measured 100 menu reads for 50 individually displayed subtitle/latency updates,
and 16 reads of each rectangle for eight updates with both layers present;
geometry and reported slots did not change. These are work counts, not CPU,
frame-rate or native Wayland input measurements.

Pass OverlayWindow's existing stable subtitleRootRef to the memoized control
window. Ignore a mutation batch only when every record belongs to this known
subtitle subtree, which is a fixed-size sibling of the absolutely positioned
embedded controls. Read the current ref inside the observer callback. Keep the
original schedule for absent/unsafe roots, unknown targets or record kinds,
mixed batches, external siblings, ancestors, popup portals and notifications.
Keep STYLE/LINK targets and added/removed stylesheet-containing subtrees even
inside the subtitle root: those can affect the whole document. This narrow
exclusion does not require tracking detached portal identities or assume that
arbitrary sibling DOM changes cannot affect control geometry.

Keep the initial measurement, requestAnimationFrame coalescing, geometry-key
deduplication, viewport resize, mode teardown and native slot clearing intact.
The stable ref object must not reopen the parent-to-control render path; the
existing fifty-snapshot real OverlayWindow regression remains the guard.
Separate control windows and fixtures without this ref retain the existing
observer policy. Do not replace the observer with ResizeObserver: a position
change can alter hit slots without changing layer dimensions. The existing
style-only attribute observation does not gain a class/font-loading contract.

Regression and browser acceptance distinguish caption-only, latency-only and
combined updates; panel states with no layer, only menu, only toast and both;
actual notification replacements; portal creation/removal/geometry; ancestor
style, stylesheet and mixed records; resize and pending-frame teardown. Compare
reported slots with the final actual rectangles and preserve DOM identities.
Artificial ancestor transforms are compatibility controls, not a reported
production defect. Chromium uses real components and layout with synthetic
native IPC; native Wayland/compositor click-through remains separate acceptance.

The formal caption/latency × four-layer matrix first failed all eight cases
against the original observer policy. Warm the panel's initial focus frame
before opening a menu, and explicitly assert each expected layer exists; a
menu closed by that focus frame must not silently turn a menu case into a
no-layer case. Each channel publishes three individually displayed updates.
The fixed observer performs no popup lookup, layer rectangle read or bounds
IPC for these updates. The no-layer assertion counts popup lookup as the
observable start of measurement, covering work that has no rectangle to read.

Real-component controls retain stylesheet subtree creation/removal, stylesheet
Text changes, LINK addition/removal, ancestor style, unknown sibling and mixed
batches. Actual menu/toast replacement and removal update the synthetic native
slots, viewport resize preserves Select dismissal, and leaving panel mode
cancels pending measurement before clearing slots. A component-callback edge
test additionally checks a null/replaced/unsafe ref, Text and Document targets,
unknown record kinds and empty batches. Document/unknown records are defensive
fixtures, not claimed body-observer emissions. Independent static review found
no blocker. The affected quick check passes provider copy, TypeScript, selected
lint, diff and 210 related tests in thirteen files, including the original
fifty-snapshot memo checks and both 445-draft stress variants. Complete frontend
acceptance is reserved for the combined frozen changes.

Chromium then exercised the same production source with real OverlayWindow,
panel, Select, Timeline and SettingsToastRegion. The original individually
displayed fifty-update sample now adds zero popup frames/rectangle reads/IPC,
compared with 100 menu reads before filtering. Eight displayed updates with
menu and toast present add zero reads of either rectangle, compared with
sixteen of each before filtering. Eight isolated caption-only/latency-only ×
layer cases each display three updates with zero popup measurements; geometry,
layer identities and control render/model counts remain unchanged. The original
hidden/island/panel fifty-snapshot cases retain zero extra control renders and
model computations while the parent and latest visible subtitle still update.

Fourteen geometry/subscription controls check the final reported menu/toast
slots against actual rectangles: genuine notification content replacement,
STYLE addition/Text mutation/removal, LINK addition/removal, a mixed local/menu
batch, ancestor inline transform, portal size/removal/addition, locale and
viewport. The stylesheet and notification controls actually change toast
dimensions. Locale keeps equal geometry in this sample and correctly adds no
bounds IPC. Browser errors are empty; separate final panel/toast and visible
menu screenshots were inspected. The first instrumentation attempt rejected a
new multiline function signature before acceptance; the corrected counter still
requires exactly one match and preserves the production implementation.

The tested production files remain unchanged after the quick/browser runs
(SHA-256 prefixes: control window `40b649018ea4ddf5`, overlay
`ebbb04b6b938cd27`). Browser controllers closed. The Vite command session exited
130, but port 1430 continued responding without a visible owning PID in the
execution environment; cleanup was handed to the coordinator and is not claimed
complete. Temporary browser artifacts are not archived as a durable sample.

Root combined this frozen source with the independently previewed name-draft
repair in acceptance commit `7ba8423b4c6d1d14b3f79df6a52be9e94988a9a0`.
All 2,398 frontend tests in 146 files passed with one worker (232.17 seconds),
followed by full lint, provider-copy verification and the typechecked production
build, including icon verification and the production debugger scan of 17 output
files. No assertions or timeouts changed; the existing large-chunk advisory
remains. Subsequent edits only record evidence. A final explicitly net-enabled
process query still found no owning PID for port 1430; no unrelated process was
terminated and cleanup is not claimed complete.
