# Settings organization and editing flow

The growing settings page mixed current-session choices, provider models,
connection credentials and configuration management. Credential editing changed
page height when opened, status icons depended on hover, and the subtitle page
used a different container/background from its siblings.

## Responsibilities

- Subtitles: appearance, preview, placement and display behavior.
- Audio & Languages: input sources, active configuration, recognition language
  and translation target. Current language controls have one settings-page home. Original-only is one
  choice in the target picker, without a second skip-translation switch.
- Models & Services: provider configurations, available model/route choices,
  workspace/address, connection credentials, language presets, proxies and
  configuration management. Presets describe a configuration's saved selection;
  they do not duplicate the live source/target editor.
- Save & Export: opt-in local history/audio and existing export actions.
- General: application preferences, updates and advanced diagnostics.

Retain native and existing hash navigation targets. A new category must not
break tray/overlay links to the active configuration or Apple resource setup.
All pages share the same canvas, field alignment, section rhythm, neutral colors,
button sizes and Lucide icon family. Use flat sections rather than a card around
each group. The subtitle preview itself remains distinct from its appearance
controls. Keep explicit actions, readable state labels and normal-size errors;
non-essential explanations remain in the existing help tooltips.

## Editing

Saved connection inputs stay in the same place, masked by default. The inline
view action alone reads a saved key; viewing is not an edit. Save and Cancel
appear for a replacement draft; success or cancellation discards that draft,
while a failed save keeps it for retry. Preserve reveal invalidation on blur,
hide, navigation, deletion and unmount. Safe editor metadata may load when the
form is visible; it must not contain saved secrets.

Qwen-MT model selection has its own model section. Custom recognition and text
models remain editable without exposing or re-entering an unchanged saved key.
Changing a destination still requires appropriate fresh authentication. Editing
an already configured inactive profile saves it without implicitly selecting
it. Copy and Delete stay together in configuration management; deletion uses
its standard confirmation modal. Names remain single-line automatic fields
without a chevron that resembles a selection menu.

New desktop Alibaba configurations start with LiveTranslate. An incomplete
empty workspace may persist as setup metadata; session and check validation
still rejects it before provider access. Saving a key alone does not activate
an unfinished route. Existing profiles, explicit ASR + MT and duplicates keep
their saved mode. Current language choices use the shared capability rules.

Product-facing settings, overlay/tray labels, native window/display names and
Android strings use Mimi. Preserve application identifiers, storage keys,
signing requirements, binary/update filenames and canonical installation paths.

## Single and multiple configurations

With one configuration, Models & Services opens its settings directly. Keep Add
in the header, omit the list counter and Back action, and retain the same editor
if a second profile arrives while editing. After a successful settings load,
zero profiles show the provider picker directly, with a first-service heading
and a short next-step instruction. There is no intermediate empty page or
cancel-to-empty action. Two or more profiles retain the saved-order list. Loading and deletion down to
one profile follow the same rule. Opening the provider picker from the sole
editor keeps unsaved drafts mounted; cancellation restores its position and Add
focus. Revealed saved secrets are invalidated on this transition and stay hidden
on return. No automatic selection, connection check or secret read accompanies
opening the editor.

The same provider picker serves initial setup and later Add actions. Use a flat
list with two columns when its content width allows, one column in narrow
windows, consistent 24 px brand marks, visible service names and adjacent help.
Do not add promotional cards or decorative backgrounds. Keep provider order and
platform availability intact. Picking a provider opens the existing confirmation;
only confirming creates it. Cancel restores option focus, errors allow retry,
and a created first profile opens its editor at the top. Loading/error states
must not masquerade as an empty setup screen.

## Configuration list density

Keep Use / Edit / More in one compact text-action strip. Copy and Delete live
in More, with the destructive color inside that menu and the existing standard
confirmation dialog. The current-state label occupies the Use slot when no
selection change is needed. Keep the list flat, with row dividers and individual
control feedback; avoid repeated button outlines or whole-row hover emphasis.
Move the action strip below the identity at narrow widths while preserving
visible action wording. Menu keyboard entry, arrow navigation, Escape/Tab,
outside focus, blur, scrolling and disabled mutations must remain predictable.

Align the recognition mark with the title, including mixed-provider profiles.
Avoid repeating the recognition provider identified by the primary brand mark;
retain distinct custom recognizer names and the independent translation route.
For ordinary names, show the translation inline as “Apple Speech → DeepL”, with
one primary brand mark and the full translation role in the accessible label.
A configuration alias stays primary with any independent translation route
inline. Only a distinct user-defined recognizer alias needs secondary text. Do not use a second indented
logo and “Text translation” label to inflate every mixed-provider row.
Omit normal credential status from list text, editor headers and help controls.
Missing/unavailable credentials remain visible inline beside the list title. When the active profile can restore a different saved
language pair, retain its Use action and show its current marker beside the name
so this operation stays available. Keep the list in saved order when the active
profile changes; do not move the selected row to the top.
Use the same artwork dimensions as its icon slot to prevent marks crowding text.

## Navigation and reading position

Enter a service editor, duplicated/created configuration or provider picker at
the top after the new DOM is committed. Move keyboard focus to its heading for
an in-page transition. Returning to the list restores its prior position and
entry action; a removed entry falls back to Add. Ordinary metadata updates do
not move the viewport. Category navigation starts at the top and keeps sidebar
focus; preserve unsaved editor drafts when categories are hidden. Native deep
links retain their specific destination focus, including Apple resources.
Confirmation dialogs keep the background position and restore trigger focus
when cancelled. Do not treat a modal as a new scrolling page.

## Verification

Use synthetic browser fixtures for clicks, responsive geometry and review
screenshots; distinguish them from native-device evidence. Check all eight UI
languages at minimum/default widths and light/dark themes, including platform
input controls. Cover saved/missing credentials, cancellation, delayed results,
failed saves, model-only edits, active/paused guards, category navigation and
confirmation cancellation. Actual provider calls, capture and recording need
separate user authorization.
