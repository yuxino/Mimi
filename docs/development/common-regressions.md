# Common regressions and local-test rules

Read this before launching, packaging, installing, or debugging macOS system
prompts. The visible app name and version are not enough to establish identity.

## macOS app identities

| Use | Canonical app | Bundle identifier | Signing identity |
| --- | --- | --- | --- |
| Pre-push development and UI checks | `/Applications/mimi-dev.app` | `app.yuxino.mimi.dev` | Dev-specific local pin, otherwise `mimi Local Development` |
| Local release-shaped bundle | `src-tauri/target/local-package/bundle/macos/mimi.app` | `app.yuxino.mimi` | Formal-specific local pin, otherwise `mimi Local Development` |
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
- Serialize `check.sh`, `dev-app.sh`, `prepare-macos-release.sh` and other
  frontend builds in one checkout. They write the same `dist` directory even
  when Cargo target directories differ. Concurrent writes can remove a CSS or
  JavaScript asset while Tauri embeds it; finish the first build before retrying
  the second, without clearing shared caches.
- `./scripts/package-app.sh` creates a local QA package without updater
  artifacts, using the `local-package` profile with ThinLTO and parallel code
  generation. Public builds retain the size-focused `release` profile. Set
  `CARGO_TARGET_DIR` to choose the QA output root.
  `./scripts/prepare-macos-release.sh` creates public artifacts on
  the signing Mac with the pinned certificate. CI adds the existing updater
  signature after verifying the app. The
  code-signing private key stays in that Mac's Keychain; CI verifies the
  prepared draft assets. See [release signing](macos-release-signing.md).
- Before replacing a formal app, run
  `./scripts/verify-macos-install-identity.sh NEW_APP /Applications/mimi.app`.
  A mismatch fails closed. `MIMI_ALLOW_IDENTITY_CHANGE=1` is reserved for a
  deliberate, one-time certificate migration whose extra prompts are expected.
- If a restricted executor reports `CSSMERR_TP_NOT_TRUSTED`, repeat the exact
  read-only identity check with permitted access to macOS security services
  before diagnosing a certificate failure. During v1.5.15 preparation, the
  restricted check failed twice while the same check outside that boundary
  passed without changing the app or certificate. This comparison establishes
  an execution-boundary difference, not its underlying system cause. Preserve
  the pinned identity and trust settings; an unrestricted failure still blocks
  replacement. Do not bypass verification or change trust to suppress the error.
- After an explicitly approved local signing migration, keep the public
  certificate fingerprint in `local-codesign-identity.txt` under that app's own
  config directory: `app.yuxino.mimi` for formal packaging and
  `app.yuxino.mimi.dev` for development. `dev-app.sh` selects the development
  scope; it must never inherit a formal app's pin. The optional
  `MIMI_LOCAL_CODESIGN_IDENTITY_FILE` and `MIMI_DEV_CODESIGN_IDENTITY_FILE`
  overrides also apply only to their respective scope. `MIMI_CODESIGN_IDENTITY`
  explicitly overrides either scope. An absent pin uses the unique stable
  self-signed identity; an invalid, unavailable or ambiguous pin fails closed.
  Complete designated-requirement checks still reject an unintended replacement.
  Never use `MIMI_ALLOW_IDENTITY_CHANGE=1` just to make a routine build pass.
  These files contain no private key or API credential and do not change the
  public-release certificate policy.
- Never use ad-hoc signing for local QA or new public releases. Missing or
  changed identities fail closed. Never use `tccutil reset`, delete Keychain
  entries outside the verified migration, or rotate a certificate as a routine fix.
- Branch and pull-request CI compiles macOS with `--no-bundle`; tag CI verifies
  the prepared macOS assets and publishes only after both platforms pass.
- Keep only one live mimi copy while testing. Confirm its executable path, not
  just the process name, before diagnosing shortcuts, windows, or permissions.
- With CUA, do not query the app after clicking Quit: an app-state query can
  relaunch it in ordinary mode. Quit without a follow-up app observation, then
  verify that the exact executable's process exited with a read-only process
  check; a successful AX click alone does not prove shutdown. Use the explicit
  launcher mode before querying the next test instance.
- All worktrees install to the same development path. A later launch from
  another worktree can replace the package, including with an older UI-only
  build, even when the application name, version and signing identity match.
  Coordinate canonical installs during acceptance; the install lock does not
  reserve the app for the rest of a testing session. If controls disappear,
  inspect the running window's mode, package modification time and build log,
  then relaunch the intended worktree. Do not reset preferences or permissions
  to repair a package mismatch.

## Apple Speech build and resource boundaries

Apple silicon macOS builds require Xcode 26 or later for the Swift Speech adapter.
The app deployment target remains macOS 13, with macOS 26 and actual
`SpeechTranscriber.isAvailable` checks before every supported entry point. Intel,
Windows and Linux use an unavailable implementation and hide the add-provider choice.
Do not remove availability checks or silently ship an Apple silicon build with the
adapter omitted because its SDK is old.

When worktrees share a Cargo target directory, switching back from a branch without
the Swift adapter can leave a stale build-script output. If linking reports missing
Apple bridge symbols, inspect the current build output for the Swift link/search
directives and rerun this branch's build script (for example, touch `src-tauri/build.rs`).
Preserve reusable caches and signing identity; do not clear global caches or reset TCC.

Language support and installed resources are queried from the running app. A successful
standalone probe does not establish the app's asset state: prepare through the explicit
settings action under Mimi's actual identity, then query again. Querying capabilities or
starting recognition must not trigger hidden downloads. The adapter accepts already
captured PCM; it must not request microphone access for a system-only session.

## Apple Translation readiness and language identity

Translation resources are separate from SpeechTranscriber resources. A global
installed-language result does not establish that the current translation session
is ready. The installed-only bridge awaits session readiness and forbids downloads;
only the explicit settings preparation action opens Apple's confirmation UI.
Leaving that UI before preparing must keep the pair unready and allow retry.

Treat language aliases and scripts at the owning adapter boundary. A Simplified
Chinese selection must not fall back to a Traditional Chinese recognition locale
and then report the source as Simplified Chinese. Explicit regional variants need
an exact runtime match; a base language does not certify every regional variant.
Keep provider mixed-language modes out of same-language passthrough checks, including
floating subtitle projection, or the translation can disappear despite a valid
server response.

## ScreenCaptureKit native property types

Use the Apple SDK and Objective-C runtime method signatures when checking native
configuration accessors. In `screen-capture-kit` 0.7.1, `sampleRate` is bound as
`f64`, although the native property is `NSInteger`. Use Mimi's integer setter
adapter; a wrapper call completing without an exception does not prove that the
native configuration received 16000 or 24000. The configuration-object regression
checks both rates without starting capture or touching TCC.

For a video with no subtitles, first check the actual sent-audio WAV and capture
format diagnostics. A playing, decoded, unmuted browser video is not proof of
nonzero system output. Entirely zero sent PCM cannot be scored as missed speech.
Keep failed cases, compare a bounded authorized capture target, and distinguish
configuration, source output, capture input, decode and provider evidence. Do not
reset recording grants or expand an application filter from that observation
alone.

Browser tab/Space mute is separate from the media element's `muted` and `volume`
properties. Ego's [changelog](https://www.egolite.ai/changelog) documents muted
agent-created task tabs. Check the output boundary separately; extracting
nonzero audio from an element does not prove the browser played it to the OS.
If using a verified local clip as a playback control, record that source change
and do not claim a causal before/after fix from it.

## Know which prompt appeared

Production and ordinary development credentials use private local files on all
desktop platforms. Legacy OS items are read only during automatic upgrade import,
then deleted after durable write and read-back verification. Completed imports
must never fall back to the OS store, even if the local file later goes missing.
See [local credential storage](../plans/2026-10-04-local-credential-storage.md).
The optional read-only dev `.env` presets remain separate; removing that file
returns to editable local profiles. Signing-private-key and capture authorization
are independent of provider credential storage. Never weaken native ACLs.

These prompts have different causes and fixes:

- **Screen & System Audio Recording:** TCC compares the bundle identifier and
  designated requirement. A changed certificate requires one new grant. A
  stable identity at a canonical path must not require repeated grants.
- **Legacy API-key Keychain access:** only the one-time importer reads an old
  saved provider key. Verify a durable local copy before deleting its original
  OS item. Checkpoint pending cleanup independently, so interrupted deletion
  resumes without rereading the old secret. Routine startup, snapshots,
  switching and credential edits use only the local file. On Windows, reading
  an old Enterprise credential must not rewrite it as Local. Native migration
  tests must verify the local file copy, retirement of the old item, and no
  recreated native item after restarting or saving an edited key.
- **Code-signing private-key access:** `/usr/bin/codesign` is using the private
  key for `mimi Local Development` while packaging the app and DMG. This is not
  API-key access. Grant persistent access only when the dialog names that exact
  private key and tool; do not automate a login-keychain password or widen the
  whole keychain ACL in build scripts.
- **Gatekeeper / Open Anyway:** the fixed self-signed GitHub package is not Apple-notarized. This is separate from capture and Keychain authorization.

The historical OS-credential implementation needs the following distinction.
The local development certificate is self-signed and has no Apple Team ID. It
provides a stable requirement for local and newly prepared release TCC
identities; historical ad-hoc signatures were build-specific. The file-based Keychain also applies a partition
check that can fall back to the build's CDHash. Therefore:

- avoid secret reads in all macOS settings snapshots, not only unselected
  profiles: the earlier inactive-only fix still prompted when users switched
  keys. Test repeated selection of every saved profile and zero secret reads;
  actual credential use must still validate and authorize required slots;
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

## Apple Speech resources can exist while the module check fails

On the signed `7885ede9` development app, a Japanese resource preparation found
an existing system ASR asset, then the framework's anonymous XPC endpoint failed
with Cocoa 4099 during the post-install status checks. Do not turn this into a
network diagnosis or collapse `unsupported` and `supported` into "not downloaded".
Record the native status and failed stage without error descriptions, content or
paths. A same-identity development rebuild with diagnostic changes only was
reopened and its Japanese recognition setup check succeeded in 540 ms; this is
recovery evidence, not proof of a permanent OS fix or full audio recognition.
Keep unconfirmed resources out of all ready-language lists, invalidate earlier
recognition results when resource work starts, and offer a read-only refresh.
Do not reset TCC, change signing or delete system assets without evidence.

## Overlay and UI checks

For shared layout, notification choices and the cross-page review checklist,
read [UI consistency and feedback](ui-guidelines.md). A fix to one reported
page must include a review of other instances of the same pattern.

- Subtitle confirmation time follows its own display preference for every audio
  input. Do not gate it on microphone selection or source-icon visibility. Test
  system, microphone and both inputs explicitly in normal and immersive views;
  the Timeline default input can otherwise hide a system-only regression. Keep
  the Rust/TypeScript minimum-height contract in sync with the metadata row.
- The multilingual label repair in `dc2143f` did not cover the Windows-only
  output selector. Issue #132 put the selector and idle sentence in one inline
  wrapper: its intrinsic width squeezed Chinese labels into one-character
  lines and misaligned the picker even in a maximized window. Keep status out
  of the control column, bound long device names, and test platform-only controls
  with explicit fixtures on all three interface languages. An idle instruction
  belongs in help; missing-device and live-capture states stay visible below.
- The first #132 preview still let adjacent 36 px pickers touch: the shared
  section body had no gap. Preserve explicit spacing between sibling rows and
  assert the rendered gap in the same geometry regression. Inspect the actual
  screenshot before delivery; correct label width does not prove visual quality.
- Windows CJK glyph fallback cannot be inferred from a macOS screenshot or a
  successful CSS `font-family` assertion. Keep Chinese sans families ahead of
  Japanese fallbacks outside Japanese UI, include `Microsoft YaHei` as well as
  `Microsoft YaHei UI`, and verify the actual installed font on the affected
  Windows image before claiming its serif-font report resolved.
- The #132 screenshot review also missed a lone shield below the configuration
  list. It was storage help, but its detached position gave no clue what it
  described. Attach it to the configuration count, and inspect diagnostics,
  profile headings, credential toolbars, proxy rows and confirmation previews
  for the same pattern. Help needs a visible context, not its own empty row.
- The same review missed the saved-key eye below the translation field. Its
  action text existed only for screen readers, so DOM text tests passed while
  the visible action was ambiguous. Make the text visible in all three languages,
  share the field's label line, and check actual caption dimensions plus preview
  states. Do not populate a replacement draft with a revealed saved credential.

- Moving a control between windows also requires updating its Tauri command
  permissions. Settings audio switches once invoked a command authorized only
  for the floating panel, so browser tests passed while native clicks failed.
  The same gap recurred when pause/resume was added to the floating panel.
  Check the actual calling windows, add a regression for the command registration
  and that window's permission scope, then rebuild and restart the signed app
  before exercising success and failure. Frontend reloads cannot update its
  compiled Tauri permissions; do not widen unrelated start/stop permissions.
  UI tests with mocked IPC do not prove native capability access; error Retry
  must be exercised in the signed app and shown to reach backend `session_start`.
- Searchable popups must scroll their result list directly. `scrollIntoView`
  can scroll a clipped ancestor in WebKit and hide rows below the search field;
  pointer hover must not move the list. Check a long list and keyboard search.
  DOM focus does not prove a macOS floating NSPanel is key: explicit clicks must
  give the actual WKWebView first-responder status and make the control panel
  key, never main or proactively activating. Wry contentView is a wrapper:
  `canBecomeKey` and `makeKeyWindow` alone do not prove keyboard delivery.
  Use `with_webview` on the main thread, only for an explicit expanded panel.
  Automation may activate the app before typing and hide the real failure.
  Verify a human click from another app into search, then repeat after restart
  and after removing any temporary diagnostics. `isKeyWindow` alone only
  describes application-local key status, not system keyboard delivery.
  Collapsing it must release key status. Before release, type English and Chinese
  searches in the signed native panel and confirm media playback continues.
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
- Error presentation may temporarily expand and unlock the subtitle window, but
  must leave a visible drag handle usable. Keep dragging separate from collapse;
  explicit movement changes position without saving the temporary error height.
  Use the same effective lock state in native hit testing and React top actions.
  Replay a real mouse drag and inspect top controls in an error reached from
  normal, collapsed, locked and immersive states, then verify recovery and saved
  geometry. Text visibility or clicking only the error-body settings button is
  insufficient acceptance. Cover the capsule panel too: controls overridden by
  the error must not accept a change that only takes effect after recovery.
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

## Validate the test audio before blaming capture

In one macOS sandboxed integration run, `say -o` returned success but created
an AIFF with zero audio frames. `afplay` also failed inside the sandbox with
`AudioQueueStart failed (-66680)`. Check `afinfo` for nonzero frames and the
expected duration before playback, then verify playback completes. If the
environment blocks audio services, use an approved local execution path and
validate the regenerated file again. An empty fixture or a failed player does
not establish a Mimi capture or recognition defect. Keep the sample
non-sensitive; do not change permissions or reset TCC to repair the fixture.

See [the integration learning loop](integration-learning-loop.md) for comparable
samples and [the run ledger](integration-runs.md) for this check's exact scope.

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
  In versions before the local-file migration, the initial settings snapshot
  checked credential status for the whole profile catalog. Selecting the private
  dev preset did not isolate ordinary profiles' Keychain reads. A sampled
  `FileSecretStore -> Keyring -> SecKeychainFindGenericPassword` wait identified
  that historical boundary; current snapshots must stay on the local-file path.
  Record an unfinished upgrade import separately, quit normally, and keep UI-only
  or offline replay results distinct from live provider acceptance. Repeated
  frontend Retry cannot cancel an unfinished native authorization read.

## Before handing off

Run `./scripts/check.sh`. For signing changes, additionally build with
`./scripts/package-app.sh`, verify the bundle, and compare its designated
requirement with any app that would be replaced. Before a commit, inspect the
diff for credentials, recordings, subtitle content, personal paths, build
artifacts, and signing material.

## Configuration input and development-mode acceptance

- Before replacing the canonical development app, identify the source revision
  and any unpublished provider support in the running build. A stable signature
  does not make an older provider catalog parser compatible. A main-based build
  without the local Apple Speech trial rejected that trial's saved catalog and
  showed unavailable fallback profiles. The catalog stayed protected by the
  write-blocked path; recover with the compatible source instead of recreating
  configurations. Preserve the existing signed bundle until ordinary startup
  verifies the expected configurations, and use an isolated combined preview
  when testing an unrelated change alongside an unpublished provider trial.
- A long placeholder cannot be selected or edited. Load saved nonsecret fields
  into their real input value and use concise empty-field hints. Check the full
  value and actual editing, not just absence of page overflow. Persistent text
  configuration uses the shared wrapping expansion; secrets keep their explicit
  reveal boundary.
- macOS WebKit mouse activation may blur an input with no related target before
  an inline button gets focus. Replacing that input with a textarea can therefore
  trigger proxy auto-save/validation during expansion. Preserve focus through
  the shared expansion action and verify a dirty draft by real mouse activation;
  a DOM test that focuses the button first does not reproduce this path.
- Set UI-test and automatic-start flags explicitly on every canonical dev launch.
  A normal launch was observed with a residual UI-test flag despite omitting
  `--ui-only`; its relaunch source was not proven. Verify the exact running bundle
  and runtime/window mode, not only the script's success message. After UI-only
  QA, restore the ordinary idle development app before handing it to the user.

## Gemini rotation and continuous transcripts

Stage rotation audio by a finite PCM-byte bound. A native callback can be 20 ms;
a fixed count of callbacks does not establish a two-second buffer. Do not hold
the audio sender behind the old transcript drain. Exercise the real
AudioSendPipeline with byte order, padding, Stop, Clear, cancellation and the
staging cap. Replacement readiness and first subsequent transcript are separate
measurements; a successful socket switch does not prove complete translation.

Keep Gemini fragment/cumulative assembly in shared Rust and assert drafts through
actual JNI as well as Rust fixtures. Preserve raw spaces and subwords, and do
not deduplicate by matching tails. Sentence breaks belong only to live display,
never to transcript buffers or committed history. Keep preview clocks separate
from local quiet checkpoints.
