# Settings notifications and development debugger

One-time action feedback uses one compact fixed toast in the settings theme
root, outside its scrolling panels. Success lasts three seconds, failure eight;
manual dismissal, category navigation, window blur, document hide, native close and unmount
clear it. A newer operation supersedes earlier pending notifications, and a
late result cannot revive a notification after its scope changes.

Audit and changes:

- Diagnostics copy, refresh and feedback use the shared toast. Remove the
  diagnostic-page CSS override that placed notifications in normal flow.
- Service/profile save confirmations and language-save confirmations use the
  toast, including applied compatibility adjustments; unsaved-field errors remain in their editors.
- Session export/clear outcomes, clipboard-paste failures, development-case
  export/failures, restart acknowledgement and user-initiated up-to-date checks use the same toast.
- Connection checks keep their request duration and results beside the action.
  Input validation, credential/storage failures, evidence-loss counts, recording
  limits and update download/restart choices remain visible. Overlay and tray
  capture errors are actionable state, not success acknowledgements.

The development debugger follows the diagnostics page's neutral typography and
spacing. Group matching compact buttons in one wrapping toolbar, place metric
labels above values, localize event/snapshot labels, and use explicit chevrons
for configuration and event details. Put stage selection alongside the event
heading. Empty recognition/translation uses a single clear empty state;
populated output keeps source identity and bounded selectable text.

Trace collection, explicit audio/subtitle recording opt-in, case-bound reads,
loss accounting, bounded pagination and production exclusion stay unchanged.
Validate those regressions plus notification replacement, expiry, scope
invalidation and stale async results. Check the complete page and fixed toast
in the signed UI-only WKWebView and the existing light/dark/narrow diagnostic fixtures.
