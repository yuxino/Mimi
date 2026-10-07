# v1.5.15 unified integration regression

The user requested that the three independent changes be integrated here and
tested together before merging main. The user's latest authorization allows
publication after both work streams and the unified regression are complete.

## Inputs

- Main baseline: `033b6c30`.
- Configuration menu/details: PR #200, `702b4ecf`.
- Volcano desktop/Android protocol: PR #201, `04f3b423`.
- Traditional Chinese, Korean, French and German UI: PR #202, `300871ef`.

The integration branch retains all source commits and each run-ledger entry.
The four new dictionaries also receive the current-configuration navigation
label introduced by #200. Neither an individual PR's tests nor its earlier
native build establishes acceptance of the combined revision.

## Checks

1. On one recorded clean revision, run `scripts/check.sh`, then Android Debug
   and Release unit tests plus both lint checks. Reuse caches and serialize
   local builds/tests to avoid cache locks and CPU-related test timeouts.
2. Run the actual React browser geometry fixtures across all seven interface
   languages, both themes, minimum/default widths, long names, configuration
   details, dialogs and Windows/Linux-specific controls. Check the two-line
   configuration menus and direct navigation separately from general layout.
3. Inspect the canonical signed UI-only development app across settings,
   tray/control panel and overlay. Cover empty, error, paused, collapsed,
   translating and long-subtitle fixtures. Check repeated current-profile
   navigation, same/different provider indicators, language switching without
   losing paused content, focus and visible action meaning. Keep UI-only state
   distinct from actual capture/provider evidence; restore normal development
   mode after this test.
4. Preserve the shared Volcano fixtures and run both Rust and actual JNI/
   Android checks. Earlier direct-cloud probes are evidence for their own
   revision only. The original #184 disconnect remains unverified and open;
   no new cloud upload, recording or content capture follows from this plan.
5. Record results and remaining device/provider limits in the integration
   ledger, update the same English/Chinese release notes, and review the final
   diff. Merge the integration PR only after required checks pass.

The existing v1.5.15 tag and prepared macOS assets refer to the earlier main.
They must be reconciled and rebuilt before any later authorized publication.
Resume publication only after those gates pass, using assets rebuilt from the
final merged revision.

The combined seven-language review found that German saved language pairs
were truncated in both configuration menus. Keep the configuration name on
one line, allow the description beneath it to wrap, and reserve room for the
additional line when positioning the bounded menu. Rerun the same 112-case
menu matrix before accepting this repair.
