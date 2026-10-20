# Preserve provider source order at subtitle admission

## Observed failure

The LiveTranslate aligner observes source A's draft, source C's draft, C's final,
then A's late final. Its source ordinals correctly retain A before C. While the
mailbox receiver is held, the two replaceable drafts coalesce and the reliable
finals are delivered first. The reducer used to register C before A from this
delivery order, making the old A final replace C. Direct reduction of the same
four aligned inputs keeps C current.

The failure was reproduced using actual source at both `90fa8cb` and the prior
provider fix. A maintained real aligner → held mailbox → controller regression
also failed on this change's base, `d5664012`. It covers the four inputs alone
and completed translations arriving in either order. Dropping A's final pair
would hide the symptom but lose legitimate confirmed history.

## Shared admission

Carry the aligner's existing private ordinal through `ProviderEvent` into the
shared session controller. Register its stable order before reducing the
caption, after text and lifecycle admission. Both source and translation lanes
use that evidence; missing metadata does not erase previously learned order.
The zero-based ordinal is checked and normalized to the reducer's positive
range. No public wire, session snapshot, IPC, or Kotlin ordering field is added.
No additional reliable event or identity table is needed.

Each entry in the reducer's existing 64-identity window gains an optional known
order. Stable known order is independent of its existing local arrival ordinal.
Known identities compare by provider order while an unknown current owner uses
the existing arrival rule. Known-order and arrival high-water marks never move
backward. When a registered unknown translation becomes the visible owner in a
mixed stream, its arrival evidence is retained too. Pure unknown streams keep
their existing behavior. Provider IDs remain opaque and are never numerically
or lexically sorted to infer order.

Live source, live translation and retained complete display each keep a
constant-size optional owner order. This evidence survives bounded identity
eviction; a higher known successor can still advance. The live comparison uses
the actual visible owner when an old retained complete pair hides a raw source
line. Live and complete-display frontiers remain independent: an old completed
pair may fill the retained display while C is unfinished, but cannot replace C
or a newer completed display. Every valid new final pair still enters bounded
history and the existing archive path exactly once.

New reducer fields default to absent for older serialized state. Validation
checks order bounds, contradictions, owner evidence and frontier coverage.
Clear and the existing generation/session boundaries reset this content scope;
confirmed history/display retain their established behavior. A new client may
restart at ordinal zero without competing against a previous client's rank.
System and microphone controllers keep independent scopes, including equal
provider IDs and ordinals. Evidence outside the aligner's bounded identity
window remains unknown; this does not promise arbitrary-age replay detection.

The existing `SubtitleSourceBoundary` contract remains separate. Volcano's
provisional clock can be corrected before confirmation, so it cannot be treated
as the aligner's stable rank. Its original visible behavior, correction, bounds,
and arrival-counter contract are preserved. A clock boundary cannot retarget an
identity for which stable order is already known.

## Native application and timeout ownership

The shared owning application returns whether an accepted event resolves the
current translation timeout, plus a newly started deadline when appropriate.
The desktop adapter applies the full envelope under its existing content gate
and controller lock, then updates the timer from that result. Old, invalid or
duplicate identified final pairs cannot cancel another current request's timer.
Generation, revision, pause, error normalization, recovery and terminal-session
guards retain their existing order. Desktop termination still ends all selected
sources. JNI's normal receive and both graceful-finish drains use the same
shared envelope application.

One existing limitation remains outside this fix: the production LiveTranslate
emitter publishes an ID-less `TranslationStarted` before each source final.
A late old source final can therefore rearm the current pending deadline before
its ranked caption is applied. Actual-emitter evidence confirms that rearm is
unchanged; the later old identified pair preserves the resulting deadline and
history. Fixing ownership of that start signal is separate work.

Production changes are limited to the core reducer, runtime session controller,
runtime provider envelope, desktop session adapter and JNI runtime adapter. The
client change only adds the maintained regression; polling and UI behavior are
outside this change.

## Verification boundary

The maintained regression reads the shared synthetic live-pair fixture and
compares held and direct delivery, including both completion orders and history.
Core regressions cover late metadata on a translation-only preview, mixed
known/unknown ownership, hidden retained lines, source/display eviction,
independent frontiers, reset, legacy serialization, forged state and bounds.
Shared application regressions cover both audio sources, stopping admission,
revision filtering, reconnect, current deadlines and duplicate confirmations.
The desktop helper has a maintained timeout/history regression; its exact source
and test were additionally compiled with actual shared source outside the GTK
dependent desktop crate. Independent actual-source review ran 100,000 fixed-seed
mixed operations with validation after every operation.

Quick shared checks pass core 104 plus four contracts, runtime 814 (one existing
ignored), Rust JNI 18, updater four, formatting, strict Clippy and architecture
checks. A fixed debug JNI library passes actual Java exchange through a local
CONNECT/TLS/WebSocket server: current final preserved, old complete history
retained, stale replay ignored, current translation preview admitted, and current
completed history retained through graceful finish. The previous draft-barrier
loopback is also retained as an affected-path check.

The maintained Kotlin/JNI test covers the same normal and finish phases and
SubtitleBus publication. Public polling cannot pause the native mailbox consumer;
only the Rust regression deterministically holds that backlog. Local Java
evidence does not compile Kotlin or replace exact-commit platform CI. The host
lacks GTK development libraries and Kotlin/Gradle tooling; full desktop and
Kotlin/JNI suites remain CI gates. No real provider, credential, audio capture,
device or native UI was used.

## Combined main acceptance

The repair was combined with main's conditional JNI polling fix, then rebased
onto `fdc765f9514322c80f908f830a174f63e6f1411d`. All shared, desktop and Android
source/test inputs stayed byte-identical across the last rebase; the added
frontend listener repair already had separate full frontend/Android CI.
The no-argument canonical check on the combined native tree passed core 104
plus four contracts, runtime 814 with one existing ignored test, JNI 24 and
updater four, including strict shared checks. Desktop compilation stopped at
the host's missing `glib-2.0.pc`; exact-head native/Kotlin CI remains the gate.

The combined JNI library SHA-256 is
`020316e740864b85e2138754fcf4ddcf248d87e4711c69d72cc201d7e0c7a7db`.
Actual Java calls passed the source-order scenario (one connection, ten content
events, twenty polls) and the previous draft-barrier scenario (one connection,
eight content events, twenty polls). Both preserved their caption/history,
preview, graceful-tail and transport-close assertions. Neither network run
claims deterministic mailbox backlog. The final evidence-only commit changes
no tested production code.
