# Subtitle clear boundary and native controls

## Accepted scope

Clear must remove the visible captions and opted-in current-session text without
allowing an older queued event, HTTP result, or callback to restore them. It must
not restart capture, reconnect the provider, refund rate-limit budget, reset an
active cooldown, or change recording preferences. Existing audio recording
continues. This follows the user's request to finish the outstanding subtitle
and native interaction defects.

## Content ownership

Provider event envelopes carry a content revision independently of the session
connection generation. Clear advances that revision, cancels replaceable preview
and final-translation work, and clears local assemblers. Async callbacks retain
the revision they started with; dispatch and the manager both reject stale
content. Lifecycle errors and connection acknowledgements remain deliverable.

The manager serializes clear with subtitle application **and** the subsequent
private-history append. A popped event must be checked after acquiring this
gate. The gate is released before lifecycle teardown waits, avoiding a lock
inversion with the command's lifecycle guard. A failed local-history clear leaves
the content boundary unchanged and returns an actionable control error.

Clear also resets pending preview/final flags and cancels the old pending timer.
Quota/backoff state belongs to the connection/request budget, not display
content, and therefore survives clear.

## Real speech boundaries

Audio3 exposes cumulative sentence IDs and optional word timestamps in its
[server events](https://help.aliyun.com/zh/model-studio/qwen-audio-asr-streaming-server-events).
The current pipeline does not map those timestamps to the UI click or audio
capture clock, so they do not establish the exact clear instant. If clear happens
during a known sentence, suppress its
remaining drafts/final and resume at the next real sentence. Do not subtract
text prefixes or guess which rewritten words were spoken after the click.
Other protocols use existing item/turn boundaries where available. Services
without a source-turn identity can guarantee local-buffer and callback clearing,
but cannot claim a precise cloud-audio cut at the click. Audio whose first
recognition event arrives after clear cannot be retroactively time-classified.

## Native interactions

The nonactivating macOS subtitle panel must accept keyboard focus when the user
interacts with it, while opening/updating captions must not steal focus. Home,
End, PageUp, and PageDown must reach the reader. Secondary icon actions need
bounded pending feedback and an error message on failure; a failed native
collapse must restore the matching optimistic frontend state.

## Verification

- Delay an MT response across clear; reject its partial/final and accept a later
  real sentence. Exercise queued events already popped before the clear gate.
- Preserve request pacing, consumed budget and 429 cooldown across clear.
- Check repeated clear, stopped/paused states, and lifecycle error delivery.
- Check native long-subtitle read/End behavior and global commands using the
  signed app at its single canonical dev path.
- Run canonical checks and the available Windows/Linux native CI smoke routes.
  Keep virtual/physical device and provider-session evidence distinct.
