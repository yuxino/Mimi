# Service name editing and visible service identity

## Accepted interaction

Configuration and independent translation names save automatically after a short
typing pause, on leaving the field, or on Enter. No ordinary Save action or
success notification appears. Saving must not disable the editor, remove focus,
trim the visible draft, or overwrite newer input. Chinese composition finishes
before persistence; composition-confirming Enter never submits credentials.
Keep sanitized failures beside the retained draft with a retry action.

The persisted display name trims surrounding whitespace. Internal spaces,
punctuation and Unicode remain valid. Clearing a translation alias restores its
protocol name; a configuration name is required. Keep the existing bounded name
length and single-line value semantics, including explicit expanded editing.

## State and persistence

Use one shared automatic name field, keyed by profile and translation route.
Coalesce and serialize each field's writes, preserve the latest draft while older
requests or settings snapshots arrive, and keep cleanup/completion scoped to the
original field. While the field is focused, outstanding acknowledgments cannot
replace a newer draft or move the caret. After blur, a clean field accepts external
updates even when they reuse a previous name. A successful command
acknowledgment and the current globally accepted settings snapshot are distinct.

Make the profile name optional in the metadata update command. An alias or proxy
patch must preserve the current configuration name at the storage boundary,
rather than carrying a captured old name. Metadata edits never write credentials
or change recognition/translation selection.

## Display and keyboard behavior

Refresh labels when an alias changes without changing its route identity. Show
both recognition and translation services on the overlay for independent routes;
use one service mark for a shared built-in route. Original-only mode must not
claim that an unused translation service is running. Keep labels single-line,
bound long names, protect icon sizes and verify narrow-window spacing.

The subtitle font field remains beside display mode. After manual selection has
closed its menu, a focused font trigger uses Up/Down to immediately select and
apply the previous/next installed font, without reopening the menu or requiring
Enter. Rapid keys must retain focus and eventually persist the last choice;
searchable-menu keyboard behavior and other selectors remain unchanged.

## Verification

Cover repeated edits with delayed responses, stale broadcasts, concurrent edits
of both names, spaces/punctuation/Unicode, IME, empty names, errors/retry,
profile/route/navigation changes, expanded editing and same-route label refresh.
Check font selection after a filtered manual choice and during a pending save.
Run the canonical checks, browser geometry for all interface languages and both
themes at default/minimum widths, and signed native UI checks without capture or
provider requests. Keep browser platform fixtures distinct from actual Windows
or Linux device evidence.
