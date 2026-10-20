# Provider draft admission across a late previous final

## Observed failure

The shared LiveTranslate path can observe source A's final, source B's draft,
then A's translation final. The aligner correctly stamps each event with its
source identity. If the session consumer has not caught up, the mailbox reads
the reliable A final before the replaceable B draft and discards B behind its
global publication-sequence barrier. Confirmed A history survives, but the
current source remains A until another B event arrives.

The regression holds the real mailbox receiver while the real runtime aligner
publishes that sequence, then reduces the delivered events through the shared
session controller. Provider IDs are deliberately opposite lexical order.

## Decision

Use source order already owned by `mimi-core::LivePairAligner`. A cumulative
offset makes its existing bounded source-order window's ordinals stable when
the oldest identity is evicted. The read-only lookup returns no order for
identities outside that window. Older serialized states default the offset to
zero; clearing content clears both the window and offset.

The LiveTranslate adapter captures the ordinal in the same aligner operation
and lock as each output, then publishes it as private mailbox metadata. It is
not a provider-wire, session-snapshot, IPC, or Kotlin field; the opaque core
state only gains the backward-compatible offset described above. The transport has no second identity
table, never sorts opaque provider IDs, and emits no extra reliable messages.

Only a queued stamped draft behind the current barrier can use this evidence.
Both source and translation lanes may cross an identified final when their
content revision matches, that final is still the latest barrier, and their
known source ordinal is newer. The same identity stays frozen after its final.
An additional constant-size delivered-source frontier retains the highest
ordinal admitted in that revision and whether its source or complete pair is
final. A later-arriving older final cannot lower that frontier. Queued drafts
with a known translation ordinal also advance it: the aligner has already
observed their source, even if that source draft coalesced. Queued drafts
below it stay obsolete; at the same ordinal a source final closes only the
source draft, while its translation preview remains admissible until the pair
is final. This protects coalesced middle sources and already-delivered drafts
without retaining another identity collection.
Unknown order, an older identity, other lifecycle/content barriers, and another
content revision retain conservative admission. Events published after the
barrier retain their existing admission rule.

The aligner's existing bounded identity/replay window is unchanged. Eviction
does not rewrite metadata already attached to queued events. This change does
not promise recognition of arbitrarily old provider replays after all bounded
identity state has forgotten them.

## Verification boundary

Direct Rust tests cover the deterministic held-receiver backlog, both lanes,
old and unknown identities, repeated finals, lifecycle/content boundaries,
and core ordinal compatibility, eviction and clear. The shared synthetic
live-pair contract retains the same wire and aligned-event shapes.

The maintained actual JNI loopback test uses normal setup and phased provider
frames to check late-final history, the following draft, old-source replay and
Kotlin publication. JNI polling reads snapshots rather than draining the
provider mailbox; there is no public consumer-pause gate. Its protocol/bridge
coverage therefore supplements the deterministic Rust regression and is not
claimed as a deterministic old-mailbox failure. No production test hook is
added. No real provider, credential, capture, or native device test is involved.

The direct quick/shared and actual Java/JNI evidence was frozen as `4a479e8`
on `a71680c`. Replaying this change on `8d60c76` preserves all three shared
production files byte for byte, while retaining that base's PCM recovery and
merging its Kotlin fixture modes. The earlier JNI library does not validate
the combined adapter build; that exact revision needs its own integration
checks.

Root completed that combined verification at `aa01686` on merged main
`47c5a35`, with the same tree as `d631a17`: canonical passed core 95 plus four
contract tests, runtime 810 (one existing ignored), Rust JNI 18, updater four,
formatting, strict Clippy and architecture checks. Desktop compilation stopped
at the host's missing `glib-2.0.pc`. The combined JNI debug library (hash
`06fc0bcd`) passed the same actual Java/JNI three-phase loopback test: one
connection, eight content events, twenty polls and all preview/history/replay/
graceful-tail assertions. Deterministic backlog remains the maintained Rust
regression's role; full native and Kotlin/JNI CI are still merge gates.
