# Android credential draft boundaries

Android's service editor filled every blank secret from the saved configuration
before constructing either a connection probe or a saved profile. This bypassed
the shared runtime's existing destination ownership checks: changing an Azure
resource, Tencent AppID/SecretID, or Baidu AppID could retain the old secret.

Both editor actions now pass raw fields and saved credentials through a thin JNI
operation calling `ProviderCredentials::resolve_draft_for_profile`. Keep the
existing ownership/validation policy in shared Rust. Kotlin only serializes the
native fields and presents required-field feedback; it does not reimplement URL
normalization or credential inheritance. A rejected draft performs no check or
save. A complete explicit replacement remains possible, and unchanged owners
may retain their saved secrets. No request or response is logged or persisted
by the bridge.

Legacy Android Azure fields may include trailing slashes or whitespace. Compare
both resource addresses with the existing shared canonical validator. Equivalent
spelling retains the key; invalid old addresses do not block complete explicit
replacements and cannot supply a key to a new resource.

The text translator's connection-check control also tracks edit availability
separately from its pending request. A session-state refresh cannot re-enable a
pending check, and cancellation/disposal cannot enable an unavailable editor.
The click handler rejects disabled or duplicate actions independently of View
state.

Verify shared credential regressions directly and through actual JNI, Android
adapter serialization and editor save/probe paths, and the slow loopback UI
fixture's pending/refresh/disabled transitions. Host tests and compiled native
fixtures do not establish device UI acceptance or real provider connectivity.
