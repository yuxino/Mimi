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
adapts platform trust roots, system proxy selection, playback/microphone capture, lifecycle
and native UI. Production capture and the text connection-check button use Rust;
protocol regressions exercise that same implementation.
`shared/translation-contracts.json` and `shared/realtime-display-contracts.json`
provide common synthetic wire and display cases.

## xAI recognition and text translation

Desktop and Android expose xAI streaming ASR through their existing independent
text-service editor. New xAI profiles select ASR and Grok text translation;
profiles saved without a mode retain Grok Voice. ASR can use other supported text
services, and Grok is available to other supported independent recognizers.
Each stage stores its own key and has a separate proxy snapshot and check.
The same existing xAI key may be entered in both fields. Old STT profiles receive
one verified same-profile migration; later deletion never borrows the speech key.
Original-only captions skip text credentials and requests. Grok Voice retains its
integrated route and hides, without deleting, the independent selection.

Android also exposes independent recognition through Alibaba ASR and the
OpenAI, Gemini and Azure ASR modes described below. Desktop-only Apple/custom
recognizer entry points remain unavailable there.

## OpenAI, Gemini and Azure speech modes

Both platforms offer simultaneous translation and ASR + MT for OpenAI, Gemini
and Azure OpenAI. New profiles and profiles without `speechTranslationMode`
default to simultaneous translation. Mode changes require a stopped session.
Switching hides, without deleting, the saved independent text destination, its
credentials and proxy. Simultaneous sessions never use that hidden destination.

ASR + MT uses the existing shared recognition/MT pipeline and independent text
editor. OpenAI uses dedicated realtime transcription; Azure uses the configured
transcription deployment and resource API key; Gemini uses dedicated Live
transcription. Azure ASR does not require a translation deployment. Switching
back preserves editable saved fields while refusing to start simultaneous
translation until that deployment is supplied. Recognition and text checks
resolve only their own credentials; original-only captions require no MT setup.

Recognition hints and exact wire spellings come from
`shared/provider-language-catalogs.json`. Gemini uses its published BCP-47
catalog. The OpenAI/Azure explicit hint picker currently exposes the examples
documented by the realtime transcription guide, not a claimed full model
language catalog; automatic detection remains available. Selected text services
further restrict language choices according to their own capability contract.

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

## Android capture and saved sessions

Android defaults to system playback and also exposes explicit Microphone and Both.
Each selected source has its own AudioRecord, resampler, bounded PCM ingress and
shared runtime handle. Microphone-only starts the microphone foreground-service
type without a MediaProjection token; Both uses both service types. Android still
requires RECORD_AUDIO for system playback, but system-only never opens a microphone.
Input changes require a stopped session; failure or revocation stops selected lanes
without mixing audio or silently falling back. Existing lock-screen stop behavior
remains. Device capture quality requires native acceptance in addition to shared tests.

Save captions is off by default and must be enabled before Start. Confirmed events
feed the same private bounded JSONL journal and final JSON archive as desktop via
`shared/mimi-runtime::session_history`; JNI serializes source ownership and sealing.
Stop drains accepted work, then finalizes after both runtime leases release. Disabling
saving discards the current journal; saved sessions require explicit deletion.
Android reads one shared 30-entry page at a time and exports shared TXT/Markdown
through the system document picker, streaming an app-private temporary file.
No full transcript is retained in Kotlin. Storage failures and limits remain visible.
Android does not offer PCM recording or audio export.

## Preventing maintenance forks

A desktop business fix belongs in the shared crates, so an Android build uses
the same changed source without a second manual patch. Run
`python3 -B scripts/check-shared-runtime.py` to check repository path dependencies,
desktop re-export facades, the production Android engine entry and references to
retired Kotlin clients. Historical engines, protocol clients and their exclusive
tests remain in Git history. Golden JSON contracts, direct Rust tests, actual
JNI tests and native adapter checks are the maintained coverage. Metadata and
native probe result/cancel types remain in their native adapters.
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
document. Architecture checks alone do not establish successful platform builds
or native device acceptance.

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
  session runs; Stop keeps bounded confirmed content readable until a new session begins.
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
  OpenAI/Azure append-only streams share timing/punctuation alignment and safe
  tail flush. A jointly covered interval stays whole: independently trimming
  each lane at its last punctuation can put adjacent sentences in the wrong
  pair. `transcript-continuity-contracts.json` checks this through direct Rust
  and actual JNI. Their shared runtime detects sustained translation without
  source progress (20 seconds, at least eight updates and 64 characters, with
  no translation gap over five seconds); silence alone does not trigger it.
  This retires the stale source preview, bounds the unpaired translation to
  320 characters, and keeps the connection and translated output running. A new
  original starts a fresh block without inheriting that orphan translation.
  Missing optional source transcription is not a transport failure; Gemini's
  separate block protocol does not use this rule.
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
error presentation differs; Kotlin does not decode realtime provider frames.

| Area | Desktop | Android |
| --- | --- | --- |
| Independent text services | DeepL, DeepLX, ChatMock, OpenAI compatible; original-only with independent recognition | Same shared text clients with Alibaba, xAI, OpenAI, Gemini and Azure ASR; original-only supported |
| Text service display names | Optional names per profile and independent route; shown in settings and subtitle service label | Existing provider labels |
| Recognition selection | Eight built-in services plus custom DashScope/OpenAI ASR and Apple Speech on supported Apple silicon Macs running macOS 26+; optional per-profile user declaration narrows custom recognition languages | Eight built-in adapters; independent text pairs with Alibaba, xAI, OpenAI, Gemini and Azure ASR; no custom speech profile/declaration editor |
| Profile activation languages | Optional saved source/target pair per configuration; defaults to keeping current languages. Temporary session changes do not rewrite the pair | No saved profile-language pair editor |
| Active configuration/model changes | Saved profiles can switch through reconnect; editing a model requires Stop | Service/model editors require Stop and reject stale Save actions; service/model details remain in the app, not on the simple floating caption panel |
| Runtime recovery feedback | Shared connection/retry/timeout state in native UI | Same shared state, localized in floating subtitles; capture-no-audio guidance takes priority |
| Apple local recognition | One language selector with runtime resource status; explicit Download and use prepares and applies a missing language, while ready languages can be applied directly; no speech API key; independent text translation keeps its own service and credentials | Not available; no Apple API or asset-management dependency |
| Apple local text translation | Apple silicon with macOS 26+: runtime language-pair availability, explicit in-app model preparation, installed-only translation without API keys or a text proxy; pairs with independent recognition | Not available; Apple Translation is a native macOS adapter, not a shared HTTP provider |
| Alibaba modes | New profiles default to simultaneous translation (LiveTranslate), with automatic source detection and 60 represented text-translation targets. ASR + MT remains selectable; existing and copied profiles retain their mode | Same two modes and shared capabilities; new configurations default to simultaneous translation. Existing profiles migrate as ASR + MT; each profile keeps its mode, workspace and independent translator |
| Alibaba ASR + MT pipeline | Audio 3.0: 30 explicit recognition hints plus automatic detection; Qwen-MT retains its model-specific target catalog | Same Audio3 + Qwen-MT factory, models and capability normalization |
| Translation scheduling | Speculative drafts plus serial prioritized finals and provider recovery | Same Rust speculative preview, final priority, queues, retry/cooldown and MT budget continuity |
| Opening MT previews | Shared opening length eligibility for the stability timer; a non-resetting maximum still permits short drafts after at least two seconds. Existing preview corrections and source finals retain their paths | Same Rust opening eligibility and wait bound; no native reading-speed toggle or separate MT scheduler |
| Independent text HTTP bounds | HQ source fields up to 65,536 UTF-8 bytes; response bodies up to 1 MiB; decoded text uses native adapter bounds | Same shared clients and UTF-8/response bounds |
| Subtitle display and appearance | Original/translation/bilingual presentation, alignment, source colors and background controls | Captions combines one pinned live preview, display mode, the independent legacy immersive setting and directly visible styles. Preview and scrollable controls stay together, above/below in portrait and side by side on wide screens. In-app appearance controls retain left/center/right alignment, direct color swatches, width (55–100%), height (10–70%, default 17% subject to a readable-line minimum), font, caption color, background and window opacity. The normal floating panel has no route, settings entry, provider badges or Immersive footer action. Recognition without translation always shows original text. These choices do not change provider requests |
| Subtitle background | Adjustable card opacity (80% default); history does not fade with age | Pure black background matching desktop (80% default, 0–100%) and separate window opacity (100% default); explicit saved values survive updates. Immersion disables both controls while retaining their saved values |
| Subtitle font family | Search installed computer fonts or use System default; one saved family applies to original/translation in standard and immersive subtitles, with glyph fallback | Native subtitle size controls; no installed-family picker |
| Subtitle font size | 14–48 px, default 16; bilingual reference uses a stable 90% size, and the minimum window height grows with the selected font | Native 12–24 sp range; desktop sizing does not change Android typography |
| Floating subtitle controls | Shared settings/control-panel time preference; pause/resume is reachable in Immersive Mode. Keyboard-accessible collapse and native window drag/resize | One simple normal panel has a small inset top-right switch, accessible as Adjust captions, without a separate background or divider. Checked/ON enables moving from its top handle and resizing width/height from its corner; OFF makes the caption window touch-through without changing its bounds, paint, typography or caption/history content. The switch remains separately touchable for recovery; Android exposes its checked state, without a second Watch label. This does not change the legacy immersive preference. Normal and immersive positions remain independently saved. Legacy immersion retains lighter text, chosen alignment, compact line limits and a 24dp exit inside a 48dp target; exit restores normal size/position. Floating views hide above Mimi pages; Captions previews current captions and leaving restores the same views. App language, service, appearance and Start/Stop controls remain available; the main footer keeps Stop visible in every section |
| Reading long current subtitles | Bounded live tail with explicit history reading and return-to-live behavior | Normal panel follows complete glyph lines from the bottom reading edge, so a growing reference does not push an unchanged translation downward. It scrolls the complete bounded current pair and optional history inside the selected height; immersive captions keep two source and three translation lines. Captions reuses the actual normal/immersive native frame and bounded history, measured at real window dimensions and uniformly scaled into a stable viewport; normal previews do not reintroduce removed toolbar/footer chrome. Before speech or during silence, an active overlay shows desktop's listening message with the selected typography/color/alignment |
| Audio capture | System audio by default; explicitly selected microphone or both sources with independent lanes. OS-specific desktop capture; selected-app audio on macOS and Windows build 20348+, Linux retains output-monitor capture | System playback by default, explicit microphone or both independent lanes with source labels and matching foreground-service types. Only apps that allow playback sharing; no selected-app picker. Android still requires RECORD_AUDIO for playback capture |
| Caption reading metadata | Optional sentence dividers and local confirmation time, both off by default | Same optional appearance choices; time comes from the shared final, not Kotlin arrival time; dividers stay hidden in immersion |
| Saved configurations | Multiple named profiles, including the same provider | Multiple named profiles with independent encrypted credentials and translators; Add, Settings, Duplicate and Delete, with tap-to-select. Legacy provider settings migrate once; saving an edit does not switch the active profile |
| Saved service selection | Settings, tray and subtitle controls can switch during listening via reconnect; pause and confirmed subtitles persist. A recording cannot switch PCM sample rate | Tap a saved configuration in the list to select it; switching/editing requires Stop. Order remains stable; deleting the active profile selects the first remaining entry |
| Proxy preferences | Per-profile independent recognition/text routes; integrated realtime uses one route | Shared network transport with immutable Android platform proxy/trust snapshots; no per-stage proxy UI |
| Gemini simultaneous connection rotation | Prepare one replacement after GoAway, bounded old-tail drain and up to two seconds of PCM staging; shorter Gemini live preview cadence | Same Rust Gemini client, replacement-socket handoff and preview cadence; ASR mode uses the shared transcription client and recovery policy |
| Secret storage | Private local credential file; one-time OS-store migration | Private profile metadata file and separate Android Keystore-backed encrypted credential revisions; legacy settings migrate only after verified durable writes |
| Local HTTP | Existing loopback endpoint validation | Explicit per-config opt-in and Android network allowlist; includes emulator host |
| History/recording | Optional bounded local files and selected-input recordings; saved-session names and name/text/audio filters, original/translation/bilingual reading and passage copy, TXT/Markdown export with content and optional confirmation times, WAV export. Saved sessions can export during capture; inline playback with pause and seeking requires Stop, and includes current audio only while it remains available. Confirmed panel rows retain their existing bound across stop/start. The display-count choice defaults to All; recent 1–10 confirmed subtitles apply only while following live speech, and scrolling up opens the existing bounded history. This display projection does not truncate saved files or exports. | Bounded in-memory subtitle history defaults to three confirmed captions on fresh installs; existing settings and the legacy zero default are preserved. Stop retains confirmed display; a new session clears it. Opt-in private saved sessions use the same Rust journal, paged reader and TXT/Markdown formatter, with native SAF export and explicit deletion. No PCM recording, session renaming or desktop audio playback UI |
| Development evidence | Exact dev app only; opt-in sent audio, subtitle snapshots, causal traces, saved-case playback and bounded evidence workspaces | No matching debugger or sent-audio recording claimed |

These differences are current scope, not proof of live-account acceptance. When expanding a
feature, update this table and the affected cross-platform fixtures instead of assuming the
other implementation already matches. Keep transport behavior shared while respecting each
platform's native UI, permissions and resource limits.

The shared runtime owns provider-specific request, result and HTTP body limits
as well as the shared reducer UTF-8 limit. Native adapters consume these results
without keeping another provider implementation.

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
These are desktop presentation preferences; Android uses native source labels and
its existing timestamp preference, without matching source-color controls.

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

Desktop and Android support nine interface languages: Simplified Chinese,
Traditional Chinese, English, Japanese, German, Korean, French, Thai and Russian. Desktop uses
regional system-language detection and in-place window updates; Android keeps
its native resource and locale selection path. Recognition/translation language
support is independent of the interface language and still follows each
provider's capabilities.

## Android immersive placement

The normal-panel adjustment switch is an interaction control, not an immersive
mode selector. ON allows move/resize; OFF makes the same caption panel
click-through. Keep its bounds, background/window opacity, typography, alignment
and caption/history projection unchanged across this toggle. The top-right
switch is visually inset, with its full 48dp target clear of the reading viewport.
CaptionPanelView paints its inert marker in native and preview; an aligned,
non-painting independent MaterialSwitch window owns touch/accessibility so it
remains reachable when the caption window is not touchable, without compositing
the control twice. Its localized accessible name is Adjust captions; use
Android's checked state for ON/OFF.
Do not repurpose the stored immersive setting or add a separate Watch label.

Legacy immersive mode remains independent, with its existing compact line limits,
anchor and restore control. Native mode changes update the existing window and
caption views. The saved immersive bottom offset is independent of the normal-panel frame;
exiting immersive returns to its saved normal size and placement. Only
explicit completed caption drag/resize writes new bounds. Rotation, content/font
reflow and inset constraints clamp the visible offset without replacing that
preference; a larger viewport restores it. Android 30+ fits system-bar/cutout
insets once, ignoring bar visibility, and sizes against the same window metrics.
API 29 uses its native visible display frame and still needs device acceptance.

The legacy Captions immersive setting retains its existing first-use explanation,
independently of normal-panel adjustment. Its small touchable restore window stays
adjacent to caption text (below when there is space, otherwise above); the immersive
caption window remains touch-through. This is Android presentation adaptation,
not another subtitle/translation implementation.

`python3 scripts/check-android-overlay-placement.py` compiles the actual native
placement class and runs the same cases used by Android JUnit without a SDK.
This checks state/geometry, not WindowManager pixels, insets delivery or touch
dispatch. The native `overlay_interaction` fixture covers Home/panel entry,
restoration, repeated toggles and font/content changes when run on a device.
