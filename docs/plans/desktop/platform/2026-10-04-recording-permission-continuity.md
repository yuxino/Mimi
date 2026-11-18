# Recording permission continuity in release and development builds

A macOS Screen & System Audio Recording consent regression after a local
certificate migration established the boundaries below. Credential migration
and recording permission are independent. Preserve existing configuration and
credentials when diagnosing or repairing a recording grant.

## Established causes and boundaries

Release and development have separate bundle identifiers and stable signing
pins. Verify each app's full designated requirement against its own installed
identity; do not infer continuity from the certificate display name. Normal
rebuilds and updates preserve that requirement. An intentional certificate
migration requires its own explicit scope; never create a replacement identity
or weaken system consent as routine recovery. A visible enabled Settings entry
alone does not establish that it matches the current binary. The shortcut uses
the regular session start path; first capture still requires a valid macOS grant.

Two regression paths must stay covered. Application enumeration uses NSWorkspace
for running application names and bundle identifiers, with stable filtering and
sorting; only actual capture accesses ScreenCaptureKit. Native stream-stop errors
must preserve permission/user-stop reasons. Stop recovery immediately on
non-retryable capture permission or user-stop errors. Keep transient failures
eligible for bounded recovery and preserve the final relevant error.

Development launch must resolve its own optional signing pin instead of the
release pin. Keep explicit overrides, unique fixed-certificate fallback, and
full designated-requirement verification before installation. Missing or invalid
pinned identities fail closed. Normal updates must retain each installed app's
identity; intentional migration is separate from routine packaging.

## Verification and recovery

Add focused regressions for enumeration without capture access, native terminal
error classification, permission denial during recovery, and separate signing
pin resolution. Run the canonical checks and verify both signed bundles. Test
application listing, shortcut start, stop/restart and same-identity replacement
from the canonical paths; report actual capture separately from mocks/UI-only
mode. Real capture or provider validation needs authorization for the current
task; a historical report is not continuing permission. Do not retain or log
user audio/subtitles.

The README FAQ links concise recovery steps for a stale grant: normally quit the
affected app, remove only its old recording entry, add its matching canonical
application and enable it, then reopen and test. Development and release grants
remain separate. Do not reset global TCC, edit the permission database, grant
unrelated access, or manipulate Keychain to repair recording permission.

References: [Apple recording access controls](https://support.apple.com/guide/mac-help/control-access-screen-system-audio-recording-mchld6aa7d23/mac)
and [running applications](https://developer.apple.com/documentation/appkit/nsworkspace/runningapplications).
