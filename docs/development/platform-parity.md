# PC and Android maintenance

The supported live-session providers use one Rust implementation in
`shared/mimi-runtime`: provider selection, protocol clients, Audio3 recognition,
Qwen-MT and independent text translation, speculative preview, prioritized final
requests, bounded PCM/event queues, reconnect policy, health timing and the
session controller. Desktop modules are re-export facades. Android capture
calls the same runtime through `shared/mimi-android-jni`; Kotlin does not schedule
translation requests or interpret live provider protocol events.

`shared/mimi-core` remains the pure subtitle/reduction contract. Both renderers
consume its current complete pair and replaceable realtime projection. Android
adapts platform trust roots, system proxy selection, playback capture, lifecycle
and native UI. Historical Kotlin transport classes remain for offline protocol
regressions; production capture and the text connection-check button use Rust.
`shared/translation-contracts.json` and `shared/realtime-display-contracts.json`
provide common synthetic wire and display cases.

## Native audio target differences

| Platform | Selected system-audio target | Identity |
| --- | --- | --- |
| macOS | Application | Existing native application selection |
| Windows | Application on supported process-loopback versions | Live PID and creation time |
| Linux | One playback stream through PulseAudio / PipeWire-Pulse | Opaque connection/stream-lifetime token |
| Android | Existing platform playback capture | No Linux stream picker or desktop application selection |

Linux never groups streams by application or reroutes playback. Deleted streams,
server disconnects and app restarts require explicit reselection. A live stream
moving outputs uses desktop capture recovery. Desktop system/microphone lanes,
pause, target switching, confirmed subtitles and recording consent retain their
common implementation. Discovery and capture remain native adapters, separate
from shared provider/translation rules.

All three desktop application/stream pickers use optional bounded embedded PNG
icons. Linux resolves stream properties and installed desktop entries through
GTK's local icon theme; missing artwork keeps a selectable neutral fallback.
Android keeps its native capture UI and has no desktop stream/icon picker.

## Preventing maintenance forks

A desktop business fix belongs in the shared crates, so an Android build uses
the same changed source without a second manual patch. Run
`python3 -B scripts/check-shared-runtime.py` to check repository path dependencies,
desktop re-export facades, the production Android engine entry and references to
historical Kotlin clients. Historical Kotlin engines, protocol clients and the
independent text-client factory live only in the JVM test source set. Production
metadata and native probe result/cancel types remain in their native adapters.
The guard has no historical-file exemptions and rejects a new Java/Kotlin provider
transport or engine in production, including variant/flavor source directories.
APK verification also rejects the historical runtime descriptors in every DEX
file, so a stale or generated source cannot silently ship those implementations.

Canonical/shared checks and desktop CI run the guard and its mutation regressions.
Android preBuild and direct native-library builds run it before compilation.
Gradle tracks shared Rust sources, manifests, locks, build scripts and Cargo
configuration; shared changes invalidate the native-library build inputs rather
than leaving an old generated library marked up-to-date. Android CI now follows
all desktop frontend/backend source, shared source, scripts and workflow changes
on pull requests and main pushes, while retaining both build variants, actual
host JNI tests and APK verification.

These checks prevent the covered structural forks; they do not prove every
future semantic policy or native rendering change is identical. Capture,
credentials, trust/proxy, lifecycle and UI remain platform adapters and need
their own evidence. New desktop-only features must be reviewed against this
document. The CI edits take effect after this source is integrated; cloud
architecture tests are not a successful remote CI or Android build.

## Shared behavior

In Alibaba ASR + MT mode, both platforms select Qwen-MT Lite, Flash or Plus
through the same provider factory; Lite is the default. Recognition uses Audio3. The shared capability
normalizer retains the Lite language intersection for comparable trials; Audio3
itself represents 30 explicit hints plus automatic detection. Plus returns
complete translations; Lite and Flash stream. Android keeps old model/custom
endpoint/hotword preference keys, but does not offer unsupported built-in
transport overrides. Effective runtime languages are normalized without writing
back over saved preferences.

- DeepL, DeepLX, ChatMock and OpenAI-compatible services are text translators, separate from speech
  recognition. They receive confirmed recognized text and their own credentials.
- DeepL uses a required API key and the official Free/Pro origin. DeepLX uses its own JSON
  protocol and an optional Bearer token. OpenAI-compatible services allow a blank key when
  authentication is not required; never substitute a speech key or reuse a key after a
  destination change.
- ChatMock and OpenAI compatible are independent saved choices using one transport. Only
  a new ChatMock draft defaults to `http://127.0.0.1:8000/v1`; the model remains required.
  Existing combined-entry settings remain OpenAI compatible, and credentials never move
  between choices. Localhost means the device running Mimi, while translation models are
  still called online through the signed-in ChatMock account.
- OpenAI-compatible/ChatMock text translation accepts the represented source and target language codes on both platforms, including Traditional Chinese. These are configurable request values, not a discovery result or guarantee of model support. Existing Chinese/English/Japanese prompts are unchanged; shared fixtures cover the additional mappings. DeepL/DeepLX retain their separate implemented catalogs.
- Provider language catalogs are model-specific and checked against `shared/provider-language-catalogs.json` and shared wire fixtures. Settings, tray and floating controls resolve the same saved source/target pair. Tencent and Volcano restrict targets by the selected source; their Chinese/English mixed modes are explicit choices, not unrestricted automatic detection. DeepL and DeepLX keep separate source/target catalogs.
- Volcano's native adapters accept proto3 omitted text as empty and accumulate
  incremental source/translation previews separately until their official sentence
  boundaries. Both enforce the existing shared 64 KiB UTF-8 subtitle-field limit
  before appending each delta; Android also retains its turn-buffer character bound.
  Shared wire/sequence fixtures check the common behavior.
- Endpoint prefixes are preserved. An OpenAI-compatible base receives `/chat/completions`;
  use an explicit `/v1` base for ChatMock. Complete endpoints remain unchanged. HTTPS is
  the default; each platform's explicit local transport boundary remains enforced.
- Only complete translation text is displayed. Leading ChatMock reasoning blocks are
  removed; unfinished responses and malformed/empty data fail without echoing their text.
- Shared subtitle fields preserve complete text within the same 65,536-byte UTF-8 limit; punctuation
  and long sentences do not cause suffix-only display or cropped confirmed history.
  One complete current pair is retained independently of optional history. Raw drafts
  cannot clear that retained pair; shared `realtimePreview` can display newer realtime
  lanes while it remains available. Previous final lanes and different sentence owners
  cannot accompany a new draft. Older confirmed identities cannot overwrite newer complete owners.
  An Android idle-hide timer cannot hide this complete current pair while the
  session runs; replacing it or ending the session retires its display.
- Final work keeps its own immutable source identity, one active request, at most three
  waiting requests and a 45-second total budget including waiting and retries. Retry
  classification, attempt bounds and backoff come from shared Rust policy. Explicit stop
  uses bounded provider/final grace windows; aborts cancel immediately. Late retired-
  generation results cannot enter a new session. Previews never confirm history.
  The shared high-quality client deduplicates finish against a known source
  revision, or the same local confirmation when the ASR has no ID. Equal text
  from separate confirmations stays independent on both production paths.
- DashScope realtime pairs source and translation using conversation-item links;
  a known response without a source link cannot guess a source by arrival order.
  OpenAI append-only streams share timing/punctuation alignment and safe tail flush.
  Gemini transcription options belong to setup, and language-only text updates
  are empty deltas. Its ID-less continuous transcript uses the shared complete-
  block checkpoint after 2 seconds without changing text, including unpunctuated
  paired text and unequal sentence counts. Fragments append verbatim; strict
  cumulative extensions replace the current lane, with drafts published first.
  This heuristic can delay confirmation during continuous speech;
  drafts stay bounded at 5,120 characters. Both platforms retain explicit turn
  boundaries with a shared 500 ms late-tail grace; interruptions discard buffers.
  Gemini Stop allows 4.5 seconds inside the six-second provider budget so its
  two-second paired quiet checkpoint can finish without `turnComplete`. A
  deadline or one-sided tail is never promoted into a final pair.
- Configuration changes are drafts until the explicit save action. Field labels and
  errors stay legible; protocol explanations use help controls. Provider artwork comes
  from the same existing desktop asset source.

## Deliberate platform differences

Both production paths use the same Audio3 decoder and classify the exact
`UNSUPPORTED_LANGUAGE` failure as requiring configuration correction. Native
error presentation differs; the historical Kotlin realtime decoder is not the
Android production route.

| Area | Desktop | Android |
| --- | --- | --- |
| Independent text services | DeepL, DeepLX, ChatMock, OpenAI compatible; original-only with custom ASR | Same shared text clients after Audio3 ASR; original-only supported |
| Text service display names | Optional names per profile and independent route; shown in settings and subtitle service label | Existing provider labels |
| Recognition selection | Eight built-in services plus custom DashScope/OpenAI ASR and Apple Speech on supported Apple silicon Macs running macOS 26+; optional per-profile user declaration narrows custom recognition languages | Eight built-in adapters; independent text currently pairs with Alibaba ASR; no custom speech profile/declaration editor |
| Profile activation languages | Optional saved source/target pair per configuration; defaults to keeping current languages. Temporary session changes do not rewrite the pair | No saved profile-language pair editor |
| Active configuration/model changes | Saved profiles can switch through reconnect; editing a model requires Stop | Service/model editors require Stop and reject stale Save actions; Captions and expanded controls show the session's actual shared-factory model list |
| Runtime recovery feedback | Shared connection/retry/timeout state in native UI | Same shared state, localized in floating subtitles; capture-no-audio guidance takes priority |
| Apple local recognition | One language selector with runtime resource status; explicit Download and use prepares and applies a missing language, while ready languages can be applied directly; no speech API key; independent text translation keeps its own service and credentials | Not available; no Apple API or asset-management dependency |
| Apple local text translation | Apple silicon with macOS 26+: runtime language-pair availability, explicit in-app model preparation, installed-only translation without API keys or a text proxy; pairs with Apple/custom/Alibaba recognition | Not available; Apple Translation is a native macOS adapter, not a shared HTTP provider |
| Alibaba modes | New profiles default to simultaneous translation (LiveTranslate), with automatic source detection and 60 represented text-translation targets. ASR + MT remains selectable; existing and copied profiles retain their mode | Same two modes and shared capabilities; new configurations default to simultaneous translation. Existing profiles migrate as ASR + MT; each profile keeps its mode, workspace and independent translator |
| Alibaba ASR + MT pipeline | Audio 3.0: 30 explicit recognition hints plus automatic detection; Qwen-MT retains its model-specific target catalog | Same Audio3 + Qwen-MT factory, models and capability normalization |
| Translation scheduling | Speculative drafts plus serial prioritized finals and provider recovery | Same Rust speculative preview, final priority, queues, retry/cooldown and MT budget continuity |
| Opening MT previews | Shared opening length eligibility for the stability timer; a non-resetting maximum still permits short drafts after at least two seconds. Existing preview corrections and source finals retain their paths | Same Rust opening eligibility and wait bound; no native reading-speed toggle or separate MT scheduler |
| Independent text HTTP bounds | HQ source fields up to 65,536 UTF-8 bytes; response bodies up to 1 MiB; decoded text uses native adapter bounds | Same shared clients and UTF-8/response bounds |
| Subtitle display and appearance | Original/translation/bilingual presentation, alignment, source colors and background controls | Captions combines one pinned live preview, display mode, immersion and directly visible styles. Preview and scrollable controls stay together, above/below in portrait and side by side on wide screens. Native page/floating controls share left/center/right alignment, direct color swatches, width (55–100%), height (20–70%), font, caption color, background and window opacity. Floating quick settings update the actual caption window above the editor. Recognition without translation always shows original text. These choices do not change provider requests |
| Subtitle background | Adjustable card opacity (80% default); history does not fade with age | Pure black background matching desktop (80% default, 0–100%) and separate window opacity (100% default); explicit saved values survive updates. Immersion disables both controls while retaining their saved values |
| Subtitle font family | Search installed computer fonts or use System default; one saved family applies to original/translation in standard and immersive subtitles, with glyph fallback | Native subtitle size controls; no installed-family picker |
| Subtitle font size | 14–48 px, default 16; bilingual reference uses a stable 90% size, and the minimum window height grows with the selected font | Native 12–24 sp range; desktop sizing does not change Android typography |
| Floating subtitle controls | Shared settings/control-panel time preference; pause/resume is reachable in Immersive Mode. Keyboard-accessible collapse and native window drag/resize | One normal panel moves from its top handle and resizes width/height from its corner. Normal and immersive positions remain independently saved. Immersion uses lighter text, chosen alignment and a 24dp exit inside a 48dp target. Exit restores normal size/position. Floating views hide above Mimi pages; Captions previews current captions and leaving restores the same views. Services exposes the active service restriction with Stop, while styles remain editable. The main footer keeps Stop visible in every section |
| Reading long current subtitles | Bounded live tail with explicit history reading and return-to-live behavior | Normal panel follows complete glyph lines from the bottom reading edge, so a growing reference does not push an unchanged translation downward. It scrolls the complete bounded current pair and optional history inside the selected height; immersive captions keep two source and three translation lines. Captions reuses the actual normal/immersive native frame, including chrome and bounded history, measured at real window dimensions and uniformly scaled into a stable viewport. Before speech or during silence, an active overlay shows desktop's listening message with the selected typography/color/alignment |
| Audio capture | System audio by default; explicitly selected microphone or both sources with independent lanes. OS-specific desktop capture; selected-app audio on macOS and Windows build 20348+, Linux retains output-monitor capture | Android playback-capture consent and foreground service; only apps that allow playback sharing, no microphone input or selected-app picker. Android still requires RECORD_AUDIO for playback capture |
| Caption reading metadata | Optional sentence dividers and local confirmation time, both off by default | Same optional appearance choices; time comes from the shared final, not Kotlin arrival time; dividers stay hidden in immersion |
| Saved configurations | Multiple named profiles, including the same provider | Multiple named profiles with independent encrypted credentials and translators; Add, Settings, Duplicate and Delete, with tap-to-select. Legacy provider settings migrate once; saving an edit does not switch the active profile |
| Saved service selection | Settings, tray and subtitle controls can switch during listening via reconnect; pause and confirmed subtitles persist. A recording cannot switch PCM sample rate | Tap a saved configuration in the list to select it; switching/editing requires Stop. Order remains stable; deleting the active profile selects the first remaining entry |
| Proxy preferences | Per-profile independent recognition/text routes; integrated realtime uses one route | Shared network transport with immutable Android platform proxy/trust snapshots; no per-stage proxy UI |
| Gemini planned connection rotation | Prepare one replacement after GoAway, bounded old-tail drain and up to two seconds of PCM staging; shorter Gemini live preview cadence | Same Rust Gemini client, replacement-socket handoff and preview cadence |
| Secret storage | Private local credential file; one-time OS-store migration | Private profile metadata file and separate Android Keystore-backed encrypted credential revisions; legacy settings migrate only after verified durable writes |
| Local HTTP | Existing loopback endpoint validation | Explicit per-config opt-in and Android network allowlist; includes emulator host |
| History/recording | Optional bounded local session files and selected-input recordings; confirmed panel rows survive stop/start within the app process and keep the existing display bound | Bounded in-memory subtitle history defaults to three confirmed captions on fresh installs; existing settings and the legacy zero default are preserved. Capture starts clear its overlay; no desktop recording/export parity claimed |
| Development evidence | Exact dev app only; opt-in sent audio, subtitle snapshots, causal traces, saved-case playback and bounded evidence workspaces | No matching debugger or sent-audio recording claimed |

These differences are current scope, not proof of live-account acceptance. When expanding a
feature, update this table and the affected cross-platform fixtures instead of assuming the
other implementation already matches. Keep transport behavior shared while respecting each
platform's native UI, permissions and resource limits.

The shared runtime owns provider-specific request, result and HTTP body limits
as well as the shared reducer UTF-8 limit. The old Kotlin transports are offline
regression subjects and do not define production limits.

## Verification and change review

- Run `./scripts/check.sh` for desktop, including shared core and fixture tests.
- Android JVM tests load the actual host JNI library and replay
  `shared/subtitle-contracts.json` and `shared/live-pair-contracts.json`;
  there is no test-only Kotlin fallback. APK checks require every native ABI and 16 KiB
  page alignment. See `shared/mimi-android-jni` and the native build script.
- Run Android debug/release unit tests, lint and APK builds; Kotlin reads the same JSON from
  its test resources. Native instrumentation covers draft/save/key isolation and local HTTP.
- Shared contracts or provider changes trigger both CI paths. UI-only platform edits keep
  their usual checks. CI never substitutes for an actual provider/account/device session.
- Review queue bounds, lock ordering, cancellation, stale generations and final deadlines
  on both sides when changing scheduling. Repair provider rules in shared Rust;
  do not copy desktop scheduling into a second Kotlin production engine.

Desktop subtitle controls support independent system/microphone subtitle colors,
background transparency, and optional local HH:mm:ss confirmation timestamps for
every input, including Immersive Mode. Settings and the floating panel share the
same time switch, which also takes effect immediately in system-only mode.
System-only mode hides the system source icon and keeps retained microphone rows
identifiable with a small icon (inline when time is off); the timestamp preference survives input
changes. Single-source output keeps its own color and a neutral bilingual original.
These are desktop presentation preferences; Android does not currently expose
matching source-color or time controls, or desktop microphone selection.

Android's normal panel has a bounded reading viewport, with complete current
source and translation text inside its scroll area. Opening it locates the current
caption; a changed completed pair locates its beginning only while the user is
still reading the current-caption region. Draft updates retain the scroll position,
and reading earlier history is not interrupted. Completion uses the shared
`displayPairFinal` field. Optional history keeps its existing display limit;
normal reading does not enable retention, and immersive captions keep
their existing line caps. This native presentation policy is separate from the
desktop live-tail behavior and does not imply parity in capture, recording or
timestamp preferences.

Desktop offers a default-on **Show interim subtitles** preference, including live session changes. Its realtime transports consume shared realtime projection; current Alibaba Turbo and independent text translation keep complete previews. Android uses the same shared realtime projection for integrated streaming events. With it off, only final lines and confirmed pairs appear; bounded Stop-tail fallback still follows the existing desktop final lane. This affects presentation, not provider requests or accuracy. Android currently has no matching switch. Shared snapshots expose `displayPairFinal` on both platforms, and Rust/JNI fixtures distinguish a completed preview from an accepted final even without retained presentation history.

## Interface languages

Desktop and Android support Simplified Chinese, Traditional Chinese, English,
Japanese, German, Korean, French and Thai interface languages. Desktop uses
regional system-language detection and in-place window updates; Android keeps
its native resource and locale selection path. Recognition/translation language
support is independent of the interface language and still follows each
provider's capabilities.

## Android immersive placement

Native mode changes update the existing window and caption views. The saved
immersive bottom offset is independent of the normal-panel frame;
exiting immersive returns to its saved normal size and placement. Only
explicit completed caption drag/resize writes new bounds. Rotation, content/font
reflow and inset constraints clamp the visible offset without replacing that
preference; a larger viewport restores it. Android 30+ fits system-bar/cutout
insets once, ignoring bar visibility, and sizes against the same window metrics.
API 29 uses its native visible display frame and still needs device acceptance.

Captions and the floating controls use one preference and the existing
first-use explanation. The small touchable restore window stays adjacent to
caption text (below when there is space, otherwise above); the caption window
remains touch-through. This is Android presentation adaptation, not another
subtitle/translation implementation.

`python3 scripts/check-android-overlay-placement.py` compiles the actual native
placement class and runs the same cases used by Android JUnit without a SDK.
This checks state/geometry, not WindowManager pixels, insets delivery or touch
dispatch. The native `overlay_interaction` fixture covers Home/panel entry,
restoration, repeated toggles and font/content changes when run on a device.
