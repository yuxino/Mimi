# Mimi for Android

Floating translated subtitles for app playback, your microphone, or both. Watch a video,
follow a class, or translate a conversation with the original text and translation on screen.

[English](README.md) · [简体中文](README.zh-CN.md) · [Desktop home](../README_EN.md)

<a id="start"></a>

## Start here

1. **Install Mimi.** [Download the APK](https://github.com/yuxino/mimi/releases/latest) for Android 10 or later.
2. **Add a service.** Open **Services → Add**, choose a service, enter its credentials, and save. The first configuration is selected automatically. The [setup guide](../docs/provider-setup.md#choose) explains which service to choose and where to get a key.
3. **Start captions.** In **Captions**, choose the audio input and languages, then start. Allow display over other apps and audio recording. If you selected system audio or both inputs, also accept Android's playback-capture prompt.
4. **Play something.** Switch to a video, stream, or other app and play speech. Subtitles appear over it. Immersive Mode leaves just the text and lets touches pass through.

Mimi is free. Cloud services may charge for API usage: recognition sends your selected
audio to the service, and remote text translation sends the recognized text.
Set up your credentials on the phone; they do not sync from the desktop app.

## Using subtitles

- **Audio input:** system audio is selected by default. Choose **Microphone** or **Both** to translate your voice. Both keeps the inputs separate and labels their captions. Stop before switching inputs. System-only capture never opens the microphone, but Android still requires audio-recording permission for playback capture.
- **No subtitles:** make sure captions are running, a saved service is selected, and speech is playing. Some apps block playback capture; protected audio cannot be captured. Android asks for playback-capture consent each time you start system audio.
- **Recent captions:** new installs keep the three most recent confirmed captions in memory; existing installations keep their previous setting. You can change that limit. Stopping leaves these captions readable; the next session clears them.
- **Saved sessions:** **Save captions** is off by default. Turn it on before starting to write a private local transcript. Open **Saved sessions** to read it, export TXT/Markdown, or delete it. Turning saving off deletes the current session's saved content; older sessions need to be deleted separately. Android does not export recorded audio.

Need to change the text translator? See [independent text translation](#independent-text-translation).
Android has no Apple Speech or custom recognition setup. See [known limits](#known-limits)
for other differences, or [report an issue](https://github.com/yuxino/mimi/issues).

<details>
<summary>Feature details and implementation</summary>

Native Kotlin UI and audio capture use the shared Rust subtitle and provider
runtime through JNI. Credentials are configured separately on Android. Shared
code does not prove availability on every device; see [verification status](#verification-status).

## Features

- **Main interface** — Captions, Services and More share one page shell and a
  persistent status/Start/Stop footer. Captions contains language choices, one
  live/sample preview, display mode and immersion. Width, font, color and
  opacity are directly adjustable beside the pinned preview on that same page. Services owns service
  selection and setup; More contains interface language, diagnostics and the
  setup guide. Switching sections retains their scroll positions.
- **System-audio capture** — `MediaProjection` + `AudioPlaybackCapture`
  (API 29+). Requires Android’s `RECORD_AUDIO` runtime grant for playback capture;
  never uses a microphone source unless selected. Captures what other apps play
  (`USAGE_MEDIA/GAME/UNKNOWN`), 48 kHz stereo float, box-filter downmix +
  resample to the provider's target rate (16 kHz / 24 kHz mono PCM16).
- **Microphone and Both** — explicit microphone capture uses a separate mono AudioRecord. Both keeps independent capture buffers, recognition sessions and source-labeled captions; audio is never mixed and a failed input never silently falls back. Stop before changing the input. Microphone-only does not request screen capture permission.
- **Services** — Alibaba Cloud, OpenAI, Google Gemini, Azure OpenAI, Volcano Engine,
  Tencent Cloud, Baidu and xAI. Alibaba, OpenAI, Gemini and Azure offer **ASR + MT**
  with a separate text translator; xAI offers streaming recognition or Grok Voice.
  See the [service setup guide](../docs/provider-setup.md#choose) for modes and credentials.
  Production clients are shared with desktop. Each configuration keeps its own encrypted credentials.
- **Configuration** — save multiple named configurations, including more than one for the same service. Each keeps its own credentials and text translator. Tap a configuration to select it; Settings edits it without switching. Duplicate and Delete are in the editor’s More menu. Stop captions before changing configurations. Azure requires a
  resource endpoint and transcription deployment; simultaneous translation also needs a translation deployment. Tencent requires AppID, SecretID and
  SecretKey; Baidu requires AppID and AppKey. Other services use an API key. Built-in service models,
  endpoints and language capabilities follow the shared factory. Stored secrets are not filled back into the editor;
  leaving a secret field empty preserves its saved value. Changes are written only with Save; backing out of a new editor adds nothing. Existing service settings migrate automatically.
- **Subtitle overlay** — one `TYPE_APPLICATION_OVERLAY` reading panel with
  current source and translation. Drag its top handle to move it and its corner
  to resize width and height. Confirmed lines appear only when bounded history
  is enabled; history scrolls inside the chosen height. Main and floating controls
  offer original/translation/bilingual display, left/center/right alignment,
  width (55–100 %), height (20–70 % of the safe viewport), background alpha
  (0–90 %), text color presets, font size and whole-window opacity.
  Quick settings changes update the actual captions visibly above the editor.
  The centered provider entry shows service brands; tap it to open Services and
  inspect model details. Narrow panels place it on a separate row above immersion
  and resize actions. Saved background and opacity apply from the first frame;
  changing font size preserves them.
  Before speech and
  during a silent gap, an active session shows the same localized listening
  message as desktop, using the selected typography and color. An optional
  immersive mode shows lighter
  typography and a compact contrast shadow, and passes touches
  through to the app below. A small exit control follows the subtitle position;
  The Captions page also enters/exits immersive mode without stopping the session.
  Returning restores the normal panel's saved size and position.
  Floating captions temporarily hide while Mimi's own pages are open; Captions
  shows the current caption and capture continues. Leaving Mimi restores the
  floating window. Immersive captions pass touches through; the normal reading
  panel receives touches within its covered area.
- **Subtitle previews** — the shared factory selects complete paired previews
  for Audio3 + text translation and sentence-identified realtime previews for
  integrated live clients. New sentence identities never borrow old final lines;
  the current complete pair remains available independently of saved history.
- **Alibaba Cloud** — choose simultaneous translation or ASR + MT. New configurations start with simultaneous translation; existing ones keep their mode. Follow the shared [Alibaba setup guide](../docs/provider-setup.md#alibaba-cloud) for workspace, key and model requirements. Historical built-in custom
  endpoint/model/hotword keys are retained without enabling unsupported overrides.
- **Credentials** — EncryptedSharedPreferences (Android Keystore master key),
  stored separately from profile metadata and isolated per configuration.
- **Subtitle history** — new installations keep the three most recent confirmed
  captions in memory by default. Existing installations keep their previous setting,
  including the former zero default. This bounded display is independent of **Save captions**,
  which is off by default. Saved sessions use the desktop Rust journal and export formatter,
  keep confirmed original/translation pairs and source identity, and survive Stop/restart.
  Disable saving to discard the current session; older saved sessions require explicit deletion.
  Saving must be enabled before Start. Audio recording/export remains desktop-only.
- **Interface language** — More offers Follow system and the same eight interface
  languages as desktop: 简体中文, 繁體中文, English, 日本語, Deutsch, 한국어, Français and ภาษาไทย.
  Changes apply immediately and persist after restart, independently of subtitle
  source and target languages. Android 13+ also exposes this choice in system App languages.
  Unsupported system languages fall back to English.

See the [native Before/After comparison](https://github.com/yuxino/Mimi/pull/211)
for the empty-overlay, live font-size and help-icon repairs. Optional explanations
appear on mouse hover/long press; tap a help icon to read scrollable details.
Setup, permission and audio-recovery instructions remain visible.

</details>

## Requirements

- Android 10+ (API 29)
- Credentials for one supported service (usage may incur provider charges)
- The "Display over other apps" and `RECORD_AUDIO` grants, plus per-start MediaProjection consent for system playback

<details>
<summary>Build and development</summary>

## Build

```sh
export ANDROID_HOME=/path/to/android-sdk
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android i686-linux-android
"$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" "ndk;27.2.12479018"
./gradlew testDebugUnitTest lintDebug assembleDebug
# app/build/outputs/apk/debug/app-debug.apk
```

Requires Rust 1.88+, Python 3, JDK 17, Android SDK platform 35, build-tools
35.0.0 and NDK 27.2.12479018. Gradle builds the actual shared Rust core for JVM
tests and all four packaged Android ABIs; a missing native dependency fails
the build. APK verification checks 16 KB ELF/ZIP alignment and license notices.
See [shared JNI build](../shared/mimi-android-jni/README.md) for the state boundary,
toolchain pins and cache paths. The wrapper pins
Gradle 8.10.2 and verifies its distribution checksum. minSdk 29, targetSdk 35,
Kotlin 2.0, AGP 8.7. CI tests/lints both variants and produces a debug APK and an
unsigned release APK. The unsigned artifact is for signing, not installation;
device capture remains a separate manual check.

Keep the same debug signing keystore when updating an existing development
installation. An alternate `ANDROID_USER_HOME` can select a different debug
key. Before `adb install -r`, compare the installed APK's certificate with the
new APK using `apksigner verify --print-certs`. If Android reports
`INSTALL_FAILED_UPDATE_INCOMPATIBLE`, use the existing matching debug key;
do not uninstall the configured app to bypass the mismatch. Preserve its
encrypted credentials and settings. Release signing remains separate.
CI debug certificates are disposable. For repeated local acceptance, sign both
the debug app and its instrumentation APK with the same existing local debug
keystore, verify their certificates, and keep that identity for later updates.
The `.dev` app ID alone does not provide signing continuity. Never use a release
key for these debuggable packages.

### Signed release

`version.properties` is the Android version source. Increment `versionCode` for
every public update, and keep `versionName` consistent with the release notes.
Build with `./gradlew testReleaseUnitTest lintRelease assembleRelease`, then run
`bash sign-release.sh` with these environment variables set securely:

- `ANDROID_HOME`: Android SDK directory.
- `ANDROID_KEYSTORE_PATH`: existing, securely backed-up release keystore.
- `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD`: its signing credentials.
- `ANDROID_SIGNING_CERT_SHA256`: the expected public release certificate fingerprint.

The script verifies the compiled package/version and rejects debug builds,
aligns and signs the APK, then verifies its signature against the pinned
non-debug certificate. It writes `release/mimi_<version>_android.apk` and
`release/SHA256SUMS.txt`. It never creates a temporary replacement identity.

For GitHub publishing, configure repository secrets `ANDROID_KEYSTORE_BASE64`
(base64 of that same keystore), `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`
and `ANDROID_KEY_PASSWORD`, plus repository variable
`ANDROID_SIGNING_CERT_SHA256`. Push a shared `v<version>` tag after staging the
signed macOS assets as documented in `docs/development/macos-release-signing.md`.
The main release workflow calls `android-release` to test, lint, sign and verify
an APK, then waits for Android and every desktop package before publishing one
complete release. Android's `versionName` must match the desktop version; its
APK is included in the shared `SHA256SUMS.txt`. Existing public assets are never
overwritten. The reusable Android workflow cannot publish a separate release.

Keep the signing key and its backup outside Git. Use the same key for future
updates. The debug app has a separate `.dev` package and can remain installed
alongside the release app. Preserve each app's settings and credentials. The APK uses release
build settings; this does not imply physical-device or all-provider validation.

## Local UI preview

The Android interface reuses Mimi's existing character artwork and neutral
light/dark palette. Subtitle source and target languages can be changed directly on the home screen with Undo. Tapping the
subtitle sample opens appearance settings, where changes are saved automatically
and previewed without capture or a network session. The appearance screen also
previews normal and immersive text. The service tab lists saved configurations. Add opens the service picker; each configuration has a separate editor.
Credentials require explicit Save; changing appearance never saves credentials. Your own service credentials are still required for real translation.

The main page groups Captions, Services and More. Captions offers live
original/translation/bilingual display, alignment, width, height, font, color and opacity controls.
Its preview stays visible while styles scroll, above the controls in portrait
and alongside them on wide screens. The floating style dialog has controls only;
the existing caption window stays visible above it and responds to edits.
Direct color choices wrap to retain 48dp touch targets. The main preview
keeps one latest line per language in a stable viewport, so adjusting width or
size does not move the controls under your finger. Full current captions
remain available in the normal reading panel.
Sessions open that panel unless immersive mode is enabled. Its reading viewport
is bounded by the selected height, and history does not enlarge it. The panel
has drag and two-dimensional resize handles, quick styles and a labeled immersive action.
Display choices do not change recognition or translation requests. A running
session keeps service editing locked with an explicit Stop action.

The debug app uses `app.yuxino.mimi.android.dev` (mimi Dev), with independent
credentials and permissions, and installs alongside the release app. Never
uninstall production to install a debug APK. On an idle development emulator:

```sh
./gradlew assembleDebug assembleDebugAndroidTest
adb install -r app/build/outputs/apk/debug/app-debug.apk
adb install -r app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk
adb shell am instrument -w -e theme light app.yuxino.mimi.android.dev.test/app.yuxino.mimi.android.UiSmokeInstrumentation
adb shell am instrument -w -e theme dark app.yuxino.mimi.android.dev.test/app.yuxino.mimi.android.UiSmokeInstrumentation
adb shell am instrument -w -e overlay_preview true -e theme light app.yuxino.mimi.android.dev.test/app.yuxino.mimi.android.UiSmokeInstrumentation
adb shell am instrument -w -e overlay_interaction true app.yuxino.mimi.android.dev.test/app.yuxino.mimi.android.UiSmokeInstrumentation
adb shell am instrument -w -e interface_language true -e theme light app.yuxino.mimi.android.dev.test/app.yuxino.mimi.android.UiSmokeInstrumentation
adb shell am instrument -w -e overlay_listening true -e locale zh-Hans app.yuxino.mimi.android.dev.test/app.yuxino.mimi.android.UiSmokeInstrumentation
```

The interface-language check requires a blank, idle emulator. It exercises the
native picker, settings recreation and localized overlay controls without capture
or provider requests. Editor screenshots use the normal own-page caption hiding;
overlay screenshots use a separate debug-only host without requesting capture.
Use `-e leave_language zh-Hans`, force-stop the app, then run
with `-e interface_language true -e expected_language zh-Hans` to check persistence.

On API 33+, `overlay_listening` checks service resources against a deliberately
different context locale, the normal native waiting panel,
stable history viewport, immersive restoration and return
from all settings sections. Use `-e locale system` for System. It uses synthetic
session state, starts no capture or provider and restores the app locale and
changed preferences. Include narrow/large-font and wide native layouts.

The checks exercise quick language selection/Undo, direct appearance access,
auto-save with actual touch gestures, all eight service editors, write-only secret fields, rejected incomplete configurations, history clearing and the
keyboard. Changed non-secret preferences are restored in a finally block; provider
drafts are discarded without Save. No provider or audio capture session starts. Screenshots contain sample subtitles and empty key
fields and are written to the app's external `files/ui-preview` directory. Pass
`-e overlay_interaction true` on a blank idle device for actual two-dimensional
resize/drag, live quick settings without a duplicate preview, all alignments,
stable history height, both provider badge cases and repeated immersive restoration.
Quick settings keep the full live caption frame, including its route, provider
badges, controls and current status/content. Those caption actions and gestures
are decorative during editing; closing restores the same window. Limited space
scales the complete frame rather than hiding its toolbar/footer.
The `interaction-*.png` screenshots show the actual panel and edits.
On a blank, idle API 35 device, `-e preview_parity true` compares both preview
entries with the native frame across all eight languages plus System, waiting,
no-sound/connection feedback, long captions, history and appearance endpoints.
Use `-e locale ja` (or another supported tag) for one language. Normalized pixels,
wrapping and frame geometry must match; quick editing preserves frame pixels.
`overlay_preview` also captures long-caption reading and restored normal mode. These
are synthetic native fixtures, not a real provider or audio latency measurement.
Pass
`-e demo true` for a paced walkthrough suitable for emulator screen recording;
it demonstrates the labeled sample, not live translation.

</details>

## Independent text translation

Use this when you want one service to recognize speech and another to translate it.

1. Stop captions and open a configuration in **Services**.
2. For Alibaba Cloud, OpenAI, Gemini or Azure OpenAI, choose **ASR + MT**. For xAI, choose **Streaming speech recognition (ASR)**.
3. Under **Text translation**, choose Grok, DeepL, DeepLX, ChatMock or an OpenAI-compatible service. Fill in that service's settings and save. Recognition and translation have separate credentials; Mimi does not reuse a missing translation key from recognition.
4. Check recognition and text translation separately, then start captions. Choose **No translation (original only)** if you only need the recognized text. Changing the subtitle display to Original only does not stop translation requests.

Follow the shared [text translation guide](../docs/provider-setup.md#text-translation)
for keys, addresses and models. Recognition still uses your selected speech service
and may incur charges. Android does not offer Apple local services or custom speech recognition.

<details>
<summary>Local services and translation implementation</summary>

Existing OpenAI-compatible configurations stay under that entry, including their
saved address, model and key. To use the separate ChatMock entry, select and configure it explicitly.

Use HTTPS for computers and servers, including LAN addresses. With explicit local HTTP
permission, only `localhost`, `127.0.0.1`, `[::1]` and Android emulator host `10.0.2.2`
are supported. On a phone, localhost is the phone itself. For USB development, `adb reverse
tcp:8000 tcp:8000` can make the computer's local ChatMock service available at
`http://127.0.0.1:8000/v1` on the phone. Do not use a ChatGPT password or session token as
Mimi's API key; account login stays in your ChatMock installation.

**Test translation** sends a fixed example through the same HTTP path as subtitles and
shows elapsed milliseconds. It does not save the draft or send system audio. **Save**
commits both stages; back discards changes. Translation keys have independent encrypted
storage and are not reused when the destination changes. Help icons open the requirements
without persistent explanatory paragraphs in the editor.

The final-only translation queue uses the shared Rust policy: one active request, three
waiting requests, up to three attempts and a 45-second total deadline including waiting
and retries. Explicit stop drains accepted work within finite shared grace windows;
errors and permission revocation abort. Overflow rejects new work and drains accepted
work. Late results cannot enter a newer session. These checks do not
prove a live DeepL, DeepLX or ChatMock account or physical Android device until those are tested separately.

</details>

<details>
<summary>Architecture and verification</summary>

## Architecture

```
app/src/main/java/app/yuxino/mimi/android/
  MainActivity.kt            start flow, overlay-permission gate, projection consent
  SettingsActivity.kt        service list and appearance settings
  AppearanceControls.kt     shared native Settings/floating appearance controls
  ServiceSettingsActivity.kt provider-specific write-only credential editor
  ServiceSettingsUi.kt       compact service picker
  SettingsStore.kt           EncryptedSharedPreferences-backed settings
  capture/MimiService.kt     foreground service (mediaProjection type):
                             capture loop + overlay window + auto-hide timers
  resample/StreamResampler.kt  O(1) box-filter downmix + resample to PCM16 mono
  provider/ServiceCatalog.kt  service fields, capabilities and language normalization
  provider/SharedRuntimeEngine.kt production JNI session engine
  provider/ProviderEngine.kt  native session interface
  provider/SharedSubtitleCore.kt stateless JNI bridge to shared/mimi-core
  provider/SubtitleBus.kt      native snapshot facade over the shared Rust reducer
```

Provider clients and scheduling live in `shared/mimi-runtime`, reached through
`shared/mimi-android-jni`. Historical Kotlin transport/protocol classes remain
offline comparison fixtures; production capture never selects them. Run
`python3 -B scripts/check-shared-runtime.py` to verify this boundary.

## Verification status

The updated interface and Alibaba Cloud system-playback translation have been exercised in an Android 15
emulator with Firefox. The bundled Chrome tested there explicitly disables playback capture and yields silent
samples; this finding is specific to that browser package. The additional provider adapters have protocol tests,
including Tencent signing and Volcano binary frames, but have not been verified against live accounts. Do not
interpret build or protocol-test success as physical-device or all-provider acceptance. The original contributor’s
reported Android 17 device test applies to the original port, not every subsequent change.

For real playback acceptance, use the independent debug app with a stable local
signing identity. `LiveAcceptanceChecks` requires native Start and fresh platform
consent; its bounded report stores counts and status, not subtitle content.
An explicitly authorized temporary provider fixture must enter the app's private
cache through stdin, be read back before testing, and be consumed and restored
in the test's finally block. Never put credentials in arguments, environment
variables, screenshots, logs or Git, or treat this fixture as configuration sync.

Disable video autoplay and verify the player's actual 0:00 and end states. The
tested Firefox MediaSession can retain a stale position, so timestamps derived
from it do not establish playback progress. Use visible player times together
with continuous native observations. Do not run a separate `uiautomator dump`
while instrumentation owns UiAutomation; use its own screenshots or a read-only
ADB screencap. Native visual fixtures, emulator frame statistics and a live
provider run establish different evidence; record their exact build and remaining
device/provider gaps in the [integration ledger](../docs/development/integration-runs.md).

`live_caption_reading=true` also accepts `screen_changes=true` with
`phase_seconds=90` or longer. Each display mode checks landscape, immersion,
small/large dimensions and exact portrait restoration while actual audio and
caption drawing continue. Add `browser_fullscreen=true` for operator-controlled
player fullscreen entry/exit instead of forced display rotation: respond to
`CAPTION_FULLSCREEN_REQUEST` using the observed native player controls. Keep
glyph and visible-rectangle comparisons in the same root-window coordinate
system; screen offsets of floating windows must not become clipping failures.
Browser media keys alone do not prove audio, and zero-duration injected taps
can miss Firefox controls; verify the player after a native short press.

</details>

## Known limits

- MediaProjection requires user consent on every start (platform rule).
- DRM-protected output and apps that disallow playback capture cannot be captured.
- Provider VAD can hold sentences open when background music is continuous;
  complete current subtitles remain readable until replaced or the session ends.
- OpenAI and DashScope now use the desktop shared alignment implementations.
  Their provider models and other adapters still require separate live-session validation.
- DashScope sentence finals depend on server-side VAD; continuous speech with
  music may delay whole-utterance finals. Complete text within the shared byte limit is retained.

MIT, same as upstream. Native capture and UI remain Kotlin; subtitle rules,
DashScope/OpenAI alignment and final translation policy are shared with desktop Rust.
