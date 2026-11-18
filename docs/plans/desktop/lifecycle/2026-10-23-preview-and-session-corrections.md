# Preview corrections and session entry

This pass covers three independently reproduced boundaries outside Apple
resource setup. Verification remains in controlled tests and hidden browser
fixtures while the user watches video.

## Preview content identity

The preview request fingerprint currently removes every punctuation character.
This conflates decimal and signed values such as `1.5`/`15` and `-5`/`5`, so an
ASR correction can leave an obsolete translation preview until a later textual
change or final event. Retain numerical separators/signs and word-internal
connectors in the shared fingerprint while continuing to ignore terminal
punctuation and repeated whitespace. Keep fixed-size metadata, existing request
budgets, revision pacing, final ordering and provider protocols unchanged.

Verify the same cases directly in Rust and through the existing Android JNI
exchange with shared fixtures. A loopback HQ test must observe a second request
and corrected preview, rather than only compare hash values. This does not
establish live-provider latency or speech recognition quality.

## Recognition-only settings entry

First settings navigation must use the credential requirements for the selected
target. When the target is Original, configured recognition is sufficient even
if the retained translation route is incomplete. Reuse the existing shared UI
selector already used by start controls. Keep explicit page/deep-link intent
and translation-mode setup requirements unchanged. Cover ready snapshots and
initialization completion, then inspect the actual settings components in a
hidden browser using synthetic profiles.

## Concurrent pause and resume

Settings and the floating subtitle window can issue independent pause/resume
commands. Claim the transition before reading its paused state so a duplicate
cannot reverse the accepted action or supersede its connection generation.
Resume also shares Start's existing admission guard; Stop keeps its ability to
supersede connection work. Keep the accepted transition's guard alive until
completion and release it after errors or cancellation. Verify the production
admission helper with controlled interleaving rather than reproducing its logic
in a separate test model. No real audio or session is needed for this boundary.
