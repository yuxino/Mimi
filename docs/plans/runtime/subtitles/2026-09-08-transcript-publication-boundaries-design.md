# Transcript publication boundaries

## Scope and evidence

This is a contained reliability correction to the existing transcript and
preview lanes. Provider protocols, prompts, translation modes, presentation,
and credential handling remain unchanged.

The OpenAI/Azure transcript committer consumes a prefix across multiple timed
deltas. Its eager `then_some` expression subtracts the consumed count even
from boundaries that will be discarded. When an earlier boundary lies inside
the consumed prefix, checked arithmetic in debug/test builds panics. Existing
timing tests supplied only one delta per stream and did not cover this case.

The Qwen preview lane checks its epoch before publishing, while the shared
event transport allocates sequence numbers under a separate dispatch lock.
A final can invalidate the preview and publish between that check and send.
The old preview then obtains a newer sequence number and escapes the
receiver's suppression of drafts older than a final. Aborting a Tokio task
does not interrupt a synchronous partial-result callback already in progress.

## Decisions

- Discard consumed timing boundaries before subtracting the consumed character
  count. Preserve timing and character offsets for the Unicode suffix so the
  next aligned pair commits once, without losing or repeating text.
- Check preview validity inside the existing synchronous event dispatch
  critical section, before sequence allocation and lane publication. A final
  either follows an already accepted preview or makes the subsequent preview
  check fail. It cannot publish between a successful check and that preview.
- Apply this gate to the preview's source draft, streaming translation
  callbacks, and completed translation. Keep the early epoch check that avoids
  starting an already obsolete request.
- The predicate must be short and synchronous and must not re-enter the event
  sender. No mutex guard crosses an await. Reliable queue bounds, overflow
  recovery, final priority, and latest-only draft storage remain intact.

## Verification

Regression tests cover multiple timed deltas in either stream and a retained
Unicode tail that becomes the next final. A controlled two-thread publication
test forces the old check/send gap and verifies that a final cannot be followed
by the obsolete preview. Further tests check both draft lanes, cancellation,
and current-generation callbacks without contacting a provider.

Run the canonical check and existing frontend source/translation regressions.
These tests establish deterministic ordering and correctness; they do not
measure live-provider latency or prove native UI, installation, or Windows
runtime behavior. This audit does not access saved credentials, capture audio,
or replace an installed application.
