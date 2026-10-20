# Reuse history date formatters within each render

An unchanged archive snapshot arrives every second while SessionExport is
visible. The page previously constructed a date formatter for every retained
session, a time formatter for every session button and a time-with-seconds
formatter for every visible transcript entry. Twenty retained sessions from one
older day plus a real thirty-entry transcript page created seventy formatters
on each unchanged poll. A five-poll real-component probe counted 350 constructions
while making no new transcript reads and preserving article DOM identity.

Keep all three option sets distinct: group headings use numeric year/long
month/numeric day, session buttons use two-digit hour/minute, and transcript
times additionally use two-digit seconds. ISO `datetime` attributes remain
unchanged. Construct each formatter lazily once within the current render and
reuse it for its rows. Group labels are calculated only when a date group is
first created. The same probe now counts three constructors per poll, fifteen
over five polls; it measures work count rather than application CPU or latency.

Do not cache across renders or memoize the page. Existing archive polling still
refreshes current metadata and new transcript counts immediately, recalculates
Today/Yesterday across local midnight and constructs fresh formatters using the
current UI language and default time zone. Empty lists create no formatters.
History selection, pagination, playback, deletion, node identity, data retention
and native behavior keep their existing owners and lifetimes.

The new formal component regressions check the seventy-to-three workload bound,
one label format per date group, unchanged row identities, immediate new archive
state, all eight UI languages and distinct time precision, ISO timestamps,
local-midnight labels, an injected default-time-zone change and empty history.
The workload case fails against the old implementation with seventy constructors;
the other eleven behavior cases already pass. Together with existing lifecycle,
selection, playback and deletion tests, the focused suite passes 46 cases.
The quick frontend check passes typecheck, selected lint, provider-copy and diff
checks plus 135 related tests across nine files.
The time-zone test uses real Intl formatters with a controlled default zone;
it does not establish operating-system notification delivery or native UI
acceptance. No provider, capture or private session data is used.

Independent review found no blockers. A separate extraction of the actual old
and new grouping functions produced identical output for 448 combinations of
seven time zones, eight locales and eight DST/midnight instants. A real Chromium
SettingsView with synthetic native archive/history/transcript data observed
three polls, nine formatter constructions, zero extra transcript reads and
unchanged article identity/text. Complete frontend validation with the font
ownership repair passed 2,345 tests in 145 files, lint, the typechecked production
build and all 17 output checks. Rebasing onto `a71680c` preserved the tested
frontend source. These checks do not establish native time-zone notifications.
