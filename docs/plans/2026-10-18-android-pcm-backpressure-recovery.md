# Android PCM ingress backpressure recovery

## Confirmed failure

The shared audio pipeline deliberately closes a generation's ingress when its
bounded queue fills. It returns `Backpressure` exactly once, followed by `Closed`.
Desktop capture adapters report that first failure to their session manager. JNI
previously folded both errors into `false`, and Android's capture adapter ignored
that Boolean. A live connection could therefore remain Listening while every
subsequent PCM buffer was rejected.

A real Java caller of the production JNI exports reproduced this with a local
CONNECT/TLS/WebSocket fixture. It withheld the initial setup acknowledgement for
about 2.3 seconds while submitting 100 ms PCM chunks: 21 were accepted and the
22nd was rejected. After setup completed, the accepted buffers drained, but five
later buffers were all rejected. The state remained Listening with no error or
finished flag; a matching Ping/Pong confirmed that its WebSocket was still alive.
No real provider, credential, audio capture or external network was used.

## Connection-scoped forwarding

The JNI adapter now acquires `AudioIngress` and its `ProviderEventSender` together
under the Connection-slot mutex. Its native `ConnectionIngress` endpoint forwards
only typed `Backpressure` as the existing retryable `transport_error`. The shared
pipeline still owns the queue capacity, fail-closed policy and once-only failure
transition. No new recovery state machine or queue is added in Kotlin.

The endpoint retains the sender belonging to the same actual Connection. A late
submission cannot look up the replacement connection and report an old failure
to it. Repeated submissions after the first overflow return `Closed` silently;
normal stop, retired endpoints and missing ingress do not request new recovery.
The existing drive and cancellation-safe cleanup owners retire resources before
installing the next connection.

## Scope and limits

During setup, the drive awaits `client.connect` rather than consuming provider
events. If setup succeeds after the queue overflow, it then consumes the queued
error and reconnects. If setup fails or times out first, its existing connection
failure policy applies. This change does not preempt setup or reduce its timeout.

The regression delays only the first setup; the replacement acknowledges promptly.
If every setup exceeds the queue budget while capture continues, later generations
can overflow again. Each successful connect resets the existing recovery schedule,
so this fix does not promise a global retry-count limit for that scenario. It
prevents silent permanent PCM rejection after transient congestion; it does not
retain over-budget PCM without loss or improve slow setup latency.

## Regression protection and local results

The worktree is based on `85c14105038dba57512e3f2f74fb5d88b964b72f`, including the
preceding JNI hard-stop ownership fix. The first two new Rust regressions failed
before forwarding was added because the provider event receiver stayed empty.

Four maintained Rust cases cover once-only retryable reporting, an old endpoint
reporting only to its own connection, silent Closed after hard stop, and the real
native PCM/drive/shared-client loopback path. The last case fills the queue during
setup, observes the same handle open a second connection, sends fresh voiced PCM
through that connection, rejects the retired endpoint and checks Stop opens no
third socket. All 18 JNI Rust tests passed, including existing reliable-tail,
request-budget and cancellation regressions.

A maintained Kotlin/JNI loopback case covers the equivalent LiveTranslate path
with actual production TLS routing: one handle, two setup exchanges, five fresh
PCM marker frames on the second socket, Close within three seconds of hard stop,
a stale stopped handle and no third connection during a 500 ms accept window.
Existing direct-stop, setup-cancellation, finish-cancellation and graceful-tail
cases remain. The fixture checks Close before its own shutdown.

The actual Java/JNI probe failed on a fixed copy of the old library: no second
connection arrived within five seconds after the overflow. With the fixed debug
library, the same probe opened two connections, delivered all five fresh frames
(16,000 bytes), and received Close 1 ms after Stop. The four earlier Java/JNI
stop/tail controls also passed on this library. These timings describe a local
synthetic fixture, not Android-device or real-provider latency.

JNI strict all-target Clippy, Rust format, shared architecture and diff checks
passed. Kotlin/Gradle is unavailable on this host, so the maintained Kotlin case
has not been compiled locally. Root's no-argument canonical check passed core
93 plus four contracts, runtime 797 (one existing ignored), Rust JNI 18, updater
four and shared formatting/strict Clippy/architecture checks, before desktop
compilation stopped at the missing `glib-2.0.pc`. Rebasing onto `922bbc5` preserved
the tested shared/JNI/Kotlin source. Exact-head desktop and maintained Kotlin/JNI
execution remain platform CI merge gates.
