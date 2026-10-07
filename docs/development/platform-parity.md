# PC and Android maintenance

Subtitle state, complete current pairs, DashScope item-identity pairing, OpenAI
delta alignment, Gemini continuous transcript assembly and final translation
policy now have one implementation
in `shared/mimi-core`. Desktop
imports the Rust crate; Android's Kotlin adapters call that same crate through JNI.
Provider transports and OS integration remain native. Their wire behavior is a shared
product contract and changes must still be verified on both platforms.
`shared/translation-contracts.json` holds synthetic request/response fixtures used directly
by both platforms' tests. Add an affected case there when changing an external text API;
update both implementations and their platform-specific tests in the same change.

## Shared behavior

Desktop Alibaba configurations can select Qwen-MT Lite, Flash or Plus for text
translation, including the development preset. Android retains Lite and has no
model picker. Desktop keeps the existing Lite language range across these choices
for comparable trials; recognition remains Audio 3.0. Plus returns complete
translations, while Lite and Flash stream text. Provider wire formats and shared
subtitle rules are unchanged.

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
  cannot clear it; older confirmed identities cannot overwrite newer complete owners.
  An Android idle-hide timer cannot hide this complete current pair while the
  session runs; replacing it or ending the session retires its display.
- Final work keeps its own immutable source identity, one active request, at most three
  waiting requests and a 45-second total budget including waiting and retries. Retry
  classification, attempt bounds and backoff come from shared Rust policy. Explicit stop
  uses bounded provider/final grace windows; aborts cancel immediately. Late retired-
  generation results cannot enter a new session. Previews never confirm history.
  Desktop finish deduplication additionally joins a known source revision, or
  the same local confirmation when the ASR has no ID. Equal text from separate
  confirmations stays independent. Android's final-only adapter has no matching
  draft-finish text alias; this desktop admission repair does not change its flow.
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
- Configuration changes are drafts until the explicit save action. Field labels and
  errors stay legible; protocol explanations use help controls. Provider artwork comes
  from the same existing desktop asset source.

## Deliberate platform differences

Desktop's custom Audio3 adapter recognizes the exact `UNSUPPORTED_LANGUAGE`
`task-failed` code as a configuration error and offers speech settings instead of
blind retry. Android's DashScope adapter currently uses the separate realtime
`session.update` protocol, so this Audio3 transport change has no Android decoder
counterpart. Shared subtitle and independent text-translation policy are unchanged.

| Area | Desktop | Android |
| --- | --- | --- |
| Independent text services | DeepL, DeepLX, ChatMock, OpenAI compatible; original-only with custom ASR | Same text choices after Alibaba realtime ASR; original-only supported |
| Text service display names | Optional names per profile and independent route; shown in settings and subtitle service label | Existing provider labels |
| Recognition selection | Eight built-in services plus custom DashScope/OpenAI ASR and Apple Speech on supported Apple silicon Macs running macOS 26+; optional per-profile user declaration narrows custom recognition languages | Eight built-in adapters; independent text currently pairs with Alibaba ASR; no custom speech profile/declaration editor |
| Profile activation languages | Optional saved source/target pair per configuration; defaults to keeping current languages. Temporary session changes do not rewrite the pair | No saved profile-language pair editor |
| Apple local recognition | One language selector with runtime resource status; explicit Download and use prepares and applies a missing language, while ready languages can be applied directly; no speech API key; independent text translation keeps its own service and credentials | Not available; no Apple API or asset-management dependency |
| Apple local text translation | Apple silicon with macOS 26+: runtime language-pair availability, explicit in-app model preparation, installed-only translation without API keys or a text proxy; pairs with Apple/custom/Alibaba recognition | Not available; Apple Translation is a native macOS adapter, not a shared HTTP provider |
| Built-in Alibaba pipeline | Audio 3.0: 30 explicit recognition hints plus automatic detection; Qwen-MT retains its model-specific target catalog | Qwen 3.5 live translation: 60 language codes; independent text translation uses Qwen ASR's 27 explicit hints plus automatic detection. This model difference is intentional |
| Translation scheduling | Speculative drafts plus serial prioritized finals and provider recovery | Final-only serial HTTP; same shared final bounds/retry decisions, native execution and cancellation |
| Independent text HTTP bounds | HQ source fields up to 65,536 UTF-8 bytes; response bodies up to 1 MiB; decoded text uses native adapter bounds | Source/result text up to 4,096 UTF-16 code units; response bodies up to 64 KiB |
| Subtitle background | Adjustable card opacity (80% default); history does not fade with age | Existing native overlay background settings and history styling |
| Subtitle font family | Search installed computer fonts or use System default; one saved family applies to original/translation in standard and immersive subtitles, with glyph fallback | Existing native subtitle size controls; no installed-family picker |
| Floating subtitle controls | Shared settings/control-panel time preference; pause/resume is reachable in Immersive Mode. Keyboard-accessible collapse and native window drag/resize | Native collapse, font, language-route and immersive-entry actions; text-sized single-line labels reflow into two rows when width or system font size requires it |
| Reading long current subtitles | Bounded live tail with explicit history reading and return-to-live behavior | Expanded panel scrolls the complete bounded current pair together with optional history; compact/immersive keep two source and three translation lines |
| Audio capture | System audio by default; explicitly selected microphone or both sources with independent lanes. OS-specific desktop capture; selected-app audio on macOS and Windows build 20348+, Linux retains output-monitor capture | Android playback-capture consent and foreground service; no selected-app picker |
| Saved service selection | Settings, tray and subtitle controls can switch during listening via reconnect; pause and confirmed subtitles persist. A recording cannot switch PCM sample rate | Existing Android selection flow; no live-switch parity claimed |
| Proxy preferences | Per-profile independent recognition/text routes; integrated realtime uses one route | Platform network defaults; no per-stage proxy controls |
| Gemini planned connection rotation | Prepare one replacement after GoAway, bounded old-tail drain and up to two seconds of PCM staging; shorter Gemini live preview cadence | Shared continuous text rules; native replacement-socket handoff and desktop preview cadence are not implemented |
| Secret storage | Private local credential file; one-time OS-store migration | Android Keystore-backed encrypted preferences |
| Local HTTP | Existing loopback endpoint validation | Explicit per-config opt-in and Android network allowlist; includes emulator host |
| History/recording | Optional bounded local session files and selected-input recordings; confirmed panel rows survive stop/start within the app process and keep the existing display bound | Existing optional bounded subtitle history; capture starts clear its overlay; no desktop recording/export parity claimed |
| Development evidence | Exact dev app only; opt-in sent audio, subtitle snapshots, causal traces, saved-case playback and bounded evidence workspaces | No matching debugger or sent-audio recording claimed |

These differences are current scope, not proof of live-account acceptance. When expanding a
feature, update this table and the affected cross-platform fixtures instead of assuming the
other implementation already matches. Keep transport behavior shared while respecting each
platform's native UI, permissions and resource limits.

The shared UTF-8 limit governs reducer subtitle fields. Native independent text
adapters can reject content earlier; their request, result and HTTP body limits
are not unified by the shared-core extraction.

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
  on both sides when changing scheduling. Desktop-only draft/reconnect machinery must not
  be copied into a final-only Android flow without a product need.

Desktop subtitle controls support independent system/microphone subtitle colors,
background transparency, and optional local HH:mm:ss confirmation timestamps for
every input, including Immersive Mode. Settings and the floating panel share the
same time switch, which also takes effect immediately in system-only mode.
System-only mode hides the system source icon and keeps retained microphone rows
identifiable with a small icon (inline when time is off); the timestamp preference survives input
changes. Single-source output keeps its own color and a neutral bilingual original.
These are desktop presentation preferences; Android does not currently expose
matching source-color or time controls, or desktop microphone selection.

Android's expanded panel has a bounded reading viewport, with complete current
source and translation text inside its scroll area. Opening it locates the current
caption; a changed completed pair locates its beginning only while the user is
still reading the current-caption region. Draft updates retain the scroll position,
and reading earlier history is not interrupted. Completion uses the shared
`displayPairFinal` field. Optional history keeps its existing display limit;
expanded reading does not enable retention, and compact/immersive captions keep
their existing line caps. This native presentation policy is separate from the
desktop live-tail behavior and does not imply parity in capture, recording or
timestamp preferences.

Desktop offers a default-on **Show interim subtitles** preference, including live session changes. With it off, only final lines and confirmed pairs appear; bounded Stop-tail fallback still follows the existing desktop final lane. This affects presentation, not provider requests or accuracy. Android currently has no matching switch. Shared snapshots expose `displayPairFinal` on both platforms, and Rust/JNI fixtures distinguish a completed preview from an accepted final even without retained presentation history.

## Desktop interface languages

Desktop supports Simplified Chinese, Traditional Chinese, English, Japanese, German, Korean and French, with
regional system-language detection and in-place window updates. This is desktop
interface localization; Android UI resources remain platform-native and are not
expanded by this change. Recognition/translation language support is independent
of the interface language and still follows each provider's capabilities.
