# Bounded preference saves and ASR finish ordering

## Scope and evidence

Baseline: `5d487431a1ec62ed89326a4af914a696fba6dbaa` (v1.6.4 main).
This work uses offline fixtures and background checks. Native windows, capture,
installed applications and private configuration are outside this run.

Two deterministic failures justify changes:

- A blocked preference save followed by 100 slider changes and a history opt-out
  produces 102 native saves. Every save writes preferences durably and broadcasts
  a settings snapshot. Intermediate slider values delay the final selection and
  subsequent ordinary preferences.
- An ASR bridge with an already processed partial draft can receive an updated
  draft and `SessionFinished` before its next poll. The reliable finish event
  overtakes the replaceable draft and makes it obsolete. Stop fallback then
  confirms the earlier partial text instead of the latest tail.

## Preference queue

Retain the in-flight write and coalesce consecutive, not-yet-started standalone
font-size/background-opacity drafts into one pending write. Track each field's
latest preview ownership independently. Ordinary or mixed drafts are ordering
barriers: coalescing cannot move a value across them. Preserve immediate local
preview, external-snapshot ownership, caller completion/error reporting and
failure rollback. Do not introduce timers or delay the first write.

This belongs in the existing desktop `SettingsSaveCoordinator`; native storage
and Android's separate preference UI do not need a second implementation.

## ASR finish barrier

Enable a receiver policy only for the shared high-quality client's ASR bridge.
Before acknowledging `SessionFinished`, deliver the latest still-valid earlier
recognition draft, if any. Retain at most one deferred finish event. Reuse the
queue's existing ordering, content-revision and final-supersession rules.

Other provider receivers keep their existing terminal behavior. A confirmed
final must still suppress its old draft, Clear must still discard old content,
and errors/overflow must retain their recovery semantics. Desktop and Android
receive this behavior through `mimi-runtime`.

## Verification

Rerun both original failing cases, then cover slider field interleaving,
ordinary/mixed ordering barriers, rollback and external events. For ASR, cover
draft identity, reliable final precedence, Clear and both receive APIs. Run the
canonical source checks and actual host-JNI coverage with bounded concurrency.
Record final results and native/provider limitations in the integration ledger.


## Custom recognition final-publication boundary

A later Android JNI run exposed a second boundary before the provider event
queue. The custom recognition reader retired a completed turn under its content
lock, then released that lock before publishing the authoritative result.
Concurrent finish could observe an empty turn queue, publish SessionFinished,
and invalidate the reader before its final event reached the queue. The earlier
receiver drain policy cannot protect an event that has not been published.

Retire the turn and publish its bounded result under the same synchronous
content lock. Release that lock before entering the protocol-error path, whose
event lock has other callers. No lock crosses an await; finish deadlines and
normal event-lane policies remain unchanged.

A controlled test pauses exactly between retirement and publication. Before the
fix, finish overtakes the empty completion. After the fix, empty, whitespace and
nonempty authoritative completions all precede SessionFinished. Real providers
and native capture are outside this deterministic loopback case; actual Android
JNI validation remains required.
