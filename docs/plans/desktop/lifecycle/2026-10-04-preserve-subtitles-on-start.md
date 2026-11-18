# Preserve subtitles when starting desktop sessions

## User-visible behavior

Starting from the global shortcut, tray menu, or panel after a stop or failure
keeps the bounded confirmed subtitles already shown in the panel. A start must
not be an implicit Clear. Reopening or showing a window keeps using the same
session snapshot. The explicit Clear action still removes displayed subtitles
and opted-in current-session text through the existing content boundary.

## Lifecycle and storage boundaries

Remove the clear flag from the desktop start and connection APIs so every entry
point has the same behavior. Use an explicit new-session boundary in the shared
reducer and reuse its transient reset:
retire unfinished previews, pending indicators and old provider identities while
preserving confirmed history. Changing the selected input also preserves the
confirmed rows and their original source labels. Reset every source's session
boundary, including inputs that may be enabled later in the new session. Keep the existing 20-row
aggregate display bound and per-source bounds.

A manual start remains a new capture/provider session. Finalize the previous
opted-in files, begin fresh archive metadata and recording buffers, and never
copy old display rows into the new session's saved transcript. Retention and
audio recording remain opt-in; this change does not restore an unsaved display
across a full application exit. A new session must accept its first ID-less final
even if it repeats the last previously displayed pair. Connection retries inside that session keep the usual
duplicate protection; blank results do not consume the new-session boundary.
Provider protocols and transport-generation gates are unchanged.

## Verification

Cover stop/start and retry, changed audio inputs, discarded transient state,
reused provider IDs and identical ID-less finals, bounded history across repeated
starts, separate archive contents, and explicit Clear. Run the canonical checks. Native UI verification
uses the signed dev bundle and synthetic UI-only input; keep this distinct from
real capture/provider evidence. Protect any running formal app's session while
preparing the development build.
