# Recoverable settings hydration and independent session events

The signed native handoff reproduced an overnight OS Keychain wait followed by successful backend listening/ASR/MT while the subtitle overlay still displayed idle. Each WebView owns a separate store. The previous settings snapshot deadline removed both settings and session listeners and expired their shared generation; Settings could retry its own store, while a hidden overlay had no retry path.

Keep one lifetime coordinator per WebView with independent settings/session subscriptions, snapshot operations and event revisions. A 12-second UI deadline reports loading failure without removing a successful subscription or invalidating its consumers. Session snapshots/events continue immediately while settings is blocked. An original settings read that finishes after OS authorization can hydrate persisted preferences automatically; a newer settings event wins over that older response. A late listener acknowledgement starts its first snapshot read once even if the UI attempt has already timed out, so hidden windows do not require a retry button.

Explicit retry reuses an installed/pending listener and unfinished native read. Only a genuinely rejected native operation permits a new read; there is no polling, background Keychain retry, secret fallback or OS authorization bypass. Retain the first read promise through listener reconciliation, including a fast rejection, to avoid accidental automatic re-reads. Ready transitions can come from either snapshots or events. The store's generation expires only at actual WebView disposal, not a recoverable deadline. Dispose aborts UI waits, removes established subscriptions, cleans any late listener exactly once and ignores late responses. A persisted pagehide keeps the same document's subscriptions.

Focused tests reproduce idle hydration, an eight-hour settings timeout, backend listening/preview delivery and late persisted display/proxy settings hydration. They also cover explicit retry without duplicate native work, latest events beating late responses, rejected-operation retry, late listener recovery without manual input, disposal/replacement isolation, cleanup failures and persisted-page lifecycle. This proves the frontend recovery contract; signed native listening and visible subtitles still require the coordinated rebuild and OS-authorized audio acceptance.

## Native overlay follow-up

The resumed native session exposed a toolbar movement when Clear disappeared:
the pointer was left over the adjacent Collapse action and its tooltip. Keep
the same Clear button slot when content is empty, disable its action and native
pointing target, and re-enable that node when new content arrives. Existing
narrow-window secondary-action rules remain unchanged.

A very short bilingual viewport also crossed the source-font scale threshold
when translation arrived and subtracted the lane gap. Compute the reference
font scale from mode and viewport independently of translation presence, and
use that same scale for layout measurement and line budgeting. Actual lane
space can still expand while translation is pending. Regression coverage
includes 83, 84 and 85px viewports across waiting/paired/waiting transitions.
