# Complete settings action-feedback audit

The first notification pass covered diagnostics, export and save confirmations.
The follow-up audit found transient failure paragraphs and unhandled preference
save rejections in other settings categories. Reuse the existing single toast;
do not add per-page notification components or change the neutral layout tokens.

- Subtitle display, colors, motion, size, opacity, alignment and overlay lock,
  application language, Dock and appearance choices save quietly on success.
  A sanitized failure identifies the setting and preserves the store's rollback.
- Language-save, input switching, application-list refresh/application switching,
  Windows output selection, profile create/select/delete/credential-delete,
  saved-credential reveal, history deletion, Releases opening and normal-quit
  failures use the same transient notification. Keep retry controls usable and
  discard feedback invalidated by navigation, blur, hide or unmount.
- A settings modal makes its background inert. Portal the existing toast into
  the active modal so it is visible, accessible and manually dismissible, while
  keeping exactly one notification in the window and preserving the focus trap.
- Proxy and credential/name forms retain their unsaved draft and actionable
  error/retry controls. Connection checks retain timing/results. Initialization,
  missing audio devices/applications, session errors, recording limits, ongoing
  disk-write/read failures and update install/restart choices are persistent
  state, so they remain beside the relevant controls.
- History deletion failure must say deletion failed, not history reading failed.
  Keep these distinct messages in English, Simplified Chinese and Japanese.

Verify rejection handling, sanitization, rollback/retry, late completion after
navigation, modal visibility/dismissal, unchanged source/recording opt-ins and
credential-preview cleanup. Run the canonical checks and signed macOS UI-only
settings smoke test; do not infer native Windows capture or real updater success
from frontend fixtures.
