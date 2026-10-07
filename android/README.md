# mimi for Android

Native Android version of [mimi](../README.md) — live subtitles and
translation for system audio. Pure Kotlin (no Tauri), single module.

[Download Android](https://github.com/yuxino/mimi/releases/latest)

[Provider setup and current models](../docs/provider-setup.md) · [服务开通与模型指南](../docs/provider-setup.zh-CN.md)

> Android is packaged separately from mimi desktop releases. It reads the
> same provider wire protocols as the desktop app (`src-tauri/src/core/protocols`)
> with provider-specific credentials configured on Android. Live verification is
> listed below; protocol parity alone does not establish service availability.

## Features

- **System-audio capture** — `MediaProjection` + `AudioPlaybackCapture`
  (API 29+). Requires Android’s `RECORD_AUDIO` runtime grant for playback capture;
  never uses a microphone source. Captures what other apps play
  (`USAGE_MEDIA/GAME/UNKNOWN`), 48 kHz stereo float, box-filter downmix +
  resample to the provider's target rate (16 kHz / 24 kHz mono PCM16).
- **Services** — Alibaba Cloud DashScope, OpenAI Realtime Translation, Google Gemini Live,
  Azure OpenAI, Volcano Engine Doubao, Tencent Cloud, Baidu realtime translation and xAI Grok Voice.
  Their wire contracts mirror the corresponding desktop adapters. Grok is turn-based voice translation;
  generated audio is discarded. Each service has its own encrypted credential fields and supported language choices.
- **Configuration** — tap a configured service to switch, or open its settings to edit. Azure requires a
  resource endpoint and translation/transcription deployment names; Tencent requires AppID, SecretID and
  SecretKey; Baidu requires AppID and AppKey. Other services use an API key. Optional endpoint/model overrides
  are available for DashScope, OpenAI, Gemini and xAI. Stored secrets are not filled back into the editor;
  leaving a secret field empty preserves its saved value. Changes are written only with Save and use.
- **Subtitle overlay** — `TYPE_APPLICATION_OVERLAY` floating window, text-hugging
  card, always horizontally centered over the video, vertically draggable
  (position persists). Tap it to open a translucent reading panel with the
  current source and translation; confirmed lines appear there only when
  bounded history is enabled in settings. The panel has a language shortcut,
  a labeled font-size slider, collapse action, and immersive toggle. Its compact card has adjustable background alpha
  (0–90 %), text color presets, font size, and whole-window opacity.
  Appearance changes apply immediately to an existing overlay; empty live captions
  do not leave a background-only floating window. An optional
  immersive mode shows plain text with a contrast shadow and passes touches
  through to the app below. A small control on the right edge can be moved
  vertically and exits immersive
  mode without stopping the session; the app and foreground notification remain
  backup controls. In landscape, the expanded panel stays centered and caps its
  width at 560 dp; its covered area receives touches until it is collapsed.
- **Native-subtitle behaviour** — only the current sentence is shown (drafts
  are clipped to the last sentence of the provider's cumulative buffer);
  English source speech shows source + translation lines, other languages show
  the translation only; the card hides shortly after speech ends and never
  lingers (streaming-watchdog fallback for cases where provider VAD is held
  open by background music).
- **Hotwords** — DashScope `translation.corpus.phrases` via comma-separated
  `source=translation` entries (for example, `Cyberpunk=赛博朋克`).
- **Credentials** — EncryptedSharedPreferences (Android Keystore master key),
  scoped by provider.
- **History privacy** — disabled by default; opting into history retains only a bounded
  in-memory list. Disabling it or stopping the session clears retained subtitles.
- **Interface language** — Settings offers Follow system and the same seven interface
  languages as desktop: 简体中文, 繁體中文, English, 日本語, Deutsch, 한국어 and Français.
  Changes apply immediately and persist after restart, independently of subtitle
  source and target languages. Android 13+ also exposes this choice in system App languages.
  Unsupported system languages fall back to English.

See the [native Before/After comparison](../docs/qa/android-language-and-overlay.md)
for the empty-overlay and live font-size repairs.

## Requirements

- Android 10+ (API 29); the original contributor reported Android 17 testing,
  and the updated interface is checked on an Android 15 emulator
- Credentials for one supported service (usage may incur provider charges)
- The "Display over other apps" and `RECORD_AUDIO` grants, plus per-start MediaProjection consent

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
updates. Debug installations have a different signature and must be uninstalled
first, which removes their saved settings and credentials. The APK uses release
build settings; this does not imply physical-device or all-provider validation.

## Local UI preview

The Android interface reuses Mimi's existing character artwork and neutral
light/dark palette. Subtitle source and target languages can be changed directly on the home screen with Undo. Tapping the
subtitle sample opens appearance settings, where changes are saved automatically
and previewed without capture or a network session. The appearance screen also
previews the compact card and immersive text mode. The service tab lists all eight integrations and opens a separate editor with only that service’s fields.
Credentials require explicit Save and use; changing appearance never saves credentials. Your own service credentials are still required for real translation.

On a development emulator with no active subtitle session:

```sh
./gradlew assembleDebug assembleDebugAndroidTest
adb install -r app/build/outputs/apk/debug/app-debug.apk
adb install -r app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk
adb shell am instrument -w -e theme light app.yuxino.mimi.android.test/app.yuxino.mimi.android.UiSmokeInstrumentation
adb shell am instrument -w -e theme dark app.yuxino.mimi.android.test/app.yuxino.mimi.android.UiSmokeInstrumentation
adb shell am instrument -w -e overlay_preview true -e theme light app.yuxino.mimi.android.test/app.yuxino.mimi.android.UiSmokeInstrumentation
adb shell am instrument -w -e interface_language true -e theme light app.yuxino.mimi.android.test/app.yuxino.mimi.android.UiSmokeInstrumentation
```

The interface-language check requires a blank, idle emulator. It exercises the
native picker, settings recreation and localized overlay controls without capture
or provider requests. Use `-e leave_language zh-Hans`, force-stop the app, then run
with `-e interface_language true -e expected_language zh-Hans` to check persistence.

The checks exercise quick language selection/Undo, direct appearance access,
auto-save with actual touch gestures, all eight service editors, write-only secret fields, rejected incomplete configurations, history clearing and the
keyboard. Changed non-secret preferences are restored in a finally block; provider
drafts are discarded without Save and use. No provider or audio capture session starts. Screenshots contain sample subtitles and empty key
fields and are written to the app's external `files/ui-preview` directory. Pass
`-e demo true` for a paced walkthrough suitable for emulator screen recording;
it demonstrates the labeled sample, not live translation.

## Independent text translation

In Services → Alibaba Cloud, **Text translation** offers the built-in translation,
**DeepL**, **DeepLX**, **ChatMock**, **OpenAI-compatible API**, and **No translation (original only)**.
Independent modes use Alibaba Cloud's `qwen3-asr-flash-realtime` for recognition; the selected
text service translates confirmed text. Original-only mode makes no text translation request.
Enable that ASR model for your Alibaba key. Other speech providers keep their built-in translation. This is not an OpenAI Realtime endpoint override.

- **DeepL** needs a separate DeepL API key. Mimi chooses the official Free endpoint for
  keys ending in `:fx`, otherwise the Pro endpoint. No custom URL or model is needed.
- **DeepLX** needs your service address and an optional Bearer token. Mimi appends
  `/translate` when missing; the service must accept `text`, `source_lang`, `target_lang`
  and return integer `code: 200` with translated text in `data`.
- **ChatMock** starts with `http://127.0.0.1:8000/v1`. Enter a model ID from your running
  server; Mimi does not choose a model or enable local HTTP for you.
- **OpenAI-compatible API** takes your service endpoint, model and optional key, with no
  ChatMock address prefilled. Each translator retains its own encrypted configuration.

The old combined entry remains **OpenAI-compatible API**, including its selected state,
endpoint, model and key. Nothing is copied to the new ChatMock entry or inferred from a
localhost URL. Select ChatMock and configure it separately to use the new entry.

Run [ChatMock](https://github.com/RayBytes/ChatMock) yourself and enter its `/v1` base URL
(or full `/v1/chat/completions` URL), a model ID from its `/v1/models`, and an optional
Bearer key if your reverse proxy requires one. The endpoint must accept non-streaming
Chat Completions (`model`, `messages`, `stream: false`) and return a final
`choices[0].message.content`. Recommended ChatMock options are
`--reasoning-compat legacy --reasoning-summary none`; Mimi also removes complete leading
`<think>` blocks and rejects missing final text.
The local address describes where the ChatMock API runs. Translation models are still
called online through the account signed in to ChatMock; they do not run on the phone or computer.

Use HTTPS for computers and servers, including LAN addresses. With explicit local HTTP
permission, only `localhost`, `127.0.0.1`, `[::1]` and Android emulator host `10.0.2.2`
are supported. On a phone, localhost is the phone itself. For USB development, `adb reverse
tcp:8000 tcp:8000` can make the computer's local ChatMock service available at
`http://127.0.0.1:8000/v1` on the phone. Do not use a ChatGPT password or session token as
Mimi's API key; account login stays in your ChatMock installation.

**Test translation** sends a fixed example through the same HTTP path as subtitles and
shows elapsed milliseconds. It does not save the draft or send system audio. **Save and use**
commits both stages; back discards changes. Translation keys have independent encrypted
storage and are not reused when the destination changes. Help icons open the requirements
without persistent explanatory paragraphs in the editor.

The final-only translation queue uses the shared Rust policy: one active request, three
waiting requests, up to three attempts and a 45-second total deadline including waiting
and retries. Explicit stop drains accepted work within finite shared grace windows;
errors and permission revocation abort. Overflow rejects new work and drains accepted
work. Late results cannot enter a newer session. These checks do not
prove a live DeepL, DeepLX or ChatMock account or physical Android device until those are tested separately.

## Architecture

```
app/src/main/java/app/yuxino/mimi/android/
  MainActivity.kt            start flow, overlay-permission gate, projection consent
  SettingsActivity.kt        service list and appearance settings
  ServiceSettingsActivity.kt provider-specific write-only credential editor
  ServiceSettingsUi.kt       compact service picker
  SettingsStore.kt           EncryptedSharedPreferences-backed settings
  capture/MimiService.kt     foreground service (mediaProjection type):
                             capture loop + overlay window + auto-hide timers
  resample/StreamResampler.kt  O(1) box-filter downmix + resample to PCM16 mono
  provider/ServiceCatalog.kt  service fields, capabilities and language normalization
  provider/StreamingServiceEngine.kt bounded transport for additional services
  provider/CloudProtocols.kt Gemini, Azure, Tencent and Baidu wire adapters
  provider/VolcanoProtocol.kt Doubao binary protobuf adapter
  provider/GrokProtocol.kt    xAI turn-based transcript adapter
  provider/ProviderEngine.kt   engine interface + WS URL normalization
  provider/DashScopeEngine.kt  qwen3.5-livetranslate realtime WS client
  provider/OpenAIRealtimeEngine.kt  gpt-realtime-translate WS client
  provider/SharedSubtitleCore.kt stateless JNI bridge to shared/mimi-core
  provider/SubtitleBus.kt      native snapshot facade over the shared Rust reducer
  provider/SharedLivePairStream.kt DashScope identity/transport adapter
  provider/SharedTranscriptStream.kt OpenAI transcript/transport adapter
```

## Verification status

The updated interface and Alibaba Cloud system-playback translation have been exercised in an Android 15
emulator with Firefox. The bundled Chrome tested there explicitly disables playback capture and yields silent
samples; this finding is specific to that browser package. The additional provider adapters have protocol tests,
including Tencent signing and Volcano binary frames, but have not been verified against live accounts. Do not
interpret build or protocol-test success as physical-device or all-provider acceptance. The original contributor’s
reported device test applies to the original port, not every subsequent change.

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
