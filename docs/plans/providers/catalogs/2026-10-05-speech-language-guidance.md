# Speech language parameters and configuration guidance

## Problem and chosen scope

Users cannot tell whether a language choice configures a real API parameter, constrains recognition, or adds a capability to a third-party model. A generic “use Auto for mixed languages” tooltip is wrong for explicit-source services and for arbitrary custom endpoints. The custom catalog is also artificially fixed at four languages, while xAI input hints and documented OpenAI/Gemini outputs are missing.

Keep the existing profile, preference and protocol boundaries. Use provider-specific help shared by Settings, tray and floating controls; show unknown custom model support as a visible, normal-size constraint. Auto on a custom route is labeled **Service default**, because the encoder omits the parameter. No endpoint discovery, capability probe, model download or credential migration is added. Official route catalogs are maintained by Mimi against the documented API, not remotely discovered. For custom speech, an optional per-profile `customSpeechSourceLanguages` declaration narrows explicit choices: missing/null means unknown, an empty list means Service default only, and a list contains only represented explicit codes. It is user-supplied metadata, never a verified model capability. Service default remains available and omits the language parameter. Saving a narrower declaration normalizes the active source atomically; failed saves preserve both stored state and the editable draft. The details page has no duplicate Add configuration action; new profiles start from the service list. When enabling text translation normalizes an unsupported explicit source back to Service default, one transient notice names the actual before/after languages. It waits for both successful completion and the applied settings event, in either order, and cannot reappear after blur or unmount. The tray has no translation toggle and displays the resulting shared preference.

## Request and catalog changes

- Custom ASR exposes every currently representable explicit source code in recognition-only mode (30 plus Service default). These are configurable protocol values, not supported model languages. DashScope uses `language_hints`; OpenAI uses `language`, or `languages` for `gpt-live-transcribe` names. Names determine wire shape, never endpoint trust. OpenAI-compatible/ChatMock text routes accept all currently represented source and target codes as configurable protocol values, without claiming the chosen model supports them. Existing Chinese/English/Japanese prompts stay byte-for-byte unchanged; added target mappings use stable names shared with Android contracts. DeepL/DeepLX retain their independent implemented language limits; unknown reported languages under Auto remain service-detected (see the [detected-source design](../translation/2026-10-05-detected-text-source.md)). A custom speech declaration intersects the chosen route’s supported encoder values and cannot expand that route.
- xAI adds a source preference that reaches `audio.input.transcription.language_hint` on the actual WebSocket setup. Only 11 unambiguous documented codes represented by Mimi are accepted; Auto omits the hint. Regional Spanish, Portuguese and Arabic defaults are not invented. Existing translation instructions remain verbatim.
- OpenAI dedicated realtime translation accepts its documented 13 target codes. Gemini uses its own independent 30-entry target catalog within Mimi's represented language set, with `zh-Hans`, `zh-Hant` and `fil` mappings. Generic Portuguese remains unavailable because the service documents regional variants.
- Rust, TypeScript and Android catalogs consume the same synthetic expectations in `shared/translation-contracts.json`; Rust and Android tests exercise actual encoders. Android expanded labels use system locale display names, retaining existing translations for the original entries.

Alternatives rejected: displaying one universal model-support list would misrepresent custom services; adding user-editable arbitrary JSON would expose unsupported protocol combinations; dynamically querying private services is neither a common protocol capability nor needed for this request.

## Verification and remaining boundaries

Cover omission versus hint, unsupported regional choices, target mappings, custom recognition-only versus translated-route intersections, persisted declarations and clearing, active-source normalization, rejected/busy saves, actual xAI loopback setup, localized help and all sibling controls. Generic text fixtures exercise the actual Rust and Android encoders; desktop-only custom declaration editing is documented in platform parity. Run the canonical repository check and Android shared-contract CI. No live cloud request or native capture is needed to verify field serialization. Signed native UI acceptance remains a separate check coordinated by the main task; this branch does not launch or install a dev app.

The [setup guide and audit matrix](../../../speech-language-setup.md) state the exact implemented subsets and unresolved provider gaps. This change does not claim all providers expose their complete official language catalogs.


## Language action failures (implementation only; checks deferred)

The October 5 follow-up keeps one translation target at a time. The current
31 generic output mappings, 13 OpenAI outputs and 30 Gemini outputs match their
implemented encoders; the other providers retain the documented narrower
integration limits. This is not a claim that every upstream language is exposed.

Source-language changes now reject busy, unsupported, superseded and failed
preference writes instead of returning success. Saving fails before any subtitle
state mutation or reconnect. After a durable selection, a reconnect failure
leaves the selected preference and the existing session error visible, and also
rejects the initiating control; no automatic rollback is claimed. Other lifecycle
callers retain their existing behavior. Settings, tray and overlay map known
sanitized language/provider errors and never display arbitrary IPC bodies.

The latest main service-switch change (#163) concerns saved profiles; it does
not replace this source-language failure path. Apple runtime checks belong to
its separate integration branch. Regression source was added, but no tests,
format checks, builds, UI acceptance or CI were run for this follow-up at the
user's request. Verification is explicitly pending.

## Product integration follow-up

Preserve main's independent recognition display name and live saved-profile
selection alongside optional language declarations. Profile updates patch both
metadata fields independently; selecting a declared profile keeps its source
validation before credential resolution. Translation-target switches now return
the actual reconnect outcome, just like source-language switches, so a failed
reconnect cannot dismiss its initiating control as a successful operation. A
durable selection remains selected on reconnect failure; rejected persistence
leaves the previous selection intact.

The setup guide distinguishes saving from checking a connection and states the
single-target semantics. Historical local-model session reports are excluded
from this product integration. No tests, builds, native UI or provider requests
have run for this follow-up; current integration verification remains pending.
