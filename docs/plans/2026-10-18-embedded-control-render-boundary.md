# Isolate embedded controls from subtitle-only parent renders

The native embedded-control path renders `OverlayControlWindow` inside
`OverlayWindow`. The parent subscribes to the complete session snapshot, so
every replacement draft rendered its child again even when all of the child's
primitive session selectors, settings and native mode remained unchanged.
Each render rebuilt the control model; an open panel also recreated its
provider/language options and guidance. Separately mounted control windows did
not have this parent-driven work, so the existing sibling-window regression
did not exercise the affected path.

Wrap only `OverlayControlWindow` in React's ordinary memo boundary. The sole
prop is the stable `embedded` boolean. Keep its existing store, native-mode,
shortcut, measurement and local-state subscriptions as the owners of their
updates. Do not memoize the overlay, change subtitle stabilization, cache
provider policy or alter popup observation. Add the same explicit effective
UI-language subscription already used by memoized subtitle lanes: this window
must still refresh localized copy when an App render cannot cross the memo
boundary. Selecting System reads the current navigator language; this does
not introduce an operating-system language-change notification contract.

The formal regression mounts the actual OverlayWindow, control window, panel,
capsule and Timeline with synthetic native IPC. Pass-through spies count real
parent chrome and control-model computations. In hidden, island and panel
modes, fifty source/translation/latency snapshots each render the parent fifty
times. The old implementation additionally computes the control model at
least fifty times in every mode; the memoized implementation adds zero calls.
The existing subtitle stabilizer still publishes the latest draft, and surface,
panel, picker and open portal identities survive the update burst.

Independent-update checks cover all eight UI languages without settings
replacement, selecting System with a different navigator language, static
closed-panel copy, target-language and pulse-style settings, paused/stopping
session signals, disabled controls, native mode transitions, late shortcut
discovery, island ResizeObserver reports and native listener cleanup. Native
IPC and DOM geometry are fixtures; these tests establish work counts and
React behavior, not native Wayland hit testing, production CPU, frame rate or
end-to-end latency. The separate body-wide popup MutationObserver candidate
remains unchanged and needs its own design and acceptance.

The affected quick frontend check passes provider-copy verification,
typecheck, selected lint and diff checks, plus 199 related regressions in
thirteen files. Independent read-only review found no remaining blockers.

After rebasing onto main `fdc765f`, Chromium exercised the unchanged production
code at `ef148679e9237cf39c8f86e385929cc314c46afa`. The fixture mounts the real
OverlayWindow with synthetic native IPC and the embedded flag. Remove the
motion fixture's placeholder surfaces and its global section-size override
before measuring; retain product CSS and constrain only the fixture stage.
Response-only counters retain the real implementations and assert a unique
insertion point. Each mode receives fifty snapshots: fifty-two parent chrome
computations including settled-tail updates, zero control renders and zero
control-model computations. Latest subtitle text, component identities and an
open portal survive. Visible-DOM predicates verify all eight languages without
settings replacement, target/pulse settings, paused/stopping signals, disabled
controls and island/hidden/panel transitions. The final real panel and open
picker screenshot was inspected; browser errors were empty.

The unchanged observer was measured separately. Fifty short source/translation
and latency snapshots, each awaiting its actual displayed text, produced one
hundred menu rectangle reads
while panel, anchor and portal geometry stayed constant; no bounds IPC was
added. Eight more drafts with a notification present produced sixteen reads
of each menu/toast rectangle, also with unchanged geometry and no new IPC.
Ancestor inline transform, portal resize/add/remove, toast dismissal and
viewport resize provided geometry-change controls; reported menu/notification
slots matched the actual rectangles. Locale changed copy while this sample's
geometry stayed equal, so it correctly added no bounds IPC. This is a bounded
browser work-count probe, not a caption-only attribution, proposed record
filter or a claim about font
loading, every ancestor style change, native input regions or Wayland clicks.

The same frozen frontend then passed the complete single-worker Vitest suite:
146 files and 2,361 tests. Full-project ESLint and the production build passed,
including icon verification, TypeScript checking and scanning all seventeen
emitted files for developer-only code. No test timeout or assertion changed.
The build retains its existing large-chunk warning; no chunk policy was added.
Only this evidence record changed afterward. The fixture browser and Vite
process were closed, and the temporary dependency symlink was removed. There
was no native build, provider request, audio capture or private session data.
