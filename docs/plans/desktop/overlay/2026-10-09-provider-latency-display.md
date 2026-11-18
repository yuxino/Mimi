# Show measured service timing only

The overlay currently treats every active provider as though it will eventually
produce API and translation samples. Most realtime providers do not expose a
separate translation boundary, so their translation field stays Pending forever.

Render API timing only after a valid current sample. Render translation timing
only with a valid sample and its known request/follow kind; infer neither from
the profile name nor its configured text route. Omit missing/invalid metrics and
the separator when only one is visible. Paused, reconnecting, failed and stopped
sessions must not retain current-looking timing values. Recognition-only sessions
have no translation metric or translation work status.

Show translating, rate limiting and retry recovery as independent short states,
without calling them translation duration. Apply the same measured-only rule to
the settings diagnostic summary; keep bounded raw facts in the collapsed report.
No new IPC capability matrix, polling, provider request or dependency is needed.

The API sample comes from the existing speech WebSocket health probe, including
local send wait. HighQuality probes recognition, not the independent text service.
Apple Speech health is a local connected-state check and already produces no API
sample. HighQuality records translation request time; LowLatency records the wait
between matching source and translation finals. Other realtime clients currently
return no translation sample. A true zero remains a valid measurement. Apple
same-language passthrough must not be recorded as a translation request.

Verification covers API-only, translation-only, both, neither, invalid numbers,
unknown kinds, first-sample arrival, recognition-only, pending/recovery, lifecycle
changes and all seven UI languages. Check sibling diagnostics, the full repository
suite, and the signed native development app. No performance improvement or
cross-provider end-to-end latency comparison is claimed.

Delivery: create a separate PR based on PR #235 and merge into its branch only.
PR #235 remains Draft; no merge to main, tag, release or issue closure.
