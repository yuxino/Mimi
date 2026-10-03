# Common regressions and local-test rules

Read this before launching, packaging, installing, or debugging macOS system
prompts. The visible app name and version are not enough to establish identity.

## macOS app identities

| Use | Canonical app | Bundle identifier | Signing identity |
| --- | --- | --- | --- |
| Pre-push development and UI checks | `/Applications/mimi-dev.app` | `app.yuxino.mimi.dev` | `mimi Local Development` |
| Local release-shaped bundle | `src-tauri/target/release/bundle/macos/mimi.app` | `app.yuxino.mimi` | `mimi Local Development` |
| New release pipeline | `/Applications/mimi.app` | `app.yuxino.mimi` | Certificate pinned in `scripts/macos-release-identity.txt` |
| Historical releases through v1.4.1 | `/Applications/mimi.app` | `app.yuxino.mimi` | Ad-hoc (build-specific) |

The current release pipeline uses the same fixed self-signed certificate as
local packaging by default. Older ad-hoc installations have a different
designated requirement: the first fixed-signed update is an intentional
migration and can require one new recording grant. Compare requirements,
not version labels. Keychain continuity is a separate concern below.

Rules:

- Use `./scripts/dev-app.sh` for normal local testing. Do not run `tauri dev`, a
  bare `target/*/mimi` executable, or a copy at a disposable path. The launcher
  uses the incremental `local-dev` Cargo profile and `npm run build:dev` (embedded
  frontend with source maps, without minification or a repeated type/icon check).
  `./scripts/check.sh` still runs all checks and the production frontend build.
  The first `local-dev` build creates a separate cache; later builds reuse it.
  `CARGO_TARGET_DIR` selects both the binary and temporary bundle location.
  A fresh cache builds the existing SpeexDSP echo-cancellation dependency and
  needs CMake on `PATH`, or an explicit `CMAKE=/absolute/path/to/cmake`.
- `./scripts/package-app.sh` creates a local QA package without updater
  artifacts. `./scripts/prepare-macos-release.sh` creates public artifacts on
  the signing Mac with the pinned certificate. CI adds the existing updater
  signature after verifying the app. The
  code-signing private key stays in that Mac's Keychain; CI verifies the
  prepared draft assets. See [release signing](macos-release-signing.md).
- Before replacing a formal app, run
  `./scripts/verify-macos-install-identity.sh NEW_APP /Applications/mimi.app`.
  A mismatch fails closed. `MIMI_ALLOW_IDENTITY_CHANGE=1` is reserved for a
  deliberate, one-time certificate migration whose extra prompts are expected.
- Never use ad-hoc signing for local QA or new public releases. Missing or
  changed identities fail closed. Never use `tccutil reset`, delete Keychain
  entries, or rotate a certificate as a routine fix.
- Branch and pull-request CI compiles macOS with `--no-bundle`; tag CI verifies
  the prepared macOS assets and publishes only after both platforms pass.
- Keep only one live mimi copy while testing. Confirm its executable path, not
  just the process name, before diagnosing shortcuts, windows, or permissions.
- All worktrees install to the same development path. A later launch from
  another worktree can replace the package, including with an older UI-only
  build, even when the application name, version and signing identity match.
  Coordinate canonical installs during acceptance; the install lock does not
  reserve the app for the rest of a testing session. If controls disappear,
  inspect the running window's mode, package modification time and build log,
  then relaunch the intended worktree. Do not reset preferences or permissions
  to repair a package mismatch.

## Know which prompt appeared

For routine macOS API-key testing, the fixed dev launcher supports an explicitly
isolated, private read-only file mode. See [local development credentials](local-dev-credentials.md)
for setup, strict 0600 validation and returning to Keychain. This avoids only
provider-key Keychain reads; signing-private-key and audio permissions still
apply. Production credentials remain OS-backed. Do not weaken Keychain ACLs to
avoid development prompts.

Before a normal dev acceptance run, check whether the documented private `.env`
exists and has mode `0600`, without printing its contents. A missing file selects
Keychain; rebuilding the dev bundle does not recreate it. If file mode was already
configured, investigate the existing local setup before asking for another system
authorization. Never copy a key from an unrelated project or put it in a command,
test report, or repository file.

These prompts have different causes and fixes:

- **Screen & System Audio Recording:** TCC compares the bundle identifier and
  designated requirement. A changed certificate requires one new grant. A
  stable identity at a canonical path must not require repeated grants.
- **API-key Keychain access:** the running app is reading a saved provider key.
  A normal startup reads the profile key once and caches the result. Migration
  tombstones and legacy slots are read only when the profile key is missing or
  during an explicit save/delete/migration. Keep the same service/account and
  update its value in place: deleting and recreating it discards accumulated
  access rules and creates a crash window in which the secret can be lost.
- **Code-signing private-key access:** `/usr/bin/codesign` is using the private
  key for `mimi Local Development` while packaging the app and DMG. This is not
  API-key access. Grant persistent access only when the dialog names that exact
  private key and tool; do not automate a login-keychain password or widen the
  whole keychain ACL in build scripts.
- **Gatekeeper / Open Anyway:** the fixed self-signed GitHub package is not Apple-notarized. This is separate from capture and Keychain authorization.

The local development certificate is self-signed and has no Apple Team ID. It
provides a stable requirement for local and newly prepared release TCC
identities; historical ad-hoc signatures were build-specific. The file-based Keychain also applies a partition
check that can fall back to the build's CDHash. Therefore:

- eliminating the duplicate migration-item read reduces a normal startup to
  one API-key authorization after an identity migration;
- do not promise that a rebuilt self-signed local binary will never ask for
  Keychain access again;
- do not solve this by deleting/recreating a credential, using an allow-all
  ACL, scripting the login password, or assigning a made-up Team ID. Those
  approaches either lose data or weaken code identity;
- password-free Keychain continuity across binary updates requires an
  Apple-issued signing identity with a stable Team ID. Moving to Developer ID
  is an explicit distribution migration, not a local debugging workaround.

When a system prompt repeats, compare the old and new requirements first:

```bash
codesign --display --requirements - /Applications/mimi.app 2>&1
codesign --display --requirements - /path/to/new/mimi.app 2>&1
```

Do not inspect or reset the TCC database as a first response. System logs may
be used only for content-free labels, timestamps, bundle identifiers, and code
requirements; never log recognized text, translated text, credentials, or
audio.

### Recording is enabled in Settings but capture is still denied

After an intentional switch between an old ad-hoc build and a fixed-signed
build, an enabled entry in System Settings alone does not prove that the
current binary can capture. Establish the exact executable path, verify its
signature, and compare it with the intended package before changing grants.

If a normal restart and the current app's permission grant still produce
`System audio capture permission was denied.`, use one bounded recovery in
**System Settings → Privacy & Security → Screen & System Audio Recording**:

1. Quit the current Mimi normally and confirm no other Mimi copy is running.
2. Select the existing **mimi** recording entry and verify the selection before
   using Remove. Do not remove `mimi-dev` or another application's entry.
3. Use Add to select the verified `/Applications/mimi.app`. Check the complete
   path and version in the file picker; similarly named backups are not the
   installed app. Let the user complete any system authentication themselves.
4. Confirm that the newly added Mimi entry is enabled, launch that same app,
   and test actual system-audio subtitles with non-sensitive test speech.
   Verify pause/resume/stop and restore any temporary UI settings.

This is a single-app recovery through the normal settings UI, not a reason to
run `tccutil reset`, edit the TCC database, re-sign the public app, or repeat
permission changes indefinitely. Stop and preserve the exact failure if this
one recovery does not work. Keep Keychain authorization separate: do not
delete or recreate credentials while repairing recording access.

This sequence restored capture for the public v1.3.9 package after an earlier
locally signed installation; toggling the existing grant and adding the app
without removing that old entry had not restored capture. An enabled switch
or a Listening label is insufficient evidence: check the actual session and
translated output. Record only timing/counts/status, never speech or subtitles.

## Overlay and UI checks

For shared layout, notification choices and the cross-page review checklist,
read [UI consistency and feedback](ui-guidelines.md). A fix to one reported
page must include a review of other instances of the same pattern.

- Moving a control between windows also requires updating its Tauri command
  permissions. Settings audio switches once invoked a command authorized only
  for the floating panel, so browser tests passed while native clicks failed.
  Check the actual calling windows and exercise both in the signed app.
- Searchable popups must scroll their result list directly. `scrollIntoView`
  can scroll a clipped ancestor in WebKit and hide rows below the search field;
  pointer hover must not move the list. Check a long list and keyboard search.
- Language menus in settings, the subtitle controls and the tray use the full
  `sourceLanguagesForSettings` route catalog and the shared `LanguageSelect`.
  Keep the same choices, order and localized names; use search/scrolling for a
  long list instead of introducing a second list of preferred languages.
  Verify both translated and Original routes and a stale capability snapshot.
  Explicit source selection retains the target subject to provider normalization;
  Chinese must not silently switch translation off or rewrite an Original target.
- AppKit window mutations, including window level and collection behavior,
  must run on the macOS main thread.
- Full-screen visibility requires the overlay's all-spaces and full-screen
  auxiliary behavior as well as the intended window level. Test above a real
  full-screen app, not only a maximized window.
- Immersive mode owns its complete state: locked overlay position, hidden
  recognition pill, background treatment, and hidden scrollbar. Toggling it
  must restore the prior normal-mode interaction state.
- Streaming drafts are replaceable previews; finals are durable. During
  cross-language translation, delayed or absent translations must not reveal
  source recognition as fallback. Keep translated history and the activity
  status visible; source previews belong only to Original/same-language mode.
  Changing preview kind or removing a preview must immediately invalidate the
  text stabilizer's old display value.
- Keep accessibility state in `aria-*`, but drive changing selected/checked
  visuals through explicit React class names. macOS WKWebView has previously
  left attribute-selector styling stale after the underlying state changed;
  verify the selected class visibly moves in the signed development app.
- Category navigation needs a stronger boundary. Do not depend on a dynamic
  `[hidden]` selector or leave inactive panels mounted: WKWebView has shown
  stale pixels from both the previous panel and the previous selected item.
  Mount only the active panel and remount the compact category navigation when
  the category changes. Verify that the highlighted category and visible
  heading agree after the initial settings snapshot arrives.
- Transient settings feedback must also listen to native Tauri window blur and
  close events. WKWebView may retain DOM focus when the macOS window loses
  focus; DOM blur/visibility alone is insufficient. Verify app switching and
  closing/reopening the settings window in the signed bundle.
- Use `./scripts/dev-app.sh --ui-only` for visual states that do not require a
  provider. UI-only mode must never read Keychain items, open provider sockets,
  or start system-audio capture.

## Native exit and acceptance evidence

- macOS's predefined Quit invokes AppKit termination directly; in the locked
  Tao runtime it can bypass `ExitRequested`. Keep application-menu/Cmd-Q and
  ordinary Dock Quit on the shared stop/finalize path, not an asynchronous
  cleanup task launched after `RunEvent::Exit`.
- The public termination delegate wrapper retains and forwards to Tao's
  original receiver. Keep Dock visibility on Mimi's direct AppKit activation
  policy; Tao's runtime `set_dock_visibility` reads a private ivar from the
  current delegate and is incompatible with the wrapper. See
  [the termination design](../plans/2026-10-02-native-quit-finalization.md).
- The dev bundle is named `mimi-dev.app`, while its executable may be named
  `mimi`. Read `codesign -d -r-` and match the actual bundle executable path
  when verifying processes; a `mimi-dev` binary-name filter can falsely report
  that no instance exists.
- A queued UI-state broadcast can be lost when normal exit completes before
  its 60ms coalescing delay. Verify stop completion through content-free
  lifecycle evidence and process exit, not an old UI-test state marker alone.
- A frontend settings deadline cannot cancel a native Keychain authorization
  wait. Keep real-provider acceptance pending until OS authorization finishes;
  never use credential-free UI fixtures as proof that provider audio works.

## Before handing off

Run `./scripts/check.sh`. For signing changes, additionally build with
`./scripts/package-app.sh`, verify the bundle, and compare its designated
requirement with any app that would be replaced. Before a commit, inspect the
diff for credentials, recordings, subtitle content, personal paths, build
artifacts, and signing material.
