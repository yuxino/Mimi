# PC and Android maintenance

Provider behavior is a shared product contract, even though desktop uses Rust/Tauri and
Android uses native Kotlin. Fixes do not transfer automatically between those codebases.
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
- Final work keeps its own immutable source, has bounded queues and a 45-second total
  deadline including waiting, and is cancelled on stop. Late results cannot enter a new
  session. Recognition previews cannot become durable translation history.
- Configuration changes are drafts until the explicit save action. Field labels and
  errors stay legible; protocol explanations use help controls. Provider artwork comes
  from the same existing desktop asset source.

## Deliberate platform differences

| Area | Desktop | Android |
| --- | --- | --- |
| Independent text services | DeepL, DeepLX, ChatMock, OpenAI compatible; original-only with custom ASR | Same text choices after Alibaba realtime ASR; original-only supported |
| Recognition selection | Eight built-in services plus custom DashScope/OpenAI ASR | Eight built-in adapters; independent text currently pairs with Alibaba ASR |
| Built-in Alibaba pipeline | Desktop Audio 3.0/Qwen-MT scheduling | Existing integrated realtime translation adapter |
| Translation scheduling | Speculative drafts plus prioritized finals and provider recovery | Final-only serial HTTP, one active and four waiting; explicit stop on failure |
| Subtitle background | Adjustable card opacity (80% default); history does not fade with age | Existing native overlay background settings and history styling |
| Audio capture | OS-specific desktop capture | Android playback-capture consent and foreground service |
| Secret storage | OS keychain | Android Keystore-backed encrypted preferences |
| Local HTTP | Existing loopback endpoint validation | Explicit per-config opt-in and Android network allowlist; includes emulator host |
| History/recording | Optional bounded local session files and selected-input recordings | Existing optional bounded subtitle history; no desktop recording/export parity claimed |

These differences are current scope, not proof of live-account acceptance. When expanding a
feature, update this table and the affected cross-platform fixtures instead of assuming the
other implementation already matches. Keep transport behavior shared while respecting each
platform's native UI, permissions and resource limits.

## Verification and change review

- Run `./scripts/check.sh` for desktop, including the shared Rust fixture tests.
- Run Android debug/release unit tests, lint and APK builds; Kotlin reads the same JSON from
  its test resources. Native instrumentation covers draft/save/key isolation and local HTTP.
- Shared contracts or provider changes trigger both CI paths. UI-only platform edits keep
  their usual checks. CI never substitutes for an actual provider/account/device session.
- Review queue bounds, lock ordering, cancellation, stale generations and final deadlines
  on both sides when changing scheduling. Desktop-only draft/reconnect machinery must not
  be copied into a final-only Android flow without a product need.
