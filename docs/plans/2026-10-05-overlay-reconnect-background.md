# Stable ordinary overlay background during recovery

## Observed behavior

A user reported that reconnecting removed the subtitle window's black background.
The installed development app saved ordinary mode (immersive off), an unlocked
window, and 75% background opacity. A native Tencent connection timeout followed
by the canvas Retry action visibly changed the whole card from the forced error
background (96% opacity) back to 75% while connecting. The background did not
become completely transparent in this reproduction, and no preference changed.
This establishes a status-dependent background jump, not proof of the reported
complete disappearance.

## Decision

In ordinary mode, the whole subtitle card must use the saved opacity in error,
connecting, listening and paused states. Put the readable dark background on the
error message and recovery actions themselves, so clearing an error does not
change the entire canvas. Keep the existing temporary opaque, expanded and
interactive error presentation when the saved mode is immersive; recovery still
restores that explicit reading preference. Do not change saved opacity, lock,
collapse or immersive settings, add a new setting, or impose an opacity floor.

Keeping the whole-card error override would retain the surprising transition.
Forcing a new opacity after retry would overwrite the user's appearance choice.
The localized error surface preserves both contrast and the configured canvas.

## Verification

Add frontend regressions that click actual recovery controls and defer IPC through
connecting, success and failure. Include ordinary mode at 75% and 0%, saved
immersive mode, pause/resume, and native pointer blur/motion. Assert that recovery
does not save appearance preferences. Run the canonical repository check and
inspect the signed canonical development app through a real error and retry.
The final canonical check passed: 1,177 application Rust tests (2 ignored),
shared-core/JNI checks, 1,749 frontend tests across 121 files, formatting, strict
Clippy, lint, typechecking and the production build. The new six-case recovery
suite first reproduced three failures against the old whole-card override.

The signed canonical development build launched with the existing designated
requirement. Its settings and control-panel error/retry surfaces remained usable.
The complete-disappearance report remains unconfirmed; do not describe this
limited visual correction as a proven fix for every transparent-window case.
