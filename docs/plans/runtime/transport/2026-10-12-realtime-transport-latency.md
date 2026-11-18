# Realtime WebSocket transport latency

**Goal:** Send live-provider frames without avoidable TCP small-packet coalescing,
and avoid Audio3 receive-loop writer waits for ineligible idle-silence work,
while preserving the selected route, protocol and subtitle behavior.

**Architecture:** Desktop and Android create provider WebSockets in
`shared/mimi-runtime/src/clients/transport/provider_network.rs`. Apply one low-latency
TCP policy there immediately after physical connection, before any proxy tunnel,
TLS handshake or WebSocket upgrade. Audio3 also checks the existing pending-PCM
gate before awaiting its writer: queued/in-flight real PCM makes idle silence
ineligible. Retain the post-lock gate check to close the false-to-true race.
Keep the immutable route and native trust snapshot; a socket configuration error
uses the existing sanitized transport error.

**Tech stack:** Tokio TCP, tokio-tungstenite/rustls, the existing shared Rust
runtime and Android JNI adapter; no new dependency or saved setting.

## Evidence and choice

At source `7f851953`, the WebSocket transport does not enable `TCP_NODELAY`.
HTTP translation already keeps a cloned reqwest connection pool; its default
TCP_NODELAY policy and Qwen's bounded completed-SSE tail drain are preserved.
[Tokio documents](https://docs.rs/tokio/latest/tokio/net/struct.TcpStream.html#method.set_nodelay)
that this option disables Nagle buffering; [RFC 9293 section 3.7.4](https://www.rfc-editor.org/rfc/rfc9293.html#section-3.7.4)
describes its interaction with delayed acknowledgements. This makes an explicit
low-latency policy suitable for small streaming and health-check frames.

The user reported roughly 200 ms on the desktop network badge. Read-only routing
inspection found DIRECT connections, and bounded ICMP checks of their targets
measured 43–45 ms. These use different timing boundaries: the badge also includes
local send wait and service/application Pong processing. This evidence does not
establish Nagle as the cause or predict an exact improvement. Four subsequently
authorized, zero-audio Audio3 sessions measured about 42 ms in the old build and
44–46 ms in the NODELAY candidate; they do not establish a speed gain.

The Audio3 receive loop directly awaits its idle heartbeat operation. That
operation originally waits for the sink lock before checking pending real PCM.
A controlled busy-writer fixture demonstrates this avoidable suspension even
though the operation must send nothing. A synchronous precheck removes that
wait; existing post-lock checks and heartbeat/lifecycle ownership remain intact.

Reducing capture blocks, semantic stabilization waits or shared MT budgets needs
separate evidence. Keep queue bounds, final priority, cancellation, replay guards,
service prompts, audio formats and provider selection unchanged in this change.

## Implementation and verification

1. Extend actual direct, HTTP CONNECT and SOCKS WebSocket fixtures to verify
   the physical socket's low-latency policy as well as real payload exchange.
   Confirm the old transport fails that contract before changing it.
2. Extend the existing real-PCM WebSocket fixtures with a held-writer contract:
   with pending PCM, the idle-silence future must complete on its first poll;
   subsequent real frames and post-drain silence must retain their exact order.
   Implement the synchronous pending precheck and retain the post-lock recheck.
3. Centralize TCP connection/configuration and enable NODELAY before the tunnel
   or protocol layer. Preserve the outer timeout and failure-without-fallback.
4. Run focused transport/provider checks, the shared architecture guard and
   canonical `./scripts/check.sh`, including actual host JNI. Run Android Debug
   and Release tests/native checks through the unchanged production adapter.
5. Independently review routing, TLS, framing and lifecycle regressions. Record
   exact source/test boundaries in the integration ledger; inspect the diff for
   private data before delivery.
6. If newly authorized, compare the same saved development Alibaba profile with
   and without NODELAY using bounded authenticated WebSocket Ping/Pong sessions.
   Do not start capture, send PCM, change stored proxy/profile choices or interrupt
   a user-owned active session. Keep credentials and raw service responses private.
   A network-only comparison does not establish capture-to-caption latency.

## Acceptance boundary

The transport contract must hold for direct and configured-proxy routes on the
shared source used by both platforms. Packet boundaries and security behavior
must pass regression tests. More small TCP packets are an intentional tradeoff.
Local checks establish policy and correctness, not a provider speed claim. The
authorized idle-Ping comparison showed no stable improvement. A subsequent native
Mac candidate run verified meaningful speech, bilingual overlay commits and
43–44 ms badge observations. Its near-silent old run is not a valid speech
baseline, so a speed gain and the reported 200 ms cause remain unverified.
