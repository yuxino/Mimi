# UI consistency and feedback / 界面一致性与操作反馈

修改设置、系统托盘菜单或字幕浮窗前，先阅读此规范。修复重复的界面问题时，
必须排查其他页面的同类控件，同时检查成功、失败、弹窗和异步结果迟到的路径。

Read this before changing settings, the native tray menu or the subtitle overlay.
These project rules apply to implementation and review.

## Layout

- Start with the existing design tokens, components, icons and nearby screens.
  Keep the established black, white and neutral-gray visual language. Use
  status colors only for a concrete state; preserve existing provider branding.
- Configuration pickers show the provider mark in both the selected value and
  every option, consistently with the floating controls. Application pickers
  share one icon slot; match fallback symbols to the artwork's visible size.
  Center the SVG's drawn bounds, not just its canvas; asymmetric marks can
  look off-center inside a correctly centered wrapper.
  macOS raster icons must cover at least twice their CSS dimensions for Retina
  displays, without increasing the bounded icon cache or fetching remote assets.
- Name the actual function, state or action in plain language. Do not use slogans,
  promises or personified service names such as “translation partner”. Preview
  captions use ordinary sample text. Describe only capabilities available on
  that platform; omit unsupported features from product copy. Android playback
  capture still needs the system audio-recording permission, so explain that
  requirement where permission is requested.
- Service help explains the actual workflow and relevant choice boundaries;
  a brand, model name or acronym alone is not an explanation. Check copy against
  the implemented route. Keep custom-protocol details in confirmation/editor
  help, with a plain-language purpose in the initial service picker.
  Break multi-part help at semantic boundaries (workflow, then requirements or
  limits); preserve those breaks while allowing narrow-window wrapping.
- Make the title, current state, main action and detail areas easy to distinguish.
  Group related controls, use consistent button sizes and align actions right.
  Do not scatter buttons across large blank areas or add cards to every row.
  In configuration lists, keep actions in one compact strip; move the whole
  strip below the identity when needed instead of using a two-row action block
  that inflates short rows. Keep action wording at minimum width, omit redundant
  icons if necessary, and check artwork bounds as well as its outer icon slot.
  Keep configuration order stable when selection changes; mark the active row. Omit
  routine saved/ready credential state from lists and editor headers, including
  redundant help icons; show missing or unavailable state when the user needs
  to act. A provider mark does not need a repeated brand-name subtitle under a
  configuration alias. Retain distinct user-defined recognition names.
- Navigation must define scroll and focus together. Enter settings subpages at
  the top after content replacement; return to a list at its saved position and
  entry control. Category switches start at the top without stealing sidebar
  focus. Keep modal cancellation and ordinary data refreshes in place. Test
  from a scrolled viewport, including lower-list entries and native deep links;
  a successful click alone does not establish navigation acceptance.
- On desktop, a single configuration opens directly into its settings, with Add in the
  header. Preserve unsaved drafts when adding is cancelled, restore the editor
  position and focus, and invalidate saved-secret reveals while it is hidden.
  Multiple configurations keep a list in saved order.
- Android keeps its existing native list and editor navigation when adding profile
  management. Reuse its row density, typography and touch targets; do not copy
  desktop action strips into a second row or redesign unrelated app surfaces.
  A single saved profile remains a list entry, avoiding a Back-to-auto-open loop.
  Use native single-choice indicators for profile selection. Keep selection and
  editing distinct: tapping an already selected row leaves it selected; Settings
  opens the editor. Do not crowd the name with repeated check/status text. Expose
  the checked state to accessibility, and keep saved order stable.
- With no configurations, show the service picker directly after a successful
  settings load. Keep one short next-step instruction, omit an intermediate
  empty page and cancel-to-empty action, and create only after confirmation.
  Reuse the same responsive picker for later Add actions; loading failures
  remain retryable errors rather than being presented as a fresh install.
- Keep essential labels, choices and actionable errors readable at normal text
  size. Put non-essential explanations behind hover/focus help icons. Long raw
  diagnostics and advanced details start collapsed; bound long lists and text.
- Verify wrapping, narrow windows, long labels, empty content and light/dark
  themes. Interactive controls, including help and toast dismissal, keep pointer
  cursors; preserve disabled, text-input, slider and resize cursors.
- Live subtitle geometry needs a stable reading edge as well as unclipped text.
  Automatic glides must retain ownership across row measurement and streaming
  revisions; pointer clicks cannot turn their pending scroll events into history
  intent. Test chunk growth crossing the compact line budget with real browser
  geometry, and finish pointer interactions even when release occurs outside.
  Preserve status-light animation identities when pausing invisible tracks;
  system chrome rules must not override their explicit motion setting.
  Resize ownership ends on capture loss, unmount, disabling and primary-button
  loss as well as release. Clear it before releasing capture; duplicate native
  completion without an active gesture must not replace a newer presentation.
  In normal Android mode, bottom-align the current measured lanes, matching the
  desktop reading edge. Growing reference text must not push an unchanged
  translation down; preserve complete glyph lines and deliberate history reading.
  Check changing line counts in native draws rather than only static screenshots.
- Windows control-window measurements are CSS pixels, not native logical pixels.
  Report the WebView's `devicePixelRatio` with capsule width and panel height,
  and divide by the native window DPI scale before sizing against monitor bounds.
  WebView `ZoomFactor` alone does not include the observed Windows text scaling.
  Preserve accessibility scaling; size both fixed and measured dimensions in
  the same units. Re-measure on viewport/DPI changes even when content width is
  unchanged, and verify the actual native capsule and expanded panel.
- Reserve readable label space and bound picker columns. Put ongoing device
  feedback in `SettingsRow.feedback`, below the label/control grid; never put a
  long status sentence beside a picker inside the auto-sized control column.
  Stack application and output pickers at narrow widths together.
- Keep 16 px between directly adjacent setting rows without a divider; retain
  existing spacing around separators. A no-overflow result alone is not
  visual acceptance: inspect the screenshot for touching control borders,
  vertical rhythm and heading/section separation before presenting it.
- Check all supported interface languages (Simplified/Traditional Chinese, English, Japanese, Korean,
  French, German and Thai) at the default and minimum window widths.
  Include Windows-only output choices and Linux-only recovery states even when
  developing on macOS. A hidden native capability must have an explicit local
  fixture; a successful macOS screen alone cannot cover that control. Reuse
  `scripts/fixtures/settings-layout.html` and `scripts/verify-settings-layout.js`
  for actual browser geometry, and keep the native-device evidence separate.
  In a fresh checkout, load the Vite root page once before opening fixtures
  that import optimized dependencies from `/node_modules/.vite/deps/`.
  Keep hover, animation and media checks in the foreground, with one browser
  controller at a time. Inspecting another page or native app can pause frames
  or playback; restore focus before retrying and record interrupted playback
  as incomplete acceptance.
  Memoized children that read localized copy must subscribe to the locale or
  receive it as a prop. Check paused/static content too; do not remount subtitle
  history to refresh labels and lose the reader's scroll position.
- Bound presentation work to its owner: closed pickers need no option rows,
  unrelated element scrolls need no tooltip measurements, and unchanged subtitle
  text needs no repeat formatting. Preserve keyboard intent when skipping an
  equal cursor index. Pause presentation polling on document visibility changes,
  not ordinary focus loss; keep explicit playback lifetime independent.
  Current transcript results need a session/retention identity beyond item count.
  Diagnostics reentry must request a fresh snapshot after any serialized in-flight
  operation, and old visits must not replace the new view's report.
  Reuse unchanged subtitle lanes by all presentation props, preserving row identity
  and locale subscriptions. When the last row changes, retire its measurement
  callback so a late old-row resize cannot replace the new reading budget.
  Process-stable native capability reads may be shared within one WebView;
  failures must remain retryable and every other window must verify its own IPC.
  Keep pending slider previews stable across older save echoes. Confirmations
  and rollbacks must respect field ownership, external revisions and subsequent
  ordinary settings; a completed iconless application query is a valid result,
  not an instruction to enumerate again.
- All interface text, native controls, tooltips and portaled menus use
  `--mimi-ui-font`. Order CJK fallbacks for the interface language and include
  Windows non-UI family names; do not add a separate stack to a shared tooltip.
- Anchor help to its visible heading, field label or action with a small gap.
  Do not leave an icon-only help row below a list, push it to an unrelated
  far edge, or use a shield to disguise generic help. Credential storage help
  uses the shared labeled `CredentialStorageHelp`; diagnostic privacy help
  stays with the page heading. Inspect all sibling uses when this recurs.
- Credentials use one editable input with an inline eye and paste action. Saved
  secret presence determines whether the eye can read an existing value; a
  usable service configuration alone does not imply a saved optional key.
  Keep localized visible action text, accessible labels and tooltips for the eye. Show stored values
  in that same input only on explicit request; viewing alone is not an edit.
  If a replacement draft exists, the eye toggles that draft without reading or
  overwriting it. Clear loaded values and invalidate pending reads on focus
  loss, hide, close, route change and unmount. Nonsecret configuration values
  load into editable fields locally, never into a secret-containing snapshot.
  Keep editor-only service URLs out of global snapshots. Submit unchanged fields
  as unchanged so displaying a saved address cannot clear its credential. A
  keyless destination stays valid without offering to reveal a nonexistent key.

- Store loaded nonsecret configuration in the actual editable value, never a
  placeholder. Empty fields use concise localized entry hints; concrete examples
  must not look like saved/default values. Long persistent text fields share the
  inline wrapping editor, with selection/editing and validation checked at narrow
  widths. Expansion stays within the field group and must not trigger auto-save.
  Never enable this by inferring that `type="text"` means nonsecret: revealed
  credential fields also use that input type.

- Configuration, custom recognition and translation display names use the shared automatic name
  field: preserve the visible draft, spaces, caret and IME composition while
  saving quietly. Serialize/coalesce writes per field, keep failed drafts with
  inline retry, and update only the requested metadata so an alias or proxy
  cannot replay an old configuration name. Keep compact labels single-line;
  expanded editing is explicit.

- A preference with a coalesced save queue must keep that queue in the settings
  window's lifetime when its page can unmount. Recreated or simultaneous controls
  share the latest unsent choice; an older page must not submit it after a newer
  choice. Preserve a legitimate queued choice when leaving without a replacement,
  and keep previews/notifications owned by the currently mounted view.

- Proxy system/direct choices save immediately. A custom address saves on blur,
  Enter or explicit paste; typing stays local and saving does not run a probe.
  Preserve failed mode/address drafts for retry and deduplicate Enter followed
  by blur against the acknowledged value. Keep the active-session save guard.

## Choose feedback by purpose

| Situation | Required behavior |
| --- | --- |
| Copy, refresh, explicit save, export, delete or quit outcome | Shared transient toast; no full-width banner or paragraph that moves page content. |
| Instant preference change, including switches, sliders and color choices | Save quietly on success; show a sanitized failure toast. Do not swallow the rejection or show a success toast for every slider event. |
| Invalid input or failed save with an unsaved field to correct | Keep the draft, error and retry beside that field. Toasts do not replace field validation. |
| Connection check | Keep progress, result and actual request duration beside its triggering action; recognition and translation remain independent. |
| Ongoing session, device or local-storage problem | Keep the relevant state/error and recovery control visible until resolved. |
| Update download/install/restart state | Keep progress and the next action visible. An up-to-date acknowledgement or failed Releases link uses a toast. |

Use known error labels and localized messages. Never display raw provider/OS
errors, credentials, audio or subtitle content in a notification. Name the
actual failed operation: deleting history is not a history-read failure.

## Reuse the shared implementation

Settings uses [SettingsPrimitives](../../src/windows/settings/SettingsPrimitives.tsx)
for rows, sections and actionable inline feedback, and mounts one
[SettingsToastRegion](../../src/windows/settings/SettingsToast.tsx) per window.
Use [useSettingsToast](../../src/windows/settings/useSettingsToast.ts):

- `beginToast()` reserves the notification slot before an asynchronous action.
  The returned notifier reports its result; pass `true` for a failure.
- `runWithToast(action, failureMessage)` handles quiet preference saves.
- New operations replace older notifications. Success expires after three
  seconds, failure after eight; either can be dismissed manually.
- Navigation, native/DOM blur, close, hide and unmount clear notifications.
  Late callbacks must not revive them after those boundaries or a newer action.
- A modal makes the background inert. Keep the same toast inside the active
  dialog so it is visible, announced and dismissible. Preserve the focus trap,
  Tab/Escape handling and the destructive-action confirmation.

Use each product surface's existing shared feedback mechanism. Do not import
settings UI into the overlay or replace an actionable capture state with a
disappearing notification.

## Cross-page review

Before calling a UI consistency fix complete:

1. Find sibling instances across settings categories, native tray menu and overlay.
   Search shared components, callers, inline feedback, alert/status elements,
   save handlers and catches. Include platform-specific controls and hidden
   editors; a single screenshot is not the scope of the audit.
2. Classify each instance using the table above. Reuse the shared component or
   helper instead of copying a page-specific workaround. Check both success
   and failure, including ignored/rejected save promises.
3. Verify rollback/draft retention and retry, busy/disabled controls, duplicate
   clicks, repeated actions, navigation/unmount and late results. Check toast
   expiry/dismissal and behavior inside an active modal when applicable.
4. Run relevant existing regressions and add focused coverage for new behavior.
   Run `./scripts/check.sh`. For UI changes, inspect the affected instances in
   the signed development app using the [native-test rules](common-regressions.md).
   Keep UI-only fixtures separate from real capture/provider evidence.
5. Review the final diff and report the pages/paths checked plus any intentional
   differences or unverified platform behavior. Update this guide when the
   shared rule changes; record new concrete pitfalls in `common-regressions.md`.

Collapsed overlay tooltips must fit the 54px native window and leave its drag,
pause, expand and close buttons uncovered at the minimum width in every UI
language. Use a short gesture hint when the expanded instructions do not fit;
retain a complete accessible action. Verify geometry with the actual overlay
components and confirm the affected gesture in the signed native app.

For Android, appearance preferences must refresh the existing service-owned
subtitle views. Verify actual native text size after slider input; a changed
saved integer or preview alone does not establish a live update. Preserve
subtitle history and the reader's position for ordinary appearance changes.
An empty live-caption flag is insufficient to show a padded overlay: require a
nonblank displayed caption or actionable status, while retaining expanded
controls and the immersive exit during silence. Check all eight interface
languages at normal and large font scales; fixed-height labels and a nominal
two-row toolbar can still clip long translations. Language resources, picker
and system locale configuration must match the owning desktop language catalog.

Android help icons show explanatory copy on native pointer hover/long press and
open the same scrollable details on tap or keyboard activation. Keep a labeled
48dp touch target beside the heading, and one help dialog per activity. Check
actual hover dismissal and icon/label geometry at large font scales. Optional
ready/running explanations stay in help; setup, permission, silent-capture and
storage recovery stay visible. First-run audio-sending and charge disclosures
remain visible before the user starts a session.

Android's default tooltip can ellipsize a complete explanation after three lines,
even when its tooltipText is intact. Use the shared bounded, scrollable help
popup; check the rendered line ellipsis and an actual hover screenshot. Native
UI tests must inject mouse pointer properties through the system input channel
and restore fixtures on assertion failure.

Service-owned Android UI resolves application locales explicitly on every API
level. Android 13+ application-locale persistence does not guarantee that a
display/window/service resource context matches an AppCompat activity. Following
System resolves current system locales instead of trusting a stale resource
context. Native language checks include a deliberately different resource locale
and the actual waiting overlay after immersive restoration and returning from
settings. New non-immersive sessions open the interactive panel so their state
and actions are visible. Respect the user's height and bound its reading viewport
independently of history length. Large font controls and caption reading space
must both remain visible.

Android follows live captions with measured visual-line budgets for the selected
languages. Neither original nor translation may occupy the other lane's last
complete line. Short text gives spare height to the other language; resizing and
font changes recompute from the native reading viewport, including visible time
metadata. Deliberate upward scrolling opens the complete bounded text and keeps
the reader's position through updates; returning to the bottom resumes following.
Native checks must assert visible glyph bounds for both lanes, not just populated
TextViews, an outer minimum height or pixel parity between two clipped renderers.
Android TextView's maxLines measures the first lines; a following tail must
measure the actual last lines too. Fallback fonts can have different glyph
heights within one sentence. Size and allocate from those native layout bounds,
and check the first and last visible glyphs after wrapping, font and size changes.
Continuous native checks must also inspect the visible tail at draw time.
TextView's own pre-draw scroll reconciliation must finish before tail-following;
do not cancel a whole-window draw to correct its position. Read one atomic
shared display snapshot and skip identical text assignments on health/status
updates. Coalesce pending paints without dropping shared final events. Keep the
window minimum stable across incoming scripts by measuring multilingual font
metrics from the chosen typography, while fitting actual visible glyphs inside
that window. The selected size is the main reading size in either single-language
mode and for the translation in bilingual mode; the original is a quieter 90%
reference. Avoid an extra translated-size/bold boost. Verify all three modes
through continuous updates, height edits and real portrait/landscape round trips.
Cancel transient drag/resize state on configuration changes and ignore remaining
events from the old gesture, so its final up cannot save dimensions measured in
the previous screen or open an unintended editor.
Observe sentence transitions in real playback as well as glyph bounds. A new
original can arrive while its translation is empty; translated reading modes
must keep the shared retained confirmed pair and its metadata readable rather
than flash listening. Select that whole pair, never pair its old translation
with the new original. Read the raw shared recognition lane in original-only
mode even when complete translated pairs are held. Use confirmation time only
for the same final source owner, including repeated identical text. Use the
existing shared snapshot without another retained transcript or reducer, and
cover first speech, history disabled, clear, hidden and next-pair arrival.
Sentence dividers and local confirmation time default off and are controlled in
both appearance entry points. Use the shared final's time, including with history
disabled, hide time on drafts, and avoid duplicating the current pair in history.

Before/After screenshots must show stable native frames: wait for the owning
activity to regain focus and for modal exit/entry animations before capturing.
Inspect every saved image against its scenario; a successful file write or test
result does not prove that a screenshot shows the intended page.

Android expands the native scrollbar's mouse hit area beyond its painted track.
At narrow widths and large fonts this can intercept hover over a visible help
button. Use `ControlScrollView` for native product scroll containers: controls
receive hover within their clipped visible bounds, while the scrollbar keeps
its handling elsewhere. Account for the root window offset in dialogs. Verify
actual system-injected mouse movement across the icon, keyboard focus, touch,
popup dismissal and popup/anchor separation; label geometry alone misses this.
Dismiss pending and visible help when its native window loses focus and when
the selected section changes. Gate showing on the anchor's current window
focus and hover/keyboard focus. A background activity's restored keyboard
focus must not put the old explanation over a dialog. Native checks compare
the rendered explanation with its intended anchor, not just popup bounds.

Android uses one primary activity with Captions, Services and More. Caption
content, display mode and style belong to the same section; do not split them
into duplicate primary tabs or add another preview. Keep common controls
visible without an accordion. Keep the single preview pinned while settings
scroll, above the controls in portrait and alongside them on wide screens. The
floating appearance dialog edits the actual caption window and has no duplicate
preview. Keep its complete toolbar, provider footer, action appearances and live
content/status visible, but disable preview clicks, scrolling, dragging, resizing
and accessibility actions without dimming it. Fit the complete frame above the
editor, scaling uniformly when necessary; restore interactions and ordinary
geometry on dismissal without changing saved bounds. Inspect both this entry
and the primary Captions preview when checking frame parity.
Direct color swatches
wrap before their touch targets shrink below 48dp. Retain section scroll positions and
selection through recreation. Keep status and Start/Stop visible in every
section. Use one footer row only when actual localized labels fit; stack otherwise.
Distinguish a new activity from explicit CLEAR_TOP/SINGLE_TOP navigation to an
existing instance: `startActivitySync` waits for a new instance and hangs on reuse.

Android immersive mode has a Captions entry/restore action backed by the same
preference as the floating panel. Preview and overlay use `CaptionTypography`
for lighter text, chosen-size translation and a tight contrast shadow. Apply the
chosen left/center/right alignment to both modes and the listening placeholder.
Keep source text secondary and preserve the user's color/size preferences.
Change the existing caption window in place, retaining the immersive dp anchor
and normal panel bounds. Temporary viewport/inset/editor clamps must not rewrite
saved placement. Keep
the restore control adjacent to captions, with a 48dp touch target and localized
width; caption touch-through must not make restoration unreachable. Test repeated
mode notifications, rotation, content/font reflow and returning to the normal
panel. Host geometry tests do not establish native window or touch
acceptance.

Android credential acceptance checks the actual password transformation after
single-line setup and typing, then both eye states. An input-type bit or an empty
field alone does not prove masking. Keep unsaved drafts local and discard them
on editor exit. Rebinding unchanged native help must preserve its hover/focus
state while capture health or subtitles update.

Native touch fixtures first scroll controls into their visible bounds and wait
for transient system clipboard surfaces to close. Language fixtures navigate to
More before selecting a value. Verify actual wide compositor bounds before
calling a run landscape acceptance; rotation settings alone are
insufficient when the launcher requests a fixed orientation. Check the caption
reading viewport as well as the panel's outer bounds at large fonts.

Android appearance previews reuse the actual normal/immersive caption Views.
Measure at the real safe window width and height, then scale the complete frame
uniformly into a stable pinned stage; never independently restyle or rewrap a
sample card. Include the actual toolbar, provider row, typography, alignment,
waiting/error feedback and opted-in bounded history. Decorative preview actions
must not navigate or start capture. Width, height, font and streaming changes
must not move the sliders under the user's finger. Verify native frame geometry
and normalized pixels, both slider endpoints and the control's screen position.
The desktop/Android subtitle background is pure black, with default background
opacity 80% and Android whole-window opacity 100%. Preserve explicitly saved
values; background opacity must reach solid black at 100%. Font-only changes
must not alter either alpha. Restore all fixture preferences after acceptance.

Live native checks inspect external overlay labels only after their window is
visible. Mimi intentionally hides the overlay over its own activities; checking
the hidden model label before returning to the playback app is not acceptance.

Android pinned-preview acceptance checks the rendered source/translation rows,
not only the preview container. Inspect the floating dialog separately: the real
caption window must stay visible above it, with live typography/color changes
and reachable controls. Normalize
`getGlobalVisibleRect` root coordinates with the root window origin before
comparing them with `getLocationOnScreen` glyph positions.

On API 35, a nonfocusable overlay's View and accessibility screen origins can
retain a previous frame after native movement/resizing while compositor pixels
already show the new position. Native touch fixtures resolve local offsets
against the window's gravity, attributes and safe display frame, cross-check
that frame against screenshot pixels, then verify the actual gesture result.

A dialog can cap its custom panel below a fixed child height. Bound the
workspace to the actual measure spec; otherwise its ScrollView reports a larger
viewport than the visible panel and cannot bring lower controls fully into view.
Native appearance acceptance checks the complete visible touch bounds and
injects a real tap, then verifies both saved color and the existing caption view.
`performClick` and correct saved values alone do not prove that a user can reach
the control.

Native ScrollView tests must distinguish main-loop idle from a completed fling
or smooth scroll. Wait for a stable reading offset before checking that a new
caption preserves it; wait for the actual bottom before injecting the return-to-
live gesture. Reset options changed by an earlier history fixture before testing
both switch directions, then verify the saved state and the visible control.
Hold the current pair, font and style constant when isolating retained-history
height; a seed that also changes the current language can change native fallback
font minimums. Let preceding mode layouts settle first. In large-font landscape
cases, the complete bilingual minimum can already exceed the 70% preferred
height. Require visible growth where space permits, and verify the native safe
bound and saved maximum where that minimum prevents further growth.

An inert immersive restore companion must also use a native touch-through
window while the quick editor is open. A nonclickable View can still intercept
an underlying switch. Reserve the editor's occupied bounds when positioning
that companion, and verify its native frame clears the editor at font scale 2;
ordinary dismissal must restore the clickable native target.

A bottom floating editor must remove Material's centered-dialog background
insets. Transparent top/bottom margins still count toward decor bounds and can
consume the entire caption area at large fonts. Native checks compare selected
height endpoints, actual caption space and editor separation; correct saved
percentages alone do not prove that the window changed size.

Android normal captions use one panel with a saved width and height; do not add
expand/collapse states. Drag the top handle to move and the corner to resize both
dimensions. Keep complete action targets and at least 64dp of reading space,
clamping only displayed bounds at large fonts or small screens. History must
not enlarge the user's chosen frame. Waiting copy uses the chosen font, color
and alignment so live edits are visible even before speech.

Center Android's drag mark, language route and provider group against the whole
caption window, not the remaining space beside an asymmetric action. Keep
immersion left and resizing right; use a separate centered provider row when
names cannot fit between symmetric action margins. Native geometry checks
compare window-relative centers and action separation after resizing and locale
changes, including both single and independent provider badges.
Measure wrapped footer labels at the exact width assigned by their parent.
An AT_MOST text measurement followed by a narrower exact width can add a line
without reserving its height. Verify locale changes at font scale 2, including
the complete action's rendered line height, rather than only its touch target.
When space cannot fit both the immersion label and its redundant icon, omit
the icon so the readable localized action takes precedence over decoration.
Keep mobile floating service identities compact: show brands and marks, with
model details available in Services. A tap opens that section in the existing
main activity. Initialize background/opacity before adding the native window
using the same paint path as preference updates; a font-only change must not
apply a previously hidden background default. Native checks use nondefault
saved paint values and compare compositor pixels across font and opacity edits.
