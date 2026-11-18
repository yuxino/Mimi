# Session recovery and confirmation reliability

## Reproduced failures

- The xAI streaming recognition client emits `xai_stt_*` error labels, but the
  shared recovery policy did not recognize them. Temporary transport failures
  therefore stopped the session instead of entering bounded recovery.
- Desktop maintained its own provider configuration-error list. It omitted
  Gemini transcription errors already handled by the shared policy, so recovery
  could retry an authentication/configuration failure and replace its actionable
  error with the earlier connection failure.
- A new session's identical identity-less final could retain the previous
  session's display confirmation time, despite accepting a new confirmation.
- Separately, the shared reducer's zero-history contract could append a repeated
  identity-less final to its `ArchiveSink` more than once. Current production
  runtime controllers retain 20 reducer rows and crop presentation separately;
  this is a reproduced component-boundary defect, not evidence that the current
  application duplicates saved sessions when display history is disabled.
- A valid local credential document containing a damaged custom-recognition
  value could not be repaired by saving a complete replacement. The save path
  attempted to decode the damaged old value before considering the new fields.

## Decisions

Keep provider recovery classification in `shared/mimi-runtime`. Recognize exact
xAI transport and authentication/configuration labels, preserve the existing
bounded retry schedule, and let the desktop add only its native capture and
permission conditions. Unknown provider text continues to use the sanitized
fallback. No protocol, model, prompt, timeout or retry-budget change is needed.

Use the shared reducer's existing retained complete pair and current-session
confirmation boundary to reject duplicate identity-less finals independently
of the display-history limit. A new recognition cycle, explicit clear or new
user session must still admit identical content. Reconnect alone must not rearm
it. Accepted confirmations update their own display time; provider identities
and independent audio sources retain their existing ownership rules. Do not
introduce another transcript cache or Kotlin reducer.

For a complete custom-recognition replacement, validate and encode the new
fields without decoding the old slot. Partial edits still require a valid old
value. Read the old raw slot for transactional rollback; preserve independent
translation credentials and reject an unreadable credential document or store.
This does not reopen native credential fallback or change credential migration.

## Verification boundaries

Use synthetic credentials, local temporary files and deterministic events.
Require a failing regression before each fix, then repeat those cases and check
the neighboring cancellation, recovery, retention and rollback paths. Shared
fixtures run in direct Rust and through the actual host JNI library. Run the
canonical repository checks and Android JVM suites on the combined change.

An isolated signed development launch can verify startup and native rendering;
it cannot establish live provider recovery. This task does not authorize audio
capture, access to production credentials, a public release or replacement of
the running production application. Windows/Linux devices, physical Android
devices and real provider sessions remain distinct acceptance boundaries.
