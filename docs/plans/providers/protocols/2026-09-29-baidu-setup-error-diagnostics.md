# Baidu setup rejection diagnostics

Issue #62 shows that a failed Baidu realtime translation setup currently
produces one generic error, leaving users unable to distinguish a provider
rejection from connection or protocol failure. Provider rejections also lose
the code needed to distinguish mismatched credentials, unsupported language
directions, and exhausted usage.

Preserve Baidu's numeric `code` in the setup result and show it in the existing
error message. Continue discarding the provider's free-form `msg`, which may
contain account or content details. Rejections without a numeric provider code
retain the generic error. Keep post-setup provider event codes content-free and
unchanged in shape.

A local mock WebSocket rejection checks that the numeric code reaches the
caller and that the provider message does not. This verifies diagnostics only;
the actual account, service permission, and quota behind issue #62 still need
a real provider session to identify.
