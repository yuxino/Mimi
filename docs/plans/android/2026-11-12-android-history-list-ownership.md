# Android saved-history list ownership

Android sampled active journal IDs before reading saved history. A journal
started during the read could appear in the saved list; a journal finished in
that interval could remain hidden until the next refresh. Match desktop by
filtering against the active owners after the file read completes.

A private helper accepts the real file reader as a closure, reads the items,
then obtains active IDs and filters the result. The registry mutex is not held
while this list scans files. This establishes completion-time owner filtering,
not an atomic snapshot of every file and lifecycle mutation. Only `history_list` changes; page,
export and delete keep their pre-read active-session rejection. Directory
scoping, pending owners and recoverable failed finalization keep their rules.

Read and registry failures retain the sanitized `session_history_unavailable`
error. An unfiltered list is never returned on these failures. The existing
handling of an individually poisoned journal mutex is unchanged.

Deterministic regressions use the actual registry, begin/finish operations,
leases and filesystem. The reader closure starts a journal or finishes its
last owner before reading. Both cases fail when owner sampling moves back
before the closure, and pass with completion-time filtering. A real invalid
root also checks error sanitization. Test-only serialization respects the
global journal limit; fixtures remove only their own handles and files.
