# Shared desktop and Android subtitle core

The user requested one implementation so subtitle and translation fixes reach
desktop and Android together. Shared fixtures alone check two implementations
but still allow them to drift.

## Decision

Extract the existing UI-independent Rust subtitle state machine and wire models
into `shared/mimi-core`. Desktop imports it directly. Android calls the same
compiled crate through JNI; Kotlin maps native callbacks and renders snapshots.
OS capture, permissions, credential stores and network adapters remain native.

The shared core owns complete text bounds, current complete pairs, preview
replacement, confirmation identities, history bounds, final queue admission,
total deadlines, retry decisions and finite finish windows. Preserve complete
text within the byte limit rather than cutting at punctuation or retaining a
suffix. Empty/oversized input preserves the last valid display. Only a complete
pair replaces a complete pair. Older finals may enter opted-in history without
replacing a newer display owner. History remains off by default.

DashScope conversation-item alignment and OpenAI append-only transcript alignment
also move from desktop into this crate. Kotlin only decodes transport frames and
forwards shared outputs. Known unlinked responses never guess arrival-order pairs.
Both timing streams and identity maps have bounded, validated serialized state.
Complete DashScope pairs preserve the source item ID through the reducer. A late
older pair may enter opted-in history while the newer source/display/language
owner remains visible. Identical text from distinct items remains distinct;
duplicate completion of one item is idempotent. These end-to-end cases run
through both direct Rust and real JNI, including clear/reset boundaries.

Explicit stop seals audio admission, permits accepted provider tails, drains
accepted final translations for a finite window, publishes their snapshot and
then closes. Errors, permission revocation and destruction abort immediately.
Retired-generation callbacks never change a new session. Readiness must gate
or bound early audio rather than silently discard it.

## Alternatives and costs

- Separate Rust/Kotlin implementations plus fixtures keep independent builds
  but do not satisfy the requested single implementation.
- Moving capture, UI and transport to one framework replaces mature native
  integrations and creates unnecessary permission and resource risks.
- A pure shared Rust core fixes rules once and retains native integrations. It
  adds pinned NDK cross-compilation, a JNI ABI and native libraries to APKs.
  Existing serde/JSON and already locked JNI dependencies are reused; no new
  service, credential requirement or runtime debug feature is added.

JNI exchanges bounded JSON state without pointer handles or a global session
registry. Errors use fixed labels without input, credentials or provider bodies.
Opaque serialized state has a 2 MiB aggregate budget within the 8 MiB request
limit, including room for JSON escaping on the next call. Every accepted state
must be restorable; an invalid or over-budget operation fails without replacing
the caller's last accepted state.
Tests call the actual host library; no Kotlin fallback implements the reducer.
Android libraries support the four packaged ABIs and 16 KiB page alignment.
Build caches remain reusable.

## Validation

Run the same synthetic multilingual events in Rust and through Android JNI:
punctuation, long sentences, new recognition during translation, late finals,
repeated speech, clear/reset, disabled history and UTF-8 limits. Add focused
retry/deadline, drain-vs-abort, generation and early-audio tests. Shared changes
trigger desktop and Android CI. Run canonical desktop checks, Android unit
tests/lint, APK builds and native packaging checks. Native local inspection uses
only the dev app. Emulator lifecycle checks do not establish real-device audio
capture or real provider acceptance; report those validation limits separately.

Translation prompts remain unchanged. Different service/model choices do not
promise identical translations. Document remaining adapter/model scope.
