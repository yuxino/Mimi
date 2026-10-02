# Android first-run guide (#82)

Stacked on capture-health #76 (`2471629`). Only Android code is changed.

Five short, reopenable bottom-sheet pages guide service choice, locally saved
credentials, overlay/playback permissions, actual audio and a rendered caption.
Saving never invokes a provider. Permission inspection never starts a paid session;
only the explicit trial action requests per-session MediaProjection and connects.
Skip dismisses the guide without completing it. Returning rechecks permissions and
current-session observations. Historical completion does not substitute for new proof.

Completion requires nonzero PCM passed to the selected provider and a nonempty
translation drawn by the real overlay in the same capture generation. Preview and
hidden text never count. Stop clears session evidence. No text or keys are retained
in evidence. Existing mimi brand image remains a replacement slot for approved poses;
unconfirmed character sheet is not shipped.

Help links reuse desktop's eight-provider public official research (2026-09-30),
placed beside credentials. No bypass of DRM, source opt-out or voice-call policy.

Verification uses synthetic configuration and emulator UI; it does not claim a live
provider caption or real Android 10/14/15 Bluetooth capture.

## Recovery and verification

An existing encrypted settings file is detected before settings initialization,
so upgrades do not force the first-run dialog even without a saved provider key.
Rotation restores the selected provider and page; every restore reads live grants.
Encrypted storage errors remain local, show a retry hint and never fall back to
plaintext. Permission inspection is guarded against repeated taps. Screen-off,
lock and permission revocation stop the capture session.

Android verification: debug/release unit tests (39 each), lint, both APK variants
and instrumentation APK. The API 35 Pixel 7 emulator uses only a synthetic key
that is never sent. The first-run instrumentation visits actual overlay and audio
permission requests, declines them, grants via test shell commands, revokes the
overlay grant, cancels real MediaProjection consent, checks keyboard focus and
checks upgrade suppression. App data is cleared between runs. This proves native
UI recovery, not a real provider session. Positive caption completion is covered
by the isolated evidence model; no live provider-caption claim is made. Real
Android 10/14/15 devices, Bluetooth and actual provider authentication are untested.

## Android follow-up (2026-10-02, #82 / #78)

Both the appearance switch and floating subtitle panel share the same first-use
immersive explanation. The mode stays off until acknowledgement; Cancel, Back,
and closing the host leave it off. A pending explanation absorbs repeated taps,
uses the existing overlay permission in the service, and is dismissed when its
Activity or service is destroyed. Once acknowledged, entering again is immediate.
The persistent acknowledgement records reading the help, not capture or caption
completion. The existing draggable exit control remains available.

A valid new MediaProjection result clears the previous cancellation marker.
Normal stop then reports that the next session still needs consent, instead of
showing an old cancellation. This does not cache or reuse projection tokens.
Diagnostic copying reports clipboard failures with a short retry message and
never includes the exception. OpenAI setup help now links to its official
Realtime Translation guide, matching the Android translations endpoint.

The instrumentation runner accepts `-e immersive_help true -e locale zh|en|ja`
for both real UI entries, cancellation, duplicate taps, acknowledgement, repeat
entry, the overlay exit and pending-dialog service teardown. It uses the real
floating window with synthetic preview captions and never opens a provider or
starts playback capture. Existing first-run permission-denial/revocation checks
remain separate from this preview. Physical devices, actual provider captions
and Bluetooth/wired capture continue to be tracked by #78.

### Follow-up verification

On the isolated Android 15 / API 35 Pixel 7 emulator (1080 × 2400, density
420), the unchanged `de347cf` baseline fails the new settings-first-immersion
regression because it enables the mode before acknowledgement. The corrected
settings and overlay entries pass in Chinese/light, English/dark and Japanese/light,
including actual screenshot contrast checks for the title and action labels.
The dialog surface uses existing neutral colors resolved from the dialog's own
night configuration after it is shown; the service's initial configuration can
differ from the dialog's configuration.

The three-language first-run checks also pass (17 screenshots each): skip/reopen,
missing/synthetic saved credentials, keyboard focus, real overlay/audio refusal,
overlay revocation, MediaProjection cancellation, stopped-sharing recovery and
existing-install suppression. Home, settings, service editors, keyboard and
diagnostic clipboard smoke pass in light and dark (8 screenshots each). No real
provider credentials, live capture or paid sessions are used. The accepted
immersive screenshots and baseline use the same emulator/display and synthetic
configuration; status-bar times differ.

- [Before: settings enable before confirmation](../screenshots/android-first-run/before-immersive-settings.png)
- [After: settings wait for confirmation](../screenshots/android-first-run/after-immersive-settings.png)
- [After: dark overlay explanation and exit guidance](../screenshots/android-first-run/after-immersive-overlay-dark.png)

Debug and release each pass 40 unit tests and lint with zero errors; both APK
variants and the instrumentation APK build. The canonical repository check
passes 800 Rust tests (1 ignored), 721 frontend tests, strict Clippy, formatting
and production build. Android lint retains 13 warnings per variant; frontend
lint retains the SoftwareUpdate Fast Refresh warning. Successful MediaProjection
consent followed by a live provider caption, physical-device routes and clipboard
service failure injection were not exercised.
