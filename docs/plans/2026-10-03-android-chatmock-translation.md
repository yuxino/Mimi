# PC and Android text translation parity (#90)

## Scope and decision

ChatMock exposes text Chat Completions, not Mimi's realtime audio WebSocket protocol.
Android adds an independent text stage after Alibaba realtime ASR: DeepL, DeepLX,
OpenAI-compatible/ChatMock, or original-only. Desktop adds explicit ChatMock compatibility
to its existing OpenAI-compatible text route. Shared protocol fixtures and coordinated CI
keep both implementations aligned. Existing integrated services keep their defaults.

A URL override alone cannot work. Overriding translations for every existing speech service
would still pay for their integrated translations and imply untested recognition capabilities.
The initial pairing therefore uses Alibaba's transcription-only model explicitly.

## Configuration and UI

Alibaba's editor separates speech credentials from text translation. Built-in integrated
translation remains the default. Address, model and optional bearer key are independent, encrypted, and written only
by Save and use with the speech configuration. A changed destination cannot reuse an old key.
Back/cancel never saves a draft. Nonessential compatibility, address and storage guidance lives
in help dialogs reached by small accessible help icons (also hover/long-press tooltip).
No new persistent small-print paragraphs. A check sends a fixed non-private translation
request and reports its elapsed milliseconds beside the action; edits cancel stale checks.
Chinese, English and Japanese resources remain aligned.

## Transport and session rules

Accept a base URL ending in /v1 or the complete /chat/completions URL. HTTPS is the normal
transport. Only localhost, 127.0.0.1, ::1 and the emulator host 10.0.2.2 may use HTTP, with explicit
unencrypted-connection opt-in. Android network-security-config matches that allowlist and
disables all other cleartext traffic. LAN and public endpoints require HTTPS. No URL
credentials, query tokens, fragments or redirects. Existing speech transports remain secure. ChatMock itself is operated
and authenticated by the user; Mimi never handles ChatGPT login credentials.

Use non-streaming Chat Completions with a bounded response, finite timeout, cancellable
requests and a small serial final-only queue. Translation stays paired with its source.
Ignore late callbacks after stop/restart. Empty, malformed, oversized responses, queue overflow
or transport errors fail explicitly without logging provider text or secrets. Strip complete
leading ChatMock think-tag blocks; incomplete reasoning must not become visible captions.

## Verification

Unit tests cover ASR-only setup, existing integrated setup, HTTP contract, URL restrictions,
optional authentication, reasoning/empty/error responses, queue order and cancellation.
Android debug/release unit tests, lint and APK builds plus the repository check are required.
Credential-free emulator checks cover draft cancellation, validation, help, independent fields,
local HTTP opt-in and light/dark layout. Protocol fixtures do not establish a live ChatMock
account or physical-device capture result; those limits remain explicit in the PR.

## Sources

- https://github.com/yuxino/mimi/issues/90
- https://github.com/RayBytes/ChatMock (reviewed main 5bd6e774648a107c989d5df9e360ac7dc8869370)
- https://help.aliyun.com/zh/model-studio/qwen-real-time-speech-recognition

## Desktop text-service parity

The text selector now offers the existing Alibaba integrated translation, DeepL, DeepLX,
OpenAI-compatible/ChatMock, and no translation (original text only). These match desktop's
independent text services; Qwen-MT remains part of desktop's built-in Alibaba pipeline and
is not presented as another independent credential provider on Android. Existing eight
speech adapters are unchanged. Separate text translation continues to use Alibaba ASR;
this does not imply new independent recognition support for the other integrated services.

DeepL uses its official HTTPS API, choosing Free for keys ending in `:fx`, with a required
independent key. DeepLX takes a user endpoint plus optional Bearer token, using the same
explicit loopback HTTP allowance as ChatMock. Fields and help reflect each protocol;
DeepL has no editable endpoint/model, DeepLX has no model. Each network provider has a
real fixed-example check and elapsed request duration. Only Save and use persists the
selected provider and its draft. Switching services preserves their own encrypted
configurations; keys never cross provider or destination boundaries. Existing ChatMock
settings remain readable and migrate atomically on the next explicit save.

Original-only mode uses ASR without an HTTP translation request, retains only confirmed
source history when opted in, and displays the home/appearance preview without a fake
translation or active target-language selector. Requests and subtitle queues retain the
same bounds, cancellation behavior, and content-free diagnostics.

Validation adds exact DeepL/DeepLX protocol and loopback transport tests, credential
isolation/migration and draft tests, native editor screenshots, and existing ChatMock
regressions. No real DeepL/DeepLX account or physical-device result is inferred from
these fixtures. Historical validation is recorded in PR #109.

## Scheduling and desktop ChatMock parity

Android retains final-only scheduling: one active request and four waiting sentences.
Unlike desktop, it has no speculative draft translation or automatic provider reconnect.
Port the relevant final deadline: 45 seconds from enqueue through completion, including
queue time. An active watchdog cancels overdue work and clears the queue; completion also
checks age so a delayed timer cannot publish stale text. Cancelled timers are removed to
release their captured source promptly. Session-generation checks and immutable source
pairing remain in place. Deterministic clock/scheduler tests cover queue time, stop,
overflow, expired completions and timer cleanup. Request startup and cancellation share
the lifecycle lock, so stop/deadline completion cannot race with a later HTTP enqueue;
synchronous callbacks defer cleanup until outside that lock.

Desktop's existing OpenAI-compatible text entry also supports ChatMock: an optional
translation key, no Authorization header without one, filtered complete leading reasoning
blocks, and explicit rejection of incomplete final responses. Keep its existing scheduling,
translation prompt and localhost HTTP exception unchanged. Desktop credentials use
the [private local-file design](2026-10-04-local-credential-storage.md). A missing
finish_reason remains accepted for existing compatible services; explicit non-stop reasons
are rejected. A separate explicit clear-token flag distinguishes removing a saved key
from leaving the write-only input blank to retain it. Requirements stay in the existing
help tooltip in all three UI languages.

Android service lists and editor headings reuse all eight desktop provider brand marks;
text-service options reuse the desktop marks or semantic generic icons. Decorative assets
have consistent visible bounds; long explanations belong in standard help dialogs.

## Unified maintenance

`shared/translation-contracts.json` is consumed directly by Rust and Kotlin tests. It fixes
request shapes, response rejection/cleanup, endpoint normalization, optional Authorization,
model UTF-8 byte limits and credential Unicode-scalar limits in one place. Provider/shared
changes trigger both CI paths. See [platform parity](../development/platform-parity.md)
for the maintained feature table, intentional OS differences and required review checks.


## Separate ChatMock preset

Expose **ChatMock** and **OpenAI compatible** as separate text translation choices on
both platforms. Both reuse the existing Chat Completions transport, scheduling,
response cleanup and validation. A new ChatMock draft pre-fills
`http://127.0.0.1:8000/v1`; the model remains required and has no assumed default.
Saved destinations are never replaced by that preset on reopen. Selecting a choice
only edits a draft; saving remains explicit.

Persist the distinct `chatMock` route and keep its destination credentials separate
from `openAICompatible`. Existing combined-entry configurations remain under the
OpenAI-compatible route without inferring a service brand, migrating secrets, or
changing their endpoint. Unsaved destination data never crosses choices: desktop clears it on a choice
change, while Android keeps separate in-editor drafts until save or cancellation.
Neither behavior changes speech credentials. Both routes share cancellation
and final-priority translation behavior, while their budget contexts stay isolated.

Use a compact help icon for setup and requirements. Explain that localhost refers
to the device running Mimi: the PC itself on desktop, and the phone/emulator on
Android. Connecting Android to a computer requires that computer's accessible HTTPS
service or an explicit USB port reverse; never silently widen HTTP exposure or
switch on the existing local HTTP allowance. Keep Chinese, English and Japanese in
sync. Tests cover default filling, cancellation, preserved saved endpoints, separate
storage and route switching, plus shared real-loopback protocol coverage.
