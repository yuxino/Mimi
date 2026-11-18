# Qwen streaming connection reuse and latency boundary

## Decision

Keep `qwen-mt-lite`, complete subtitle pairs, existing pacing, and bounded final
ordering. Do not increase concurrency or truncate output to make a latency
number look smaller. The system-translation exploration is postponed; no native
translation route, bridge, build integration, or IPC is installed in Mimi.

## Evidence

The current client already owns and reuses a `reqwest::Client`. Its effective
checked build features use HTTP/1, without `http2`. The complete upstream MT
request measurements reported during this iteration are about 351–400 ms; the
separate health round trip is about 46 ms. They measure different operations,
so subtracting them does not establish model inference time.

The initial localhost SSE fixture separated `[DONE]` from the HTTP chunked-body
terminator by a 5 ms timer. Before the bounded tail drain, two sequential
translations opened two TCP connections; afterwards they reused one locally.
Windows ARM64 CI later opened two connections because the server timer and tiny
TCP write were not guaranteed to arrive within the product's 20 ms cleanup
bound. That result does not justify extending the cleanup bound.

The fixture now releases the separate HTTP terminator from a one-shot signal
when the client consumes the complete SSE chunk. It uses a single-thread test
runtime and `TCP_NODELAY`, removing the server timer and delayed-ACK dependence.
The normal wrapper reuses one TCP connection. A permanent negative control runs
the same actual HTTP/decoder path but drops the response immediately at `[DONE]`;
it opens two connections. This proves a client-side reuse defect for that
response shape, without changing the production drain. It does not prove how
often the actual provider separates those frames, or how much of its measured
latency this defect contributes.

Hyper's HTTP/1 implementation only attempts an immediate drain when the body
receiver is dropped; if the body is not already drainable, it closes that read
side. See the [upstream implementation](https://github.com/hyperium/hyper/blob/v1.11.1/src/proto/h1/conn.rs).
Reqwest recommends reusing its client and connection pool in its
[client documentation](https://docs.rs/reqwest/0.13.4/reqwest/struct.Client.html).

## Implementation bounds

- `[DONE]` fixes the complete nonempty translation result.
- After that result has completed within the original request deadline, consume a small
  HTTP tail inline: at most 20 ms, 8 KiB, and 32 chunks. No background task or
  additional provider request is created.
- A stalled, malformed, or excessive tail cannot turn the completed translation
  into an error. The full request deadline still governs headers and text
  generation; cleanup adds at most 20 ms after completion.
- Dropping the request future drops its owned response, including during the
  drain. No draining operation survives cancellation.
- Successful ordinary EOF releases the connection to the existing pool. A tail
  exceeding a bound is dropped and may close that connection.
- Neither the wire request, translation content, prompt, model, proxy route,
  retry policy, preview budget, nor final ordering changes.

## Official model limits and remaining options

The [official model comparison](https://help.aliyun.com/en/model-studio/machine-translation)
already recommends Lite for the fastest simple real-time translation. Its
[model capability page](https://help.aliyun.com/zh/model-studio/qwen-mt-lite)
explicitly marks context caching unsupported, with Beijing limits of 60 RPM and
100,000 TPM. Do not add an unsupported cache parameter or advertise a cache
speedup. The [MT API reference](https://help.aliyun.com/en/model-studio/qwen-mt-api)
documents incremental Lite output and warns that a lower `max_tokens` truncates
the result. Early prefixes must not replace the stable complete pair merely to
reduce perceived latency.

HTTP/2 remains a separate experiment requiring real endpoint negotiation,
proxy compatibility, and dependency review; this change does not enable it.
Before changing the model or region, measure the same licensed playback fixture
with the same language pair and selected direct/system/custom route. Keep
complete-request p50/p95, rate-limit categories, final count/order, and visible
subtitle stability as distinct acceptance criteria. No supplier-speed claim is
made from the localhost fixture.

## Focused verification

`clients::translation::qwen_mt_client::tests` covers ordinary SSE and UTF-8 boundaries,
bounded bodies, authentication status, request cancellation, plus:

- Two complete SSE requests reuse one TCP connection when EOF arrives later.
- Dropping the completed response without draining opens two connections in
  the same signalled-tail fixture, preserving the regression's negative control.
- A missing HTTP terminator keeps the completed result and closes the peer.
- The drain obeys byte and chunk limits, including empty chunks.
- Cancelling during cleanup closes the response without a surviving task.

Both the initial module run and the shared backend rerun after the final
deadline-scope refinement passed 15 tests, with the existing public-network
smoke test ignored. The signalled-tail fixture and its negative control passed
in the subsequent module run: 16 passed, one intentional public-network smoke
ignored. Windows ARM64 confirmation requires a new CI run. Installed-app
latency and supplier behavior require the subsequent signed-dev session and
are not established by these tests.
