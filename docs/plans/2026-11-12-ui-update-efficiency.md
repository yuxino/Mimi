# Idle archive rendering and responsive transcript search

## Evidence and scope

On main `4857196f`, each successful archive-status poll installs a new object
and renders the whole library or reader even when the metadata is unchanged.
Fifty unchanged polls cause fifty React commits, 1,050 timestamp formats for a
20-session library, or 1,500 formats for a 30-entry reader.

The transcript reader applies its 180 ms search delay to every nonempty-query
request, including pagination and retries. While a new search is still pending,
it also displays the no-matches message before it knows the result. Focused
regressions reproduce both failures for current and saved sessions.

## Approach

Keep the existing serialized one-second metadata poll and its visibility,
mutation-revision and teardown guards. Skip React setters when both metadata
and its presentation context are unchanged. Compare all metadata keys and
audio-source order; any size, limit, sample-rate or storage-error change still
updates immediately. Track read failures separately so an identical successful
response clears an error and restores actions.

The poll also refreshes dates and localized copy. Capture the presentation
context after each committed render, and compare language, local calendar day
and the system IANA time zone at the next poll. Probe the time zone only when
the current view contains dates. A changed context forces a render even if
archive metadata is identical. Do not cache a default-time-zone formatter
across OS zone changes, or infer zones from the current UTC offset alone.

Apply search debouncing only when the input differs from the last query actually
dispatched. Page, retry and refresh requests for that query keep the existing
zero-delay dispatch. Clearing the query is immediate. Keep request-generation,
page-visit and archive-owner invalidation; report loading until a query settles.

This preserves the existing polling frequency and live-update behavior without
introducing a request cache, background timer or new dependency. Audio capture,
provider scheduling, subtitle stabilization and shared Rust rules are unchanged.

## Verification

- Count React commits and formatter work in the real library/reader components
  with unchanged IPC metadata; retain node identity and current content.
- Cover metadata changes, read failure/recovery, hidden pages, all UI languages,
  local midnight and system time-zone changes, including zones with equal
  present offsets but different historical offsets.
- Check rapid typing, immediate next/previous page and retry, clearing search,
  pending/empty/error states, superseded replies and hidden/unmounted readers.
- Inspect the signed macOS UI-only development preview, then run frontend
  checks and production build. Counts, synthetic timing and request dispatch
  timing do not establish a production CPU percentage or provider latency.
