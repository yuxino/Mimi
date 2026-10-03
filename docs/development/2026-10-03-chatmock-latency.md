# Local ChatMock latency investigation

Measured on 2026-10-03 against the already authenticated localhost server, with fixed synthetic English-to-Chinese inputs and Mimi's unchanged translation prompt. No credentials, audio, subtitle content or response text were logged. These measurements are diagnostic samples, not a live-session end-to-end benchmark.

| Probe | Plain request | Explicit low effort, summary none |
| --- | --- | --- |
| Short sentence, sample 1 | 2.656 s | 1.881 s |
| Short sentence, sample 2 | 3.517 s | 2.613 s |
| Paragraph, complete response | 2.481 s | 2.407 s |
| Paragraph, streaming first content | 2.162 s | 1.964 s |
| Paragraph, streaming complete | 3.035 s | 2.996 s |

All eight translation probes returned HTTP 200. Local model discovery returned in 25 ms. The selected server model was held constant. The low-effort probes also disabled reasoning summaries, and samples vary with upstream/network state; they do not isolate reasoning effort or prove a reliable speedup.

The local ChatMock code defaults to medium reasoning. It calls the authenticated upstream Responses service with streaming enabled, but aggregates all output before returning a non-streaming Chat Completions response. Mimi currently requests non-streaming Chat Completions and publishes a complete translation. In the measured streaming samples the first content already took about two seconds, so enabling streaming alone would not eliminate upstream first-content latency.

Mimi's audio-to-subtitle delay additionally includes recognition, preview pacing, complete translation and ordered final work. Each compatible HTTP attempt has an eight-second limit; retryable errors can receive up to three attempts within a bounded final age. Final translations are serial with a bounded queue. No timeout/retry/backlog was demonstrated by these probes. Request timing in the UI excludes recognition and pre-existing queue waiting.

The native session cannot be attributed to ChatMock solely from saved configuration or earlier server activity. A frozen session may differ from subsequently saved preferences. Investigating an audio-session slowdown requires confirming its actual translation route and content-free ASR/MT/queue timings. No profile, model, routing or server defaults were changed during the investigation.

ChatMock makes a separate upstream HTTP request for each translation and uses a 600-second upstream timeout. A cancelled Mimi request does not establish that its upstream non-streaming work has stopped. Connection reuse and upstream cancellation are potential follow-up investigations, not measured causes here. Avoid verbose ChatMock request/response logging for real audio.
