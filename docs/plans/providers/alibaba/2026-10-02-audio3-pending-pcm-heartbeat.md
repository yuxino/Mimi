# Audio3 idle silence and pending PCM

The local WebSocket regression reproduced an extra 100 ms silent packet
between 50 ms of sent PCM and 100 ms of already-captured PCM waiting in the
send worker. The existing idle helper only observes the time of the last
completed socket send. A scheduling or recording delay before that send can
therefore look like absent capture. This does not establish that the Queen
reproduction actually encountered this scheduling window.

## Accepted change

Each audio pipeline owns a cloneable, atomic `PendingPcmGate`. Nonempty PCM
acquires a non-cloneable RAII guard before bounded queue publication. The
buffer and guard travel together; the worker keeps the guard across the entire
send future, including recording and waiting before the socket. Completion,
transport failure, rejected ingress, dropped queued buffers and cancelled
workers release their guards. Gate generations are independent; there is no
reset that could invalidate guards still owned by an older worker.

The session binds its gate through the provider facade and HQ client to Audio3
after installing the pipeline and before starting native capture. Other
providers retain their current send behavior. The Audio3 idle helper checks
for pending real PCM while holding the socket sink lock, after its existing
idle/terminal checks. Existing pending PCM suppresses a synthetic silent
packet. A drained pipeline or a client without a capture queue retains the
existing idle heartbeat. The PCM data, 100 ms interval, protocol parameters,
prompts, queue capacity and normal send order do not change.

## Scope and verification

This fixes the demonstrated queue/in-flight window. It cannot identify a
native callback that has not arrived yet; an empty queue alone does not prove
that physical capture was silent. A callback arriving after the idle decision
is also outside this gate's guarantee. Raising a wall-clock threshold would
not resolve that distinction. A broader sample-clock or explicit native-idle
design needs separate evidence and validation.

The shared controlled local-WebSocket fixture retains the unbound baseline
(real 50 ms, synthetic 100 ms, real 100 ms) and covers the bound gate
(real 150 ms total, zero extra silent frames while pending). A separate case
checks that an idle packet can still be sent after the real queue drains.
Pipeline regressions check rejected/full ingress, queued and in-flight
cancellation, transport failure, graceful completion and empty buffers. Pure
core tests check shared clones and independent generations. No fixture uses
provider credentials or native capture.

The upstream `heartbeat=true` request option supports keeping the connection
alive while continuously sending silent audio. The client event documentation
does not prescribe a 100 ms completed-send gap as a reason to invent PCM; the
server event's heartbeat result is separately ignorable.

References:
- [Audio3 client events](https://help.aliyun.com/zh/model-studio/qwen-audio-asr-streaming-client-events)
- [Audio3 server events](https://help.aliyun.com/zh/model-studio/qwen-audio-asr-streaming-server-events)
