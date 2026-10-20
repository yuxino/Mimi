# Conditional JNI state polling

Android polls the native Session every 60 ms. Its worker currently receives and
parses a complete subtitle snapshot before comparing the version it already
delivered. During stable state, the Rust JSON tree, serialized response and JVM
JSON tree are repeated work. The optimization belongs to the JNI adapter;
subtitle state, provider queues and recovery rules stay shared and unchanged.

## Backwards-compatible envelope

The existing `poll` operation accepts an optional nonnegative integer
`sinceVersion`. Missing or unequal versions return the original full response.
An equal version returns only `unchanged: true`, `version`, `finished` and
`errorCode`. These terminal fields are read for every request, rather than
cached with the subtitle revision. Invalid optional values retain a sanitized
invalid-request error; a removed handle still fails normal Session resolution.

Old Kotlin omits the new field and keeps receiving full snapshots. Current old
native code ignores the extra request field and returns a full snapshot, so new
Kotlin can use its full-response path without exception-based retry. The first
poll of each worker omits `sinceVersion`; only a successfully delivered full
snapshot advances that worker's known revision.

Kotlin consumes terminal metadata on both response shapes. An unchanged reply
must match the revision of an already delivered full response. It never invokes
the snapshot listener or advances that revision. Full responses are delivered
before handling finish/error; a terminal full response is delivered even when
its version matches, protecting the old native publication race as well.

## Controller and revision consistency

`publish` and the periodic controller tick keep their controller guard through
the matching version increment. Poll holds that same guard while comparing the
version and capturing an owned view with current terminal metadata. It reads
`finished` before the comparison version and uses those captured values. Full
responses clone only their exposed view, release the guard, then serialize the
typed response directly. Light responses return before cloning subtitles. Neither
path constructs an intermediate JSON tree or serializes under the controller
guard. There is no full-response cache or production synchronization hook.

The previous ordering released the controller guard before incrementing its
version. A gated temporary extraction of the production methods reproduced a
full response containing new final subtitles, the old version and `finished`
true. The old Kotlin version check then skipped the listener before returning.
Keeping the revision increment under the controller guard prevents this pairing.
The temporary gates are supplementary evidence, not product APIs or maintained
test substitutes.

## Verification boundaries

Implementation base: `8d60c7613a076c971a17b9f2ee7578ade4cef712`, including the
preceding PCM backpressure recovery. Five formal Rust regressions first failed
against the original implementation: four API cases and one controller/version
contention case. The final six new cases cover legacy/first, same/changed/future
versions, late terminal fields, invalid versions, stale handles, coherent
publication and timer-driven deadlines. All 24 JNI Rust tests passed, including
existing PCM recovery, cancellation and reliable-tail cases. Strict all-target
Clippy, Rust format, shared architecture and diff checks passed.

Six pure Kotlin cursor/request tests passed using the project's Kotlin 2.0.21
and JUnit versions. This lightweight compilation used the actual modified files
with an unused Network adapter stub that fails if called; it is not a Gradle,
network-adapter or complete JNI-suite acceptance result.

An actual Java caller of JNI exports failed the new light-response contract on
the fixed old native library and passed it on the new debug library. It checked
first/legacy/mismatch full responses, equal-version light responses, invalid
versions and removed handles. Local TLS provider cases preserved final subtitles
and closed transport on graceful finish and fatal error; same-version light
responses retained current `finished` and `errorCode`. Both terminal cases also
passed against the old native library's full-response fallback. Kotlin consumption
of those full shapes is covered by the pure tests, not claimed as an end-to-end
execution of the complete Kotlin engine. Maintained Kotlin/JNI cases remain CI
gates. No real provider, production credential or audio capture was used;
protocol endpoints were intercepted on loopback.

## Production allocation comparison

The final production `snapshot_json` and response structs were extracted without
behavioral changes and linked to the actual shared runtime. The baseline is the
original `snapshot` plus its `Value` serialization from `8d60c76`. A counting
allocator measured 200 identical polls per path. Legacy full and changed-version
full had the same results:

| History rows × bytes per field | Allocations, old → new | Allocator-requested bytes, old → new |
| --- | ---: | ---: |
| 0 × 128 | 54 → 5 | 6,865 → 2,376 |
| 3 × 128 | 136 → 37 | 22,629 → 11,540 |
| 20 × 128 | 410 → 107 | 79,953 → 47,002 |
| 20 × 1,024 | 412 → 109 | 449,630 → 390,924 |

The steady light path used one allocation requesting 128 bytes for all four
fixtures, with a 64-byte response at version 7 and no error. Larger versions or
an error label change that response size. Full JSON byte counts stayed equal;
68 complete deep comparisons and 34 light/metadata checks covered all statuses,
nullable preview values, terminal states, recovery metadata and history. Object
key order may change; JSON fields, values, types and nulls remain equivalent.

These are Rust snapshot-serialization measurements, excluding request parsing,
JNI string conversion, Kotlin parsing, CPU, peak live memory and device latency.
Earlier design-prototype numbers are not used as final results.

## Combined revision acceptance

The implementation was rebased onto the following-draft repair in main
`7bf13f1fd33d39416cb6d6ca9545185bf5fa05c5`; its three production files are
unchanged from the focused implementation. The no-argument canonical check on
the combined tree passed core 95 plus four contracts, runtime 810 with one
existing ignored test, JNI 24 and updater four, including format, strict shared
Clippy and architecture checks. Desktop compilation then stopped at this host's
missing `glib-2.0.pc`; full native and Android platform checks remain merge gates.

An incremental combined native build produced SHA-256
`ed597bce98ee1acc0849684556b57306bf7572c5c801048df5d39eaa3bcd8f5a`.
All five actual Java JNI scenarios passed again: the conditional-response
contract, graceful TLS tail, fatal TLS error and both old-library full-response
fallback controls. The final main rebase preserved the complete tested tree;
subsequent changes only record this evidence.
