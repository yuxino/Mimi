# Identified source-final activity admission

## Problem and scope

Based on `7501a4e577566a0f480bb4b4d835c3a46f96cde9`, after the stable
source-order change. LiveTranslate still emitted an unscoped
`TranslationStarted` immediately before each identified source-final. A late
first final for an older sentence could therefore reopen pending after the
current pair completed, or replace the current six-second deadline. Source
ordering protected the caption but could not undo the preceding activity.

The frozen real emit → mailbox → controller reproduction moves a pending
deadline from t=6s to t=11s when the old final arrives at t=5s. A completed
current pair instead changes from no pending/deadline to the same t=11s wait.
Caption text and history stay intact. Sending the same old final without the
separate Started preserves the prior state and deadline. Actual Java JNI with
a local CONNECT/TLS/WebSocket replay also fails on the old fixed library: the
current pair completed, the older pair entered history, and pending reopened.

## Decision

- Carry a private `starts_translation` bool on the existing sequenced provider
  envelope. Translating identified source-finals publish text, source order,
  content revision and activity together, using one reliable slot. The flag is
  not a protocol event, IPC field, additional queue, or identity map.
- Ordinary sends and local controls default to false. Normalize oversized
  content into the existing Error first, then retain the flag only for an
  identified source-role final. Clearing content or rejecting a full queue
  discards the final and its activity together.
- Keep lifecycle/source selection and source-order registration ahead of the
  shared controller's activity decision. A small core query requires a
  meaningful in-limit ID/source, a non-discarded identity, and current,
  unconfirmed admission. Unknown identities additionally need capacity to
  allocate their ordinal; a valid restored MAX-1 counter cannot admit a new
  identity. A known current identity remains usable at that counter.
- Apply caption and accepted activity in the same synchronous controller call.
  Reuse the existing pending/preview/recovery/timeout transition and return the
  existing application deadline only when a start was accepted. Desktop keeps
  consuming that outcome; JNI keeps using the shared controller and tick.
- Preserve existing source-order, blank-caption and legacy protocol policies.
  Only activity has the additional meaningful-source requirement. ID-less
  SourceFinal/Started, other providers, and original-only behavior keep their
  existing contracts. Raw source-final dedup remains with the aligner; this
  does not introduce a policy for revised unconfirmed finals.

A separately owner-bearing Started would require the same admission checks
while retaining two publication/application points. A content clear, full queue
or confirmation could still intervene; solving that would require batch
publication and additional draft-barrier rules. One final envelope uses the
caption's own admission as the commit point.

## Regression coverage and verification

- A maintained client regression using the real emitter fails before the fix
  and passes after it; an extracted real-source rustc driver confirms both
  outcomes without a product test hook. It checks old-final pending/deadline
  preservation for both pending and completed current sentences.
- Core query regressions cover current/new/old-ranked, confirmed/discarded,
  whitespace/oversized input and ordinal exhaustion. Shared controller tests
  preserve pending preview, recovery, timeout and completion across rejected
  starts, then allow a fresh current sentence to start normally.
- Mailbox tests cover one-slot publication, envelope clone, both audio lanes,
  content clear, one-shot backpressure and inert invalid-event hints. Adapter
  compatibility coverage confirms that unmarked source text cannot make the
  desktop invent or replace a timer. It does not directly construct a private
  marked envelope in the desktop crate.
- One shared synthetic replay drives the real decoder → aligner → emitter →
  mailbox → controller Rust test and the maintained Kotlin actual-JNI TLS
  fixture. It completes the current pair, admits an older pair only into
  history, starts the next current sentence and retains its graceful tail.
- Quick shared passed core 106 plus four contracts, runtime 821 with one
  pre-existing ignored case, JNI Rust 24, updater four, formatting, strict
  Clippy and architecture checks. After correcting a test-only duplicate rank
  so the unseen-old case reaches current-owner admission, that focused test,
  runtime all-target strict Clippy and formatting passed again. The first
  shared attempt could not bind local mock sockets under network isolation;
  the same command passed with loopback testing permitted and no threshold
  change.
- The rebuilt fixed debug library passed the same actual Java JNI replay:
  current completion and older history leave pending false; the next sentence
  starts pending; graceful finish keeps three pairs and closes the transport
  before hard stop. The prior source-order Java/JNI replay also passed with the
  same fixed library. Evidence contains only state results, counts and hashes.
- Two exact maintained desktop helper regressions and a separate marked
  envelope clone/application fixture passed via real-source extraction. This
  does not instantiate the full native timer scheduler or compile the native
  crate. Maintained Kotlin/Gradle and complete desktop native checks remain
  exact-commit CI gates; this host lacks the desktop GTK development dependency.
  No real provider, credentials, audio capture, paid request, device or native
  UI acceptance is claimed. The root will run combined canonical verification;
  this branch ran focused/quick checks only.

## Combined main acceptance

Rebasing onto main `0cd2b2de3d378e051979f417734f08d7f180a063`
preserved every shared, desktop and Android source/test input. The no-argument
canonical check on this combined revision passed core 106 plus four contracts,
runtime 821 with one existing ignored case, JNI 24 and updater four, including
the shared format, strict Clippy and architecture gates. Desktop compilation
then stopped at the host's missing `glib-2.0.pc`; exact-head complete native and
Kotlin/JNI CI still gate merging.

An incremental combined build produced the exact same library SHA-256 as the
fixed library already used for the passing actual Java replay and source-order
compatibility cases:
`7e2ba7abf49b1e7e903ee0e1c18542efba563b59d200459798358cdb74505c2a`.
No new native source or binary change was introduced by that rebase. Subsequent
edits only record acceptance and keep this follow-up beside the source-order
entry in the historical ledger.
