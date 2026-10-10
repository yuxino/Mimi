# UI consistency and feedback / 界面一致性与操作反馈

修改设置、托盘面板或字幕浮窗前，先阅读此规范。修复重复的界面问题时，
必须排查其他页面的同类控件，同时检查成功、失败、弹窗和异步结果迟到的路径。

Read this before changing settings, the tray panel or the subtitle overlay.
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

1. Find sibling instances across settings categories, tray panel and overlay.
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

Android immersive mode has a Home entry/restore action backed by the same
preference as Settings and the floating panel. Change the existing caption
window in place, retaining the compact dp anchor and previous expanded reading
state. Temporary viewport/inset clamps must not rewrite saved placement. Keep
the restore control adjacent to captions, with a 48dp touch target and localized
width; caption touch-through must not make restoration unreachable. Test repeated
mode notifications, rotation, content/font reflow and returning from both compact
and expanded states. Host geometry tests do not establish native window or touch
acceptance.
