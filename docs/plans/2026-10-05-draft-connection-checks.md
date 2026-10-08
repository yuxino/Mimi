# Check a service before saving

Connection checks accept the current editor draft through the existing dedicated
check command. Saved configuration remains the fallback when no fields changed.
Complete drafts can be checked before their first save; incomplete or invalid
edits keep the check disabled. Speech and text checks remain independent, and
independent text checks neither require nor read the speech key.

Native preparation resolves an ephemeral validated configuration using the same
text destination merge rules as saving. An unchanged destination may reuse its
saved key; changing the destination never borrows that key. Explicit token removal
is respected. No draft check writes credentials, selects a profile, updates
preferences, starts capture or emits private configuration in a snapshot.

Diagnostics contain only the existing sanitized result and measured duration.
The UI associates outcomes with a content-free input identity; editing fields,
switching routes, navigation and stale requests must not show an older result as
validation of the new draft. Existing explicit timeouts and stage separation stay
in place. UI-only builds still make no provider requests.

Verify new and saved profiles, optional authentication, clear-token and changed
address behavior, malformed inputs, independent development profiles, no storage writes,
no unrelated secret reads, failed/late requests and the actual signed settings UI.
These are desktop configuration changes; provider wire and shared subtitle rules
remain unchanged.
