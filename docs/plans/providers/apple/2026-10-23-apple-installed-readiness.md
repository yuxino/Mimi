# Apple Translation installed-resource recovery

The user reports that Apple translation sometimes requires the settings
preparation action again after an application update. This is an observed
symptom, not proof that updates revoke authorization or remove language packs.
Keep the running production app and video playback untouched while investigating.

## Native boundary

The current bridge treats a new installed-only session's `isReady == false` as
missing assets immediately. Normal translation also probes one session, cancels
it, then creates another session for the actual translation. Only the explicit
settings action calls `prepareTranslation()` through a SwiftUI session.

Use one native installed-only readiness routine for status and translation.
Reject unsupported pairs, and require `canRequestDownloads == false` before
querying or preparing the session. If it is already ready, continue immediately.
Otherwise make one preparation attempt and recheck the same session. A global
supported-but-unready inventory result is not a reason to skip this safe local
check. Translate with the same session that passed readiness.

Missing assets still require the explicit settings action; a failed preparation
or false readiness cannot be reported as installed. Preserve cancellation and
other native errors instead of disguising them as missing resources. Retain the
existing Rust request deadlines, native request bounds and cancellation ownership.
Do not add polling, a persistent ready flag, automatic SwiftUI preparation,
permission resets, signing changes or startup windows.

Apple documents that [installed-only sessions](https://developer.apple.com/documentation/translation/translationsession/init(installedsource:target:))
cannot request language downloads, and that
[preparation](https://developer.apple.com/documentation/translation/translationsession/preparetranslation())
does not prompt when resources are already installed. The proposed recovery uses
that boundary; its effect on the user's update scenario remains to be measured.

## Verification

Exercise the real readiness routine with injected sessions: ready fast path,
cold readiness recovered by preparation, inventory lag, missing resources,
generic failures, forbidden download capability, and cancellation across waits.
The native regression executable must not initialize real translation sessions,
query models, activate windows or access audio. Run the native Swift checks and
the appropriate desktop/canonical checks in the background with bounded jobs.

A signed same-identity restart/update comparison remains necessary to establish
that this fixes the reported update behavior. It is deferred while the user is
watching video. Browser fixtures cannot prove native model or update readiness;
no interface layout changes are needed for this correction.
