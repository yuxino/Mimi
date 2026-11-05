# Optional Alibaba LiveTranslate experiment

Keep existing Alibaba profiles on their saved route. New desktop Alibaba
configurations default to LiveTranslate; ASR + MT remains selectable. This experiment adds an explicit profile choice for
`qwen3.8-livetranslate-flash-realtime` without restoring the old translation-mode
picker or changing existing profiles.

The shared runtime owns the protocol. The first iteration is audio-only,
16 kHz mono PCM16, text-only output, Beijing workspaces, automatic source
recognition, and Original and all 60 documented text translation targets. Independent MT settings stay
saved and return when the user selects ASR + MT again. LiveTranslate requires an
existing Workspace ID and a matching saved Alibaba key; missing IDs fail before
network access. No credentials are copied when changing the mode.

LiveTranslate 3.8 accumulates source `delta` and translated `response.text.delta`.
Text completion is a preview until `response.done` confirms completed status.
Source/translation identities must remain linked before accepting a final pair.
LiveTranslate ends with `session.finish` → `session.finished`. Original mode suppresses the displayed
translation, but the model can still generate billable translation output.

The shared client distinguishes that requested finish from an unsolicited
`session.finished`. An unexpected end stops the old reader before pending drafts
can be promoted, emits `live_translate_unexpected_session_end`, and uses the
existing finite transport recovery policy on desktop and Android. It does not
replay old audio or claim lossless continuation. Manual finish still drains the
normal tail; stop/pause and generation cancellation retain ownership. Reset the
finish marker when disconnecting so a reused client cannot silently stop on its
next unexpected end. Authentication/configuration failures stay non-retryable.
No fixed session lifetime is assumed: the current official server-event guide
documents `session.finished` only as the response to a client finish request.
The issue #329 report does not establish that this event caused its disconnects.

For 3.8 error events and failed-response error details, preserve only an exact
allowlist: `ServiceUnavailable`, `InternalError` and `internal_error` map to
`alibaba_realtime_service_unavailable`; known authentication aliases map to
`credential_authentication_failed`. Unknown, quota and parameter errors remain
non-retryable. Error messages/types are never forwarded. Unexpected end and
temporary-service recovery use the existing four-attempt schedule with a 600 ms
minimum delay, shared across both native drivers. This is reconnecting a new
provider session, not resuming an expired server session or replaying missed
audio. See the official [error codes](https://www.alibabacloud.com/help/en/model-studio/error-code)
and [server events](https://www.alibabacloud.com/help/en/model-studio/live-translator-server-events).

The exact `response_idle_timeout` code observed on a healthy quiet 3.8 channel
maps to `alibaba_realtime_idle_timeout`, with the same finite recovery and 600 ms
minimum delay. It occurred after about 900 seconds with both continuous zero PCM
and a later period without callbacks while the other lane kept translating.
This measured duration is evidence, not a hard-coded timer or a documented
service guarantee. Do not manufacture speech, add a periodic reset, retry
unknown codes, or confuse this error with a failed connection-health probe.

Apply the same finite classification before `session.updated`: setup rejection
must not erase authentication, permanent rejection or known transient reasons.
The portable client preserves typed setup failures; the desktop adapter and
Android JNI runtime consume their safe labels before deciding whether to retry.
Unknown, quota and configuration rejection stop even when they arrive during
recovery from an earlier transport/idle failure, and retain the new reason.

Desktop recovery success and a concurrent source failure share the generation
transition gate. Success may retire its task and recovery flag only while it
still owns the active generation; an already accepted source failure keeps its
retry handoff. A later independent failure can then queue a new recovery owner.
Manual stop or a replacement generation continues to win over stale completion.

An acknowledgement alone must not renew the retry budget. Preserve the same
shared `RecoverySchedule` across recovered connections that fail before stable
health is established, including the consumed attempts and backoff progression.
`RecoveryContinuity` in `mimi-runtime` owns this rule; native adapters report
connection/health/lifecycle events without copying it. A matching generation
resets continuity only after the existing `HEALTH_CHECK_INTERVAL` has elapsed
and its health check succeeds (both selected desktop sources must succeed).
This requires neither speech nor subtitle output, so a genuinely quiet but
healthy long session still receives a fresh recovery window for a later timeout.
The initial connection is not a retry; immediate success/failure cycles cannot
exceed the existing four recovery attempts. Explicit stop, start, manual retry
or a replacement configuration begins a new window; stale health/completion
must not clear or restore a newer generation's budget. Keep one movable schedule
and separate its lifetime attempt count from per-owner generation advancement.

The desktop selector lists **LiveTranslate (simultaneous)** first, while
keeping existing profiles on ASR + MT until explicitly
changed. An unfinished new profile may retain LiveTranslate with an empty workspace ID;
a valid ID is still required before starting a session or a connection check. Both options have
short descriptions: ASR + MT recognizes first and uses a separately selectable
text translator; LiveTranslate produces source and translated text in one model
session as audio arrives. Its help identifies the experimental limits. Fixed
Beijing console links beside the fields open API Key and workspace management.
The user flow has three steps: get an existing workspace ID and matching key;
select LiveTranslate, fill and save in Mimi; choose automatic recognition and
a supported translation target, then start a short subtitle test. The existing key may
be reused when it belongs to that workspace and has model-call access. The
workspace ID is copied from an existing workspace or the `llm-xxx` prefix of its
API Host; it is never inferred from a key or confused with a model ID. Mimi does
not create workspaces, keys, service activations, or permissions. User setup is
documented in [English](../provider-setup.md#alibaba-livetranslate) and
[简体中文](../provider-setup.zh-CN.md#alibaba-livetranslate).

The macOS manual probe reuses the production facade, the existing public-fixture
manifest validator and PCM pacing. A test-only read-only credential loader
prevents migration, normalization writes, and permission repair. It defaults to
one baseline arm; the LiveTranslate arm requires explicit selection and a current
budget gate. It sends at most one 20-second clip plus the same two-second silence
tail. Audio3 idle PCM is disabled for this comparison. No capture or native app
launch occurs. Provider acceptance, wire events, semantic quality and actual
visible subtitles remain separate evidence boundaries.

Current official references:
[LiveTranslate](https://help.aliyun.com/zh/model-studio/qwen3-5-livetranslate-flash-realtime),
[LiveTranslate server events](https://www.alibabacloud.com/help/en/model-studio/live-translator-server-events),
[Workspace hosts and regions](https://help.aliyun.com/zh/model-studio/regions),
[API Key management](https://help.aliyun.com/zh/model-studio/get-api-key).

The draft does not establish better subtitle quality, long-session stability,
Android selection UI, or Windows/Linux native acceptance. It does not authorize
merging, releasing, recording private audio, or additional paid batches.

## Input isolation experiment

The ignored `manual_live_translate_input_matrix` probe uses an explicitly selected
saved Dev LiveTranslate profile through the read-only loader. It feeds fixed
16 kHz public PCM into the production send queues and, for Both, the actual echo
processor. The same shared controller receives each source's provider events.
It never starts OS capture, changes settings, records audio or writes text.
One authorized run is bounded to 16 minutes of microphone speech plus a quiet
system lane (8 minutes of zero PCM, then 8 minutes without callbacks), followed
by 3 minutes each of microphone-only and system-only input. It reports only
source labels, byte/event/pair counts, elapsed times and safe failure codes.
The first failing lane is observed for up to 10 more seconds to determine whether
the other connection remains productive; this observation deliberately bypasses
desktop's whole-generation teardown, so it does not prove native recovery/UI.
No automatic extra paid run is authorized by retaining this ignored test.

A separately authorized speed manifest can instead feed three one-pass Russian
fixtures, at 1×, 1.25× and 1.5×, using a common source excerpt. It validates
three bounded inputs (at most 24m 1s each / 60 minutes of total PCM), preserves
all supplied samples under pacing jitter, then sends ten seconds of explicit
silence per case. Both sources stay selected and the system lane remains digital
silence throughout these speed comparisons. This is distinct from the earlier
no-callback input matrix. Content-free counters distinguish source and translation
events, Cyrillic/Han presence, final-pair follow latency, final input enqueue time,
server-finished acknowledgement, pacing gaps and existing slow-send/backpressure
trace labels. Follow latency remains source-final → translated-final, not
capture-to-screen latency or semantic-quality evidence. Additional long runs
require the current user's scope; retaining this test does not extend that scope.

The optional soak manifest schedules 2–3 hours of fixed public PCM, bounded to
eight clips, 160 MB and 200 phases. Each source independently selects speech,
digital silence or absent callbacks; reconnects join the current wall-clock
position instead of replaying missed audio. The driver reuses the production
client, health probes, queues, DSP, subtitle controller and finite recovery
classification. Stable streaming permits a new recovery window for a later
independent interruption. It reports per-phase progress and returns failure on
terminal errors, exhausted retries or an incomplete schedule. A no-network
preflight checks every scheduled second. This is a shared-service/DSP experiment;
the driver does not exercise the desktop SessionManager, native capture or
visible overlay, and cannot establish semantic quality from event counts.
