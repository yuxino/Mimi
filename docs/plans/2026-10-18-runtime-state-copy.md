# Runtime snapshot allocation and capture observation

## Scope and invariants

The review baseline is `10d726e`. Streaming events refresh the shared session
projection before the desktop's coalesced publication. The first lane's complete
state was cloned during each refresh, including its bounded history, immediately
before that copied history was discarded in favor of the aggregate history.
Desktop publication then cloned the complete session state before the existing
event converter copied its subtitles again.

Keep all IPC fields, selected-source ordering, aggregate confirmation ordering,
per-source histories, preview ownership, status precedence and activity flags
unchanged. No capture, provider, credential, history-retention or queue policy
changes are needed.

## Changes

- Build the primary subtitle projection explicitly in the shared runtime, keeping
  the existing aggregate history and copying each selected lane into its track
  once. Source identity and the bounded histories remain intact on desktop and
  Android through the same shared implementation.
- Construct the desktop event directly while briefly holding the controller
  mutex. Copy only the status needed after releasing it; client lookup, latency
  collection and diagnostics remain outside that mutex.
- Skip the capture peak scan for processed PCM. Its raw microphone observation
  already exists; processed amplitude cannot create or replace that observation.
  The provider worker's diagnostic peak calculation remains unchanged.

## Evidence and validation

The shared projection regression checks every serialized primary subtitle field
and every selected source's serialized subtitle fields, as well as aggregate
history and pending flags, for System, Microphone and Both selections. The PCM
regression uses real ingress calls to check that processed output cannot invent
sound activity or overwrite raw silence/sound observations.

A temporary Rust allocation benchmark extracts the baseline and changed
production `refresh` functions and the actual wire model shapes. It removes
serialization derives and stubs fields that `refresh` never uses; no provider or
audio input is opened. It asserts identical resulting session states. With 20
history entries and 1,024 synthetic bytes in each source/translation field:

| Selected sources | Before allocations / bytes per refresh | After allocations / bytes per refresh |
| --- | --- | --- |
| System | 86 / 84,998 | 45 / 42,758 |
| Both | 128 / 127,733 | 87 / 85,493 |

These are allocation measurements for this fixture, not an end-to-end latency or
memory-footprint claim. Empty-history allocation counts are unchanged. The PCM
microbenchmark also confirms the skipped scan, but its absolute cost is small.

A separate desktop benchmark extracts the actual old and changed snapshot
prefixes and the unchanged event converter. Forty combinations of history size,
track count and Idle/Connecting/Listening/Stopping/Error status produce identical
complete event values, including confirmation timestamps. With 20 entries of
128 bytes per field, the no-track fixture falls from 92 allocations / 13,384
bytes to 46 / 6,692; the two-track fixture falls from 278 / 41,976 to 139 /
20,988. Empty snapshots still allocate nothing.

Focused direct Rust session and PCM tests passed (60 tests). The canonical
check also passed shared core's 93 unit and four contract tests, runtime's 787
tests (one existing ignored), the Rust JNI adapter's eight tests and updater's
four tests, including their formatting, architecture and strict Clippy checks.
Local mock-socket tests require the execution sandbox's network permission;
running without it produces permission-denied bind failures, not provider calls.
The complete check then stopped at native `glib-sys` because this Linux image
lacks `glib-2.0`. Complete desktop builds and actual Java/JNI checks are CI gates;
native UI acceptance remains separate. No real-provider or audio-capture test
was performed in this review.

## Confirmed follow-up: recovery cancellation during cleanup

This performance change does not repair the following lifecycle defect. After a
health-check failure, `recover_connection` releases its lifecycle lock before
awaiting `cleanup_generation`. Once recovery is advertised and the active
generation is empty, a manual Start may acquire that lock and `cancel_recovery`
aborts the cleanup owner. If capture stop is still awaiting completion, the old
client and event pump have not yet been retired. New-generation `install_client`
then rejects the occupied slot; rollback for the new generation leaves the old
slot untouched.

The temporary `/tmp/mimi-recovery-cancel-extract.py` fixture reproduced this with
actual production cleanup, cancellation, installation and lifecycle-gate bodies
and Tokio task cancellation, replacing native capture stop with a deterministic
gate. Old generation 71 remained installed after new generation 74 failed. The
temporary copied-source driver is not a maintained project test.

The follow-up should give teardown an owner that cannot disappear when recovery
is cancelled, covering initial cleanup, connection rollback and retry failure.
Sharing only a cleanup/cancel mutex is insufficient: cancellation can win before
cleanup has acquired it. Preserve the existing manual-Start behavior during
recovery backoff, and add the gated case to production regression coverage before
changing the ownership model.
