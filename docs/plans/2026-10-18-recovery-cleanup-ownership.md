# Recovery teardown ownership

This change fixes desktop `SessionManager` cancellation at the capture/client
teardown boundary. It is based on `10d726e`; the implementation and the tests
below describe this worktree, rather than an already released build.
Before submission it was rebased onto `8ec3d62`, preserving the already merged
snapshot-copy optimization without changing the tested teardown helpers.

## Confirmed failure

The old recovery task released the lifecycle lock, then awaited native capture
stop and provider disconnect in its own future. Manual Start could abort that
future. Cancellation before the client was taken left the old generation in the
shared client slot, so the replacement failed with an occupied-client error.
Cancellation after taking the client could interrupt disconnect while the slot
already appeared empty. Clearing `capture_generation` before awaiting the native
stop also made an empty generation tag insufficient evidence that stop finished.

A Tokio reproduction extracted the actual manager methods, with gates replacing
native capture/provider I/O. It retained generation 71's connected client and
pump after a replacement generation's startup rollback. No provider connection,
audio capture, microphone permission or credential was used.

## Ownership contract

- Register a generation's entire hard teardown synchronously, before exposing
  recovery, invalidating a failed startup, or releasing lifecycle admission.
  Registration owns a teardown guard even before the returned ticket is polled.
- The first poll or Drop of a ticket starts an independent Tokio owner. Aborting
  a waiter cannot drop its native-stop/disconnect future or a client already
  removed from its slot. The existing native-stop/disconnect deadlines remain.
- Requests for the same in-flight generation join one owner. A second request
  cannot infer completion from already-cleared resource tags. Guards and pump
  finalizers from all joined requests remain owned until work completes.
- A completion-phase registration creates another owner. Registry removal checks
  the owner id so a completed owner cannot erase that replacement, including
  when settings rebuild a connection within the same lifecycle generation.
- Pump `Preserve` keeps the provider event task alive to finish its recovery
  handoff. `Abort` adds a synchronous finalizer after resource release. A later
  Abort request joining a Preserve owner must retain that finalizer. All
  finalizers and guard drops precede the completion notification.
- Cancellation takes the recovery task out of its slot, releases the ordinary
  mutex, aborts and joins that task, then waits for registered concrete owners.
  It must not wait for the global teardown count: terminal/capture callers can
  already own a guard themselves. Owner work never acquires the lifecycle or
  subtitle-content locks.

Startup's direct capture-stop and disconnect error paths use this same complete
generation owner. A source-only factory must not occupy the generation registry
and cause a later whole-selection rollback to skip the other source.

## Start, Stop and terminal events

An admitted manual Start retains its recovery-replacement intent while waiting
for old teardown. The old recovery can otherwise clear its recovery flag before
Start's second gate, leaving both tasks exited with Connecting and no client.

After obtaining the lifecycle lock, a replacement Start verifies its request
epoch under the generation-transition lock, clears any raced recovery attempt's
active generation, and reserves its teardown before any await. Clearing the
active generation prevents the old connect future from installing late resources
after cleanup inspected its slots. Start then aborts/joins recovery, waits for
the owner, and checks its epoch again. Activation also verifies the epoch under
the transition lock, so a Stop requested during cancellation cannot be undone
by a late Start activation.

Normal healthy Stop still owns capture stop, PCM drain, provider finish and tail
event drain. Cancelling recovery when no hard-teardown owner is registered does
not sweep healthy resources. A terminal provider event clears its generation's
active settings synchronously after invalidation: a settings-rebuild Abort
owner may stop that event pump while it later waits for teardown.

## Desktop and Android boundary

Shared retry/backoff, error classification, MT budget continuity and provider
behavior remain in `mimi-runtime`. The new ownership mechanism coordinates
desktop `SessionManager` task aborts with reused native capture workers,
generation-tagged client/pump slots and lifecycle command admission; it stays in
the desktop adapter.

Android creates a fresh native handle/`Session` for a new engine. Its Rust
`drive` loop awaits retirement before constructing a retry, and it has neither
desktop's reused capture slots nor the two-gate manual Start path. Copying the
desktop manager's admission/teardown registry into Kotlin would recreate adapter
logic without addressing a shared provider rule.

This does not establish cancellation safety for every Android teardown.
Android hard stop removes the handle, aborts `drive`, and separately spawns
`retire`; interruption after `drive` has already taken a `Connection` remains a
separate boundary to audit. If that network-resource case is confirmed, its
common ownership behavior should be implemented once in shared Rust and tested
through JNI, rather than duplicating desktop native-capture coordination.

## Regression evidence and limits

The committed production helper modules contain 19 Rust regressions:

- 10 owner tests cover unpolled-ticket Drop, cancellation with a taken resource,
  joined guards/finalizers, Preserve/Abort, same-generation completion reentry,
  concurrent generations, and completion after factory/work/finalizer panics.
- 9 lifecycle tests cover registration before advertising recovery/unlocking,
  capture-stop and taken-client cancellation with Stop holding lifecycle and
  Start queued, preserving healthy resources for graceful Stop, caller-owned
  teardown guards, cleared recovery flags, raced recovery attempts, and Stop
  superseding Start before or during cancellation.

All 19 passed locally using the actual repository modules and actual
`TeardownOperationGuard`/`lock_after_operations`, without GTK. The temporary test
entry point only supplied a minimal crate around those production files.
Clippy with `-D warnings`, desktop Cargo fmt, the shared architecture check and
`git diff --check` passed. Four temporary negative controls were rejected by the
committed tests: restoring the old second Start gate, retaining the retired
active generation, ignoring a superseded request during retirement, and
ignoring it during activation.

A second temporary crate extracted actual manager resource methods, the full
`recover_connection`, the Start admission path and the real shared
`RecoverySchedule`. Three gated cancellation cases and the forced
old-recovery-first/cleared-flag case passed, together with the 19 committed tests
(23 total). The Start scheduling gate represents a possible thread preemption;
the desktop history/configuration steps and native/provider effects were stubs.
These temporary extraction drivers are supplemental evidence, not maintained
product test runners.

`./scripts/check.sh --quick desktop recovery_cleanup` passed fmt but could not
compile the desktop test binary because this Linux environment lacks
`glib-2.0.pc`. Desktop tests/clippy on the configured CI platforms, and signed
native device validation of cancellation during capture stop/disconnect, remain
required. No native-device, real-provider, audio or end-to-end latency result is
claimed by these offline tests.
