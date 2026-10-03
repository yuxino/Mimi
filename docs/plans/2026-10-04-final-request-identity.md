# Keep repeated finish translations independent

## Demonstrated boundary

The desktop HQ adapter admits confirmed server finals and a bounded session-finish
fallback into one serial final lane. On clean `c7e4067`, the lane key compares text
and local confirmation revision, but treats either finish boundary as an alias
regardless of source identity. If sentence A is active or waiting and a distinct
sentence B has identical text when Stop flushes its draft, B can disappear before
its translation HTTP request. This is final admission loss, not shared reducer,
provider semantics or frontend clipping.

A focused regression calls the real `flush_pending_draft` path with A active or
waiting, then checks the queued identities. The old implementation fails: B is
absent. It uses a pending worker instead of network requests to hold that boundary
stable. Cases cover known distinct source IDs and the supported ASR path without
sentence IDs.

## Decision

Include source utterance ID and content revision in `FinalRequestKey`. Equal text
can alias only when both fields match and either the local confirmation revision
matches, or a known source ID connects a server final with a finish fallback.
Missing IDs do not establish causal identity: two `None` IDs with distinct
confirmations stay independent. Known/unknown IDs and distinct content revisions
also stay independent. Preserve idempotence for a duplicate confirmation and for
server-final/finish work on the same identified source revision.

Keep translation prompts, models, source language handling, bounded queue size,
total deadline, retry policy, preview priority and stop grace unchanged. The shared
subtitle core already preserves distinct confirmed identities. Android's final-only
adapter has no corresponding desktop draft-finish text alias; do not add desktop
preview machinery there. Review its existing shared contracts and native build
boundary separately.

## Fixed comparisons and limits

The first quality cycle freezes 12 public-media inputs and official references in
a private catalog: 10 speech conditions and two negative controls. It uses the
existing Audio3 baseline runner, `auto`, bypass input processing and at most three
workers. These direct ASR results locate upstream differences; they do not exercise
HQ finish admission or establish a translation-quality improvement.

One clean signed dev baseline captures three occurrences of a fixed Japanese
sentence with 12-second gaps. All three server finals, translation requests,
accepted pairs and history identities survive. Five reference semantic units per
occurrence are preserved in an AI review; no human bilingual gold or independent
listening is claimed. Stop follows completed finals, so this run does not reproduce
the pending finish race. Intended pause/resume controls were not reached; all three
occurrences count as captured. The regression above is the deterministic
before/after proof for this repair.

Run focused HQ tests, `./scripts/check.sh`, and the canonical signed dev launcher.
Record native inspection, CI and remaining provider/device limits in the
[run ledger](../development/integration-runs.md). Preserve original inputs, failed
regressions and provider evidence privately. No release is part of this cycle.
