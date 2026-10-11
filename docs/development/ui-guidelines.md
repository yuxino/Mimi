# UI consistency and feedback / 界面一致性与操作反馈

修改设置、系统托盘菜单或字幕浮窗前，先阅读此规范。修复重复的界面问题时，
必须排查其他页面的同类控件，同时检查成功、失败、弹窗和异步结果迟到的路径。

Read this before changing settings, the native tray menu or the subtitle overlay.
These project rules apply to implementation and review.

## Layout

- Start with the existing design tokens, components, icons and nearby screens.
  Keep the established black, white and neutral-gray visual language. Use
  status colors only for a concrete state; preserve existing provider branding.
- Name the actual function, state or action in plain language. Do not use slogans,
  promises or personified service names such as “translation partner”. Preview
  captions use ordinary sample text. Describe only capabilities available on
  that platform; omit unsupported features from product copy. Android playback
  capture still needs the system audio-recording permission, so explain that
  requirement where permission is requested.
- Make the title, current state, main action and detail areas easy to distinguish.
  Group related controls, use consistent button sizes and align actions right.
  Do not scatter buttons across large blank areas or add cards to every row.
- An empty configuration list needs a short, readable next-step instruction
  beside its add action. Show it only after a successful settings load; a zero
  count alone is not sufficient guidance for a fresh install.
- Keep essential labels, choices and actionable errors readable at normal text
  size. Put non-essential explanations behind hover/focus help icons. Long raw
  diagnostics and advanced details start collapsed; bound long lists and text.
- Verify wrapping, narrow windows, long labels, empty content and light/dark
  themes. Interactive controls, including help and toast dismissal, keep pointer
  cursors; preserve disabled, text-input, slider and resize cursors.
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
