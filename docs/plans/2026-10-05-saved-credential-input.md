# Saved credential inputs

The settings editor must distinguish a usable translation destination from a
stored optional token. An unauthenticated OpenAI-compatible destination is valid
and must not offer to reveal a nonexistent token or show a missing-key error.

A settings-only editor-state command returns field presence and the configuration
needed by the open editor. Its response stays local to that editor, including
service URLs that may contain private path segments. Secret values are excluded
from this response and all global snapshots. Actual secret viewing remains an
explicit, field-scoped request.

Use one shared editable input for hidden, shown and replacement states. Viewing
a stored value does not create a replacement draft. Editing it does. Once the
user has a draft, the eye shows that draft without retrieving the old value.
Focus loss, window hide/close, disabled operations, route changes and unmount
invalidate pending reads and clear loaded secret values. They mask but preserve
unsaved replacement drafts. Stale responses must never overwrite edits.

Populate saved addresses and models into editable fields. Track user edits
separately from loaded configuration, reject blank required replacements, and
submit unchanged fields as unchanged so merely displaying an address does not
trigger destination credential clearing. Cancellation restores stored values;
a successful save refreshes the local editor state.

Verification covers keyless destinations, same-input viewing/editing, delayed
responses, draft retention, all sibling provider editors, and signed development
UI fixtures. Existing active subtitle sessions must remain undisturbed until the
user permits restarting the shared development bundle.
