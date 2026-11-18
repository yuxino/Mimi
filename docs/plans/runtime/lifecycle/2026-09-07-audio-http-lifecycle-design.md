# Audio lifecycle and HTTP response bounds

## Status

Accepted as a focused reliability and size change. Provider prompts, final
ordering, audio capture permissions, and credential storage remain unchanged.

## Decisions

- Graceful audio shutdown explicitly closes the receiver before draining.
  Retained native callback ingress handles must not turn a completed drain
  into a timeout. A transport failure reports failure to the caller.
- The worker retains abort-on-drop ownership while a finish future awaits it.
  Dropping the pipeline, cancelling finish, or calling stop concurrently with
  finish releases the worker and its queued audio/provider references.
- Limit each Qwen HTTP response to 1 MiB, including chunked/error bodies, and
  streaming translation previews to 64 KiB. Oversized successful responses fail
  without retry; oversized HTTP errors retain their status so authentication
  and retry decisions remain correct. No provider content enters diagnostics.
- Scan only new SSE bytes and drain the processed prefix once per network
  chunk. Retain split UTF-8 sequences until a full line arrives and preserve
  missing-final-newline and `[DONE]` behavior.
- Consolidate the direct HTTP client with the updater's reqwest 0.13 version.
  Use `rustls-no-provider` with the existing updater/Tungstenite ring provider,
  explicitly initializing it before translation can create its first HTTP
  client. A direct rustls reference reuses the existing crate and does not add
  another crypto implementation. Preserve any already installed provider.
  Keep system certificate validation, streaming, and system proxies. The Qwen
  client uses raw JSON bodies, not multipart/form APIs. Request deadlines stay
  unchanged; no new credential or external-service requirement is introduced.

## Verification

Exercise retained ingress, send failure, cancellation, drop, and concurrent stop.
Check SSE segmentation, UTF-8, early completion, size boundaries, chunked HTTP
bodies, authentication status, request encoding, and request timeout using local
test servers. Verify public HTTPS without credentials separately from live
provider sessions. Compare equivalent macOS arm64 release executables before
and after HTTP dependency consolidation; do not infer Windows or installer size
from that measurement. Run the canonical check and native build, leaving formal
installed apps untouched. See the overlay behavior record for the independently
reported source-preview fix.
