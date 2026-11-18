# Confirmed source and the next live request

Starting another translation request must not create a live copy of the
preceding confirmed original. The HQ route can temporarily hold that final
source while its next request sets `isTranslationPending`; pending and timeout
are activity signals, not evidence of a new recognition cycle.

The atomic-preview presentation therefore omits an unstamped final source
which exactly equals the newest confirmed source. This applies to Original's
raw-source lane and Bilingual's source fallback when no complete preview pair
exists. Its original and translation remain in the existing confirmed row.

This is not general text deduplication. A new nonfinal source may repeat the
same lyric and remains visible. A different final, an explicitly identified
source, and a new complete preview pair remain visible. Non-atomic protocols
retain their existing repeated-final behaviour. The backend's real server
utterance boundary separately determines whether a draft is new or replayed;
the frontend does not guess that boundary from matching words.

No history content, timestamps, layout keys, scroll anchors, saved sessions,
or provider prompts change. Regression fixtures use synthetic text and cover
pending/timeout transitions, repeated drafts, different finals, identified
streams and non-atomic repeated finals. Component fixtures also assert that
the confirmed DOM row survives a pending transition without a phantom live
row, then displays a legitimate new same-text draft in Original and Bilingual.
Native song/slow-speech acceptance remains separate from these fixtures.
