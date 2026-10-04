# PC and Android maintenance

Subtitle state, complete current pairs, DashScope item-identity pairing, append-only
OpenAI/Gemini transcript alignment and final translation policy now have one implementation
in `shared/mimi-core`. Desktop
imports the Rust crate; Android's Kotlin adapters call that same crate through JNI.
Provider transports and OS integration remain native. Their wire behavior is a shared
product contract and changes must still be verified on both platforms.
`shared/translation-contracts.json` holds synthetic request/response fixtures used directly
by both platforms' tests. Add an affected case there when changing an external text API;
update both implementations and their platform-specific tests in the same change.

## Shared behavior

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
  block checkpoint after 2 seconds without nonempty text, regardless of unequal
  sentence counts. This heuristic can delay confirmation during continuous speech;
  drafts stay bounded at 5,120 characters. Both platforms retain explicit turn
  boundaries with a shared 500 ms late-tail grace; interruptions discard buffers.
- Configuration changes are drafts until the explicit save action. Field labels and
  errors stay legible; protocol explanations use help controls. Provider artwork comes
  from the same existing desktop asset source.

## Deliberate platform differences

| Area | Desktop | Android |
| --- | --- | --- |
| Independent text services | DeepL, DeepLX, ChatMock, OpenAI compatible; original-only with custom ASR | Same text choices after Alibaba realtime ASR; original-only supported |
| Recognition selection | Eight built-in services plus custom DashScope/OpenAI ASR | Eight built-in adapters; independent text currently pairs with Alibaba ASR |
| Built-in Alibaba pipeline | Desktop Audio 3.0/Qwen-MT scheduling | Existing integrated realtime translation adapter |
| Translation scheduling | Speculative drafts plus serial prioritized finals and provider recovery | Final-only serial HTTP; same shared final bounds/retry decisions, native execution and cancellation |
| Independent text HTTP bounds | HQ source fields up to 65,536 UTF-8 bytes; response bodies up to 1 MiB; decoded text uses native adapter bounds | Source/result text up to 4,096 UTF-16 code units; response bodies up to 64 KiB |
| Subtitle background | Adjustable card opacity (80% default); history does not fade with age | Existing native overlay background settings and history styling |
| Floating subtitle controls | Shared settings/control-panel time preference; pause/resume is reachable in Immersive Mode. Keyboard-accessible collapse and native window drag/resize | Native collapse, font, language-route and immersive-entry actions; text-sized single-line labels reflow into two rows when width or system font size requires it |
| Reading long current subtitles | Bounded live tail with explicit history reading and return-to-live behavior | Expanded panel scrolls the complete bounded current pair together with optional history; compact/immersive keep two source and three translation lines |
| Audio capture | System audio by default; explicitly selected microphone or both sources with independent lanes. OS-specific desktop capture; selected-app audio on macOS and Windows build 20348+, Linux retains output-monitor capture | Android playback-capture consent and foreground service; no selected-app picker |
| Proxy preferences | Per-profile independent recognition/text routes; integrated realtime uses one route | Platform network defaults; no per-stage proxy controls |
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
background transparency, and optional local HH:mm:ss confirmation timestamps while
the microphone is selected, including Immersive Mode. Settings and the floating
panel share the same time switch, which remains available in system-only mode.
System-only mode restores plain subtitles and keeps any retained microphone rows
identifiable with a small inline icon; the timestamp preference survives input
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
