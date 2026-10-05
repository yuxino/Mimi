# Provider authentication and safe recovery errors

The Gemini, Azure OpenAI, Tencent, Baidu, Volcano and xAI clients reduced an HTTP
401/403 WebSocket handshake rejection to a generic transport failure. Connection
diagnostics therefore lost the actionable authentication category, and session
recovery could present a configuration problem as a connection retry. The frontend
also lacked mappings for several fixed client errors, leaving safe, actionable
failures on the generic fallback. Adding these mappings must not accept arbitrary
provider text because it contains a familiar fragment.

## Decision

- Preserve handshake authentication rejection in all six clients with a typed
  `AuthenticationFailed` variant. Reuse the existing status classifier; only HTTP
  401/403 take this path. Its display label is exactly
  `credential_authentication_failed`, matching the established OpenAI and Alibaba
  transport behavior.
- Map those variants to `ConnectionCheckReason::AuthenticationRejected`. Retain
  the existing settings-led recovery for authentication errors. Do not claim a
  rejected credential is necessarily mistyped: authorization can also depend on
  service activation, model access or account permissions.
- Keep timeouts, HTTP 429/500, connection failures and setup-protocol rejection on
  their existing paths. Do not broaden authentication matching to generic message
  substrings or expose handshake response bodies.
- Extend `connectionDiagnostics.ts` with exact safe labels for Alibaba Live,
  OpenAI, Gemini, Azure, xAI, Tencent, Baidu and Volcano: transport/not-connected,
  health/connect/setup timeout, setup/session rejection, missing credentials and
  unsupported language. Azure's fixed endpoint/deployment labels lead to
  configuration guidance. Fixed protocol failures and unexpected session endings
  use the existing translation-unavailable/reconnect message without guessing a
  lower-level cause.
- Parse Baidu's setup-code label only as its complete fixed template with a
  canonical signed 64-bit integer. The integer alone is safe to retain; no more
  specific provider-code meaning is asserted. Unknown or malformed labels,
  including extra prefixes/suffixes, use the generic safe fallback. Localized UI
  text remains owned by Mimi; provider bodies, URLs and credentials must never
  become visible error prose.
- Keep the shared settings, tray, overlay and overlay-control error presentation
  consistent. Exact authentication, missing-credential, invalid-configuration and
  unsupported-language labels open service settings and hide blind retry. Timeout
  and unclassified rejection errors, including Baidu's numeric setup codes, retain
  retry. The compact overlay must not lose the settings recovery entry. Include
  the exact custom-ASR authentication, missing-credential, invalid-endpoint and
  invalid-model labels, plus independent translation's missing-credential label,
  in the same settings route; timeout and generic rejection labels remain retryable.
  The session recovery guard also stops automatic retries on those five exact
  custom/independent labels and preserves the actionable failure for repair.
- Classify the exact `baidu_unexpected_session_end` code and its fixed native
  message in content-free support diagnostics; private suffixes remain unknown.
  This closes a metadata gap and does not establish why a real Baidu session ended.
- Apply the same HTTP 401/403 classifier in Android's independent streaming,
  OpenAI and DashScope engines. `MimiService` preserves the exact safe label for a
  Chinese/English/Japanese settings-led credential/permission message; other
  errors retain generic feedback. Shared `translation-contracts.json` handshake
  fixtures define 401/403 versus transport behavior for Rust and Kotlin tests.
  No wire-protocol, shared reducer or JNI behavior is changed.
- Give `app-overlay-control` the `session_start` permission used by its explicit
  retry button. Keep `session_stop` unavailable to that window. On the installed
  `6ac14bde` build, Baidu reported `BAIDU_UNEXPECTED_SESSION_END` after 63.153
  seconds; clicking retry left the session in error with a generic action failure
  and no `start_requested` diagnostic. The callback invoked `session_start`, but
  the window's Tauri ACL omitted it, so the request never reached the lifecycle
  handler. Cover the actual frontend callback/store/IPC/native permission contract
  in addition to mocked UI clicks. This fixes retry dispatch, not the cause of
  Baidu's end event; native acceptance follows the rebuilt bundle.
- Preserve transcript pairing, provider prompts, language catalogs, capture and
  streaming-generation ownership.

## Verification boundary

The focused Rust handshake run passed 8 tests after the shared fixture update.
Six tests exercise each real client
connect path against loopback HTTP 401, 403, 429, 500 and a connection closed without
an HTTP response; the remaining cases cover classification. They assert the fixed
label, readiness remaining false and no response-body leakage. They use synthetic
credentials and do not contact cloud services.

Support-diagnostic Rust tests passed 13 cases, including exact Baidu code/message
classification and rejection of private suffixes. A frontend focused run
passed 141 tests for diagnostics, shared feedback and tray behavior, including
three-language recovery actions, settings navigation, strict label rejection and
Baidu integer bounds. ESLint, TypeScript and diff checks passed for those changes.
A separate 191-test baseline covered overlay projection, timeline scrolling and
streaming, snapshots, bootstrap and shared error feedback.

The initial missing-Java environment issue was resolved using temporary official
tooling. Android `testDebugUnitTest` finished with `BUILD SUCCESSFUL`: 19 suites /
129 tests, zero failures, errors or skips. The new handshake test and 2 feedback
tests passed, along with 2 `SharedSubtitleCoreJni` tests and 11 shared translation
tests. The host JNI library was actually rebuilt successfully. XML and shared JSON
parsing also passed. Reusable tool caches remain available. Android physical-device
and Release validation have not been performed.

The [native observations](../development/provider-regression-matrix.md) used
`d58f4bad`, before these patches: Alibaba, Gemini and Apple Speech + DeepL produced
fresh ASR/MT through the final sample sentence after resume/replay. Baidu displayed
six confirmed groups, then a generic failure after about 73.7 seconds connected;
its cause remains unproven. Whisper + Index also reached the final sentence, with
no pause/resume case or measured end-to-end latency. Parakeet's separate
`fbf3c06c` availability check returned unavailable without starting capture.
These were not full multiwindow snapshot traces.

The final canonical desktop check passed: 1,168 Rust tests (2 ignored), the shared
Rust/JNI checks, 1,730 frontend tests across 119 files, lint, typecheck, production
build and diff checks. The signed canonical app was rebuilt from clean `5b29d05e`.
After Baidu's real unexpected END, the overlay-control Retry now reached backend
`start_requested` and returned to listening in 929 ms. Pause/resume also reached
the backend, with resume returning to listening in 805 ms. After replaying the
same sample, the latest source/translation reached its last sentence; the diagnostic
snapshot was listening with 11 aggregate confirmed-pair events. The original ACL
failure did not recur.

Baidu still ends a session without a local FINISH; its service-side reason remains
unknown. A prior trace showed successful silent PCM sends through 1.146 s before
END, so a 30-second audio-send gap is not established. Do not add speculative
keepalives or treat a successful manual retry as proof of long-session stability.
Automatic END recovery was not added: the existing retry limit bounds consecutive
connection failures, not repeated successful connections followed by END. Android
physical-device/Release and Windows/Linux native behavior remain unverified.

## Volcano native follow-up

The native Volcano run on clean `5b29d05e` established pause/resume and bounded
four-window consistency, then exposed a later silence-period failure after a
cold launch. The support snapshot reduced its existing client error to `OTHER`.
A successful sample and restart do not establish continuous-session stability.

Preserve only exact Mimi-owned Volcano error labels and a canonical bounded
numeric provider status in support diagnostics. Serialize that status separately
from the static category/code; do not retain free-form messages, response bodies,
keys, session identifiers or endpoint URLs. Malformed numeric labels and private
prefixes/suffixes retain the unknown fallback. This is diagnostic coverage only:
provider framing, capture and recovery policy remain unchanged until the actual
server response has been observed. See the provider regression matrix for the
measured interval and outstanding live verification.

The same native run exposed a provider-picker navigation lock: opening the
picker while idle and then starting subtitles disabled both provider creation
and Cancel. Keep creation blocked for active, paused, connecting and stopping
sessions, show a short translated stop-first instruction, and allow Cancel
when no creation request is pending. Preserve the pending-request guard for
both the picker and its provider confirmation.
