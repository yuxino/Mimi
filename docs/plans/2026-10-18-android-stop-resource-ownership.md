# Connection ownership across Android hard stop

The JNI adapter previously took a `Connection` out of its slot before awaiting
graceful finish or retirement. Hard stop aborted the drive task and started a
second retirement against that slot. If the first future already owned the
connection, the second saw an empty slot and could not disconnect its client.

The original production JNI adapter and shared LiveTranslate transport were
reproduced through a local CONNECT/TLS/WebSocket fixture. The fixture withheld
`session.finished`, observed hard stop after receiving `session.finish`, and
still received the matching Pong to a Ping more than 9.6 seconds later. A direct
hard stop before finish closed promptly. The fixture then closed the abandoned
socket explicitly. No provider, real credential, microphone or capture was used.

## One shared cleanup owner

The cancellation-safe `CleanupOwners` implementation and its existing tests
move from the desktop adapter into `mimi-runtime::resource_cleanup`. Desktop
keeps a thin re-export and retains its own native capture/admission orchestration.
No provider-specific Drop implementation or second registry algorithm is added.

The registry joins requests with the same nonzero key. Desktop deliberately uses
one key for an entire generation's two-source teardown. JNI assigns a distinct
`cleanup_id` to each actual connection, independently of its retry generation.
Different clients cannot lose a cleanup factory through accidental deduplication.
Each JNI Session also owns a separate registry, so a new handle is isolated from
an old handle's outstanding retirement.

## Lease before take

`take_connection_owned` holds the connection-slot mutex while cloning the client,
registering its final-disconnect factory, and removing the connection. There is
no await or unregistered owned connection between these steps.

The returned lease holds the Connection, a release sender and the cleanup ticket,
in that field order. Normal completion explicitly drops the Connection/pipeline,
then the sender, then awaits the ticket. Cancellation drops them in the same
order. The independent owner waits for release and runs bounded disconnect;
cancelling a drive, retirement or ticket waiter cannot cancel that owner.

The drive still owns graceful PCM drain, provider finish and tail-event
consumption. Normal retirement still samples MT request accounting once and
preserves it for retry before final disconnect. Hard stop cancels those waits
immediately; it does not wait for the full graceful-finish watchdog. The existing
disconnect deadline remains unchanged, and hard stop does not save a budget for
an already removed Session, matching the previous policy.

## Stop completion and startup publication

Hard stop returns an internal, awaitable ticket with its own cleanup lifetime.
It aborts and joins the registered drive, retires any connection remaining in
the slot, then waits for the Session's concrete disconnect owners. Its registry
is separate from the Session's registry, so it cannot wait for itself. Dropping
this ticket still starts completion. The public `stop` command continues to
remove the handle and return `{}` immediately; it does not block the JVM thread
on provider I/O.

Start publishes its Connection and drive while holding the same task-slot mutex
used by Stop, and rejects a retained Session that is already stopping. Stop's
join therefore covers synchronous late installation as well as an already
awaiting drive. Connection is installed before the first await of
`client.connect`, both at initial start and retry; cancelling setup can still
find and retire the allocated client.

The final-disconnect owner changes no controller state, subtitles, source
identity, recovery schedule or wire protocol. Late completion on a removed
handle cannot publish over a new Session.

## Regression boundaries

Maintained Rust coverage includes the shared owner's ten existing cases and
six JNI cases: an unpolled lease, distinct clients sharing a lifecycle
generation, cancellation after take, synchronous installation during abort,
request-budget continuity on retry, and rejecting a retained stopped Session.
Existing native reliable-tail, PCM, handle and shared two-source tests remain.

Four maintained Kotlin/JNI loopback cases exercise the real shared client:

- hard stop after the server sees `session.finish`, requiring Close/EOF within
  three seconds, a stale stopped handle, and an unchanged second, idle handle
  that has only been created and polled;
- direct hard stop while Listening;
- stop after `session.update` while the readiness acknowledgement is withheld;
- normal graceful finish retaining the provider's final subtitle pair.

The CONNECT/TLS fixture intercepts the unchanged production endpoint only on
loopback, generates a temporary test identity, and injects its trust root through
the existing API. Post-stop subtitle events are not used as a liveness probe:
a failed event send could make the abandoned receiver exit and conceal the bug.
Close/EOF is asserted before fixture shutdown, so cleanup cannot make a failed
case pass.

Local Rust tests and a Java caller of the real JNI exports supplement the
maintained Kotlin tests. The Java/TLS comparison retains the old native library
as a negative control: old finish/stop stayed connected for 9621 ms and replied
to Ping; the fixed library closed the same case promptly. Automated loopback
tests do not establish signed-device capture, provider billing, or the behavior
of every provider when its existing disconnect deadline expires. Full Android
JVM/CI and desktop native checks remain separate verification gates.

## Local verification

The implementation was tested as a dirty worktree based on `ffddc20`; the old
JNI adapter used for the negative control is unchanged between `a1fe131` and
that base. The real Java/JNI caller passed direct stop, finish/stop, withheld
setup acknowledgement and graceful-tail cases. A second created/polled handle
remained unchanged; this does not prove isolation between two active sockets.

The JNI Rust suite passed all 14 tests. Shared runtime passed 797 tests with one
existing ignored case on a two-test-thread run, including all ten moved owner
tests. The first default-parallel run had 796 passes and one failure in the
existing xAI ambiguous-following-item test: its 50 ms idle observation ended
before the expected recovery event. The isolated rerun and the complete
two-thread rerun passed without modifying the test or its timing threshold.

Removing Stop's join in a temporary copy of the production adapter made the
formal late-installation test fail. Reusing the lifecycle generation as the
cleanup key made the formal independent-owner test fail; even without its
explicit key assertion, the completion waiter returned before the second
connection was released. The temporary copies are supplementary negative
controls, not maintained substitute tests.

Runtime/JNI strict all-target Clippy, all three affected Rust format checks,
the shared architecture check and diff checks passed. A no-GTK driver using the
actual shared runtime and desktop's thin re-export passed all nine desktop
recovery orchestration tests. The moved owner's implementation differs from the
old desktop module only in six public visibility changes and five documentation
lines; its ten tests are unchanged. This host lacks the Android Kotlin/Gradle
toolchain and desktop GTK development packages. The four maintained Kotlin/JNI
tests and full desktop build therefore remain CI gates; the local Java caller
does not claim those suites passed.

Root acceptance rebased the unchanged implementation onto `90fa8cb`. The
no-argument canonical check passed core 93 plus four contract cases, runtime
797 (one existing ignored), Rust JNI 14, updater four, formatting, strict shared
Clippy and architecture checks. Desktop compilation then stopped at the host's
missing `glib-2.0.pc`; full desktop and maintained Kotlin/JNI execution remain
the exact-head CI merge gate.
