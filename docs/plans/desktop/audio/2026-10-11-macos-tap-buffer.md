# Core Audio callback scheduling tolerance

The signed development baseline at `fd8e6dbd` repeatedly reported
`capture.backpressure` while sending silent PCM, without the portable send
pipeline's queue-full event. Metadata-only trace events 450, 541 and 677
identify the native callback boundary. This establishes a capture interruption,
not the cause of every reported Gemini subtitle stall. Audible playback of the
reported video must be tested separately.

The callback queue previously held four packets. At the observed 48 kHz,
1024-frame cadence, only about 85 ms of worker scheduling delay could stop
capture and restart the provider session. A six-packet, 128 ms burst regression
fails on that baseline.

Keep the callback nonblocking and preserve packet order. Allow up to 256
packets, with a separate atomic byte reservation limiting queued and currently
processed native PCM to one second at the negotiated mono format. Supported
formats cap this at 768,000 bytes. Packet destruction releases reservations on
normal processing, disconnected/full queues and session cancellation. The slot
bound also limits allocations for unusually small callbacks. Keep genuine
overload fail-closed and recover through the existing session controller.
Use a compare-exchange reservation loop compatible with Rust 1.88 and current
stable; do not depend on the deprecated `fetch_update` API or its newer rename.

Do not drop arbitrary audio, change provider protocols, mix inputs, or expand
capture targets. Label callback byte/slot exhaustion in content-free diagnostics
so it can be distinguished from provider-send congestion. Format validation and
native teardown remain unchanged. This platform adapter fix applies to every
provider using Core Audio taps; it does not change Android or other capture
backends.

Verify short burst order/ownership, byte and slot limits, processing/drop
accounting, stale generations and one-shot failures. Run canonical checks and
the same audible video from 0:00 on the signed development app, recording actual
native observations and remaining limits in the integration ledger.
