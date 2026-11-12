# Stable audio-status rendering

The floating capture-status row and the Windows output picker receive fresh
native JSON every second. Fifty unchanged responses currently produce fifty
React commits in each production component, even when all visible metadata is
unchanged.

Keep the existing one-second serial polling and accept only changed metadata
into React state. Compare all own fields, observation flags, optional-field
presence, per-source identity/order and device identity/name/order. A new field
must not silently become an ignored part of the snapshot. No timing, capture,
provider, permission or document-visibility policy changes are required.

The capture row's comparison belongs to its current input/target/lifecycle
stamp. Replacing that stamp starts with no accepted observation; an old request
cannot restore the retired stamp. Read failure also clears the comparison so an
identical successful response restores current status. Windows read failures
remain independent presentation state and clear on success even when the device
metadata is unchanged. Language and session-state changes continue through the
existing React subscriptions; cache only native metadata, never localized copy.
Publish error-flag transitions only; an already-false setter can still produce
an extra React commit after another state update.

Verify the same production-component Profiler workload before and after, with
equal native-call counts and visible-text hashes. Behavioral checks cover
changed observations and devices, dual-source snapshots, stale reads, failure
and recovery, lifecycle/locale refresh, and the existing Windows hide/resume
rules. Synthetic commit counts establish avoided rendering work, not native
CPU, heap or frame-rate improvements.
