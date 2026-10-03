# PC and Android maintenance

Subtitle state, complete current pairs, DashScope item-identity pairing, append-only
OpenAI transcript alignment and final translation policy now have one implementation
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
- Complete subtitle text is preserved within the same UTF-8 byte limit; punctuation
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
- DashScope realtime pairs source and translation using conversation-item links;
  a known response without a source link cannot guess a source by arrival order.
  OpenAI append-only streams share timing/punctuation alignment and safe tail flush.
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
| Subtitle background | Adjustable card opacity (80% default); history does not fade with age | Existing native overlay background settings and history styling |
| Audio capture | System audio only in the current release; microphone selection temporarily unavailable (implementation retained). OS-specific desktop capture; selected-app audio on macOS and Windows build 20348+, Linux retains output-monitor capture | Android playback-capture consent and foreground service; no selected-app picker |
| Proxy preferences | Per-profile independent recognition/text routes; integrated realtime uses one route | Platform network defaults; no per-stage proxy controls |
| Secret storage | OS keychain | Android Keystore-backed encrypted preferences |
| Local HTTP | Existing loopback endpoint validation | Explicit per-config opt-in and Android network allowlist; includes emulator host |
| History/recording | Optional bounded local session files and selected-input recordings | Existing optional bounded subtitle history; no desktop recording/export parity claimed |
| Development evidence | Exact dev app only; opt-in sent audio, subtitle snapshots, causal traces, saved-case playback and bounded evidence workspaces | No matching debugger or sent-audio recording claimed |

These differences are current scope, not proof of live-account acceptance. When expanding a
feature, update this table and the affected cross-platform fixtures instead of assuming the
other implementation already matches. Keep transport behavior shared while respecting each
platform's native UI, permissions and resource limits.

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

Desktop subtitle controls additionally support keeping text opaque while backgrounds remain transparent and system subtitle color. The microphone color control is temporarily hidden with microphone input. These are desktop presentation preferences; Android does not currently expose matching controls.
