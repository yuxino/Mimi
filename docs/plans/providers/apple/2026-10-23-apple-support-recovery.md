# Apple capability recovery across settings windows

This pass addresses confirmed recovery defects adjacent to installed-only
translation initialization. It does not establish the cause of the user's
reported update behavior or authorize model downloads and native foreground tests.

## Capability cache and publication

A failed Apple Speech or Translation inventory query currently becomes a cached
empty success. Later settings snapshots therefore skip loading it again. Keep
the published state empty on failure, but distinguish a failed query from a
successful unsupported/empty result. The next settings read may retry a failure.
Speech resource revisions continue to describe changes in published content;
equivalent failures cannot advance that revision and trigger a refresh loop.
A partial preparation confirmation must not certify a complete inventory scan.

Coalesce concurrent bootstrap reads, including failures from the same attempt,
under the existing refresh ownership boundary. A later caller may retry a
completed failure. Serialize explicit Translation refreshes so an older error
cannot overwrite a newer success. No polling, persistent ready marker or
dependency is needed.

When a settings read changes effective capabilities, publish the exact returned
snapshot to existing windows. An equivalent read does not need a broadcast.
Concurrent readers may emit equivalent recovery snapshots; consumers already
compare resource signatures, so these finite duplicates must not start a loop.

Before loading native inventories, derive only the language capabilities from
the fallible preferences/catalog read. Keep its existing migration and failure
handling. After the wait, read the current settings again and project credential
states only for the returned snapshot. Do not reuse an early full snapshot:
a profile, language or credential may change during the inventory query.

## Visible query ownership

Apple Speech must treat reopening a hidden page as a fresh visit, even if its
language signature is unchanged. Old readiness cannot remain actionable while
that visit's query is pending. A signature refresh within the same visible visit
must instead keep its current controls mounted and disabled, so an in-flight
recognition check can finish when another locale's inventory changes.
Apple Translation support and pair-status retries
must become busy immediately and share an in-flight request for the same context.
Hide, profile/pair changes and explicit preparation invalidate older results.
Hidden views do not initiate retries. Keep current localized progress/error copy.

## Verification

First reproduce the failed-cache, cross-window publication and stale/busy UI
boundaries with controlled results. Cover failures followed by recovery,
unsupported success, partial preparation, concurrent bootstrap reads, explicit
refresh ordering, duplicate Retry, old promises after navigation, and preservation
of a current recognition check during an unrelated locale refresh. Use the
real helpers and components with injected native results; no real model is needed.

Run the canonical suite and actual Android JVM JNI coverage because shared Rust
changes are involved. Review affected Apple settings through hidden browser
fixtures at minimum/default widths and representative locales. Keep screenshots
outside Git. These checks do not replace signed same-identity restart/update,
native preparation or real translation acceptance, which remain deferred while
the user watches video.
