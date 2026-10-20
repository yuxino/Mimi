# Reconcile authoritative names without losing owned drafts

Display-name inputs share NameDraft for profile, recognition and translation
metadata. A native settings event can acknowledge a save before its IPC reply,
and a newer settings event can replace the name before that reply completes.
The global settings response gate preserves the newer event, but NameDraft
previously only recorded its source value while a save was pending. Success
then confirmed the older submitted name without reconsidering that source.
The input remained old; repeating the same prop and navigating back could not
repair it because receive deduplicated that value and the owner stayed retained.

Keep a bounded candidate for a newly received authoritative name during the
current save. Initial source state is not a candidate. Acknowledgements of
older local submissions must not become external changes; confirmation of the
current submission supersedes any earlier candidate. A valid candidate becomes
the confirmed baseline after settlement, including failure, so a newer local
edit returning to an earlier name still persists against that baseline. Only
successful saves may also replace the draft, and only while the local edit
revision is unchanged and composition is inactive. A later local edit,
including an ABA edit or unfinished IME preedit, keeps its existing ownership.
If the candidate equals the trimmed draft, preserve raw spaces and the
input/caret. Failed writes retain their draft, error and explicit retry; a
current own acknowledgement alone is not a new successful baseline on failure.
Adopting an authoritative value never writes it back. No native mutation
identity, credential operation or metadata cache is introduced. Without native
mutation IDs, a reused value alone cannot prove
which operation emitted it; retain the existing own-value acknowledgement
classification and test its ordering boundaries rather than inventing timing.

The same baseline rule applies before a debounce starts: an unowned external
source advances the confirmed baseline of a dirty field, preserving its draft
and error. Otherwise typing back to the old initial value could skip a required
save, or typing the now-current external value could save unnecessarily. Clean
fields still accept external names immediately, while active/interrupted IME
keeps its owned input until composition or explicit confirmation completes.
An interrupted composer also retains its controller when its unchanged preedit
still equals the confirmed value; releasing it would lose that ownership on
navigation before the first input event.

Formal real-component regressions cover own A acknowledgement followed by
external B before a late A success, navigation/release, all three draft keys,
no native echo, new external state without an A echo, current versus older
acknowledgements, newer local edits/ABA, IME, raw spaces and failure/retry.
Twelve pending-save workload/behavior cases failed before the repair, including
the real ServiceProfiles heading-versus-input discrepancy. Additional pre-save
and IME cases exposed four failures; the interrupted unchanged-preedit case
still failed after receive reconciliation until controller release also honored
its existing composition ownership. Forty-five AutoSaveNameField cases now
pass, and the real ServiceProfiles suite passes 167. The canonical quick check
passed typecheck, selected lint, provider-copy and diff checks plus 450 related
tests in fourteen files; the last additional old-acknowledgement control passed
in the 45-case file and its selected lint, without another production change.
Complete frontend acceptance belongs to the integrating tree; no full suite or
production/native build has been run in this branch. Browser/native fixtures
use synthetic display metadata and must not access providers, stored credentials
or audio capture.

Chromium then exercised the actual SettingsView, ServiceProfiles, name inputs,
owner, store bootstrap and response gate with synthetic native IPC. Register
the real settings listener before typing, wait for bootstrap's ready UI, then
reopen the real editor if the loading boundary returned it to the catalog.
Each profile/recognition/translation path received own A, then external B,
then the old A reply. All displayed B, retained the input/focus, survived actual
Back/Edit navigation and made exactly one metadata request; the translation
case also reset its alias to empty. Additional visible-DOM checks verified the
second A write for local ABA, successful owned input without a name echo,
sanitized failure/explicit retry and a pre-debounce return to the old name.
Global B survived the old reply through the real response gate, not a direct
fixture bypass. Browser errors and credential-save/reveal/connection-check
counters were zero. Both real light-theme editor screenshots were inspected.
This does not verify native IPC delivery, native IME or device interaction;
composition ownership is covered by the formal DOM-event regressions. Browser
and the branch's Vite process were closed, and temporary dependencies removed.

The integrating tree rebased this change onto main `4ae49cb` without changing
frontend source, tests, dependencies or tool configuration, then combined it
with the independent embedded popup-mutation fix. Frozen acceptance commit
`7ba8423b4c6d1d14b3f79df6a52be9e94988a9a0` passed all 2,398 frontend tests in
146 files with one worker (232.17 seconds), full lint, provider-copy verification
and the typechecked production build, including icon verification and the
production debugger scan of 17 output files. No assertions or timeouts changed;
the existing large-chunk advisory remains. Subsequent changes only record this
evidence. Native delivery and IME limits above remain unchanged.
