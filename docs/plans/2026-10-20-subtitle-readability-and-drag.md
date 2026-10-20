# Subtitle readability and discoverable movement

## Scope

The user approved this first implementation after reviewing issues #237 and
#264 and Say It's display approach. Both reporters subsequently found Mimi's
existing drag handle; #264's latest feedback emphasizes larger, more legible
subtitles. Improve the existing desktop overlay rather than changing provider
or translation behavior.

This increment covers a discoverable top drag area, larger text and a faithful
settings preview. External position controls, independently configurable source
text sizes and new text animations remain separate follow-ups. Do not close
#264 as fully implemented by this increment.

## Interaction

Make available space in the ordinary overlay's top band draggable through the
existing native movement command and saved-frame state machine. Keep controls,
the language capsule, metadata interactions, resize edges and the subtitle
reading area independent. Preserve double-click collapse, compact mode,
keyboard activation, disabled/busy behavior and error recovery. Use the existing
localized movement label and neutral visual language to make the affordance
recognizable before a user happens to hover over the old short line.

Keep lock and immersive semantics, native nonactivating behavior, position
restoration and the macOS native cursor path. Do not implement another dragging
or position-persistence mechanism.

## Typography and geometry

Expand the desktop font range from 14–20 to 14–48, retaining default 16 and
existing saved values. Validate the upper range in the signed native preview
before acceptance. The reading target uses the selected size; bilingual source
text uses a stable 0.9 ratio. A smaller window must not silently reduce that
ratio further.

Settings preview and real captions share typography: font family, size,
line height, weight, color and immersive text shadow. Original-only mode uses
the full selected size. Preserve existing theme, subtitle colors and bounded
history behavior.

Update both native and browser minimum-height calculations and the shared
desktop layout fixtures. Account for larger glyph lines, one or two independent
audio sources, bilingual/single-language display and optional timestamps.
Reserve 22 px for microphone/source metadata and 15 px for the optional divider
between two live sources; toggling the divider must not change the minimum.
Retain the saved-frame compatibility floor where it is sufficient. Never
enlarge text without ensuring the smallest permitted window has reading space.

This is desktop-native presentation work. Android retains its existing native
12–24 sp range and shared business runtime; this change makes no Android visual
acceptance claim.

## Implementation and verification

1. Update desktop preference bounds, native/browser minimum-height rules and
   layout fixtures; verify persistence, invalid inputs and mode/source budgets.
2. Share frontend typography and extend the settings slider; verify matching
   preview/live styles and the stable source ratio.
3. Expand top-band movement and improve the visible affordance; verify gestures
   do not intercept sibling controls or subtitle history interactions.
4. Inspect the signed canonical `mimi-dev.app --watch --ui-only` preview with
   synthetic text. Cover sizes 16/20/24/32/40/48, narrow/default windows, long
   captions, original/translation/bilingual, single/dual input and immersive
   mode. Check actual native movement, collapse, lock and cursor behavior.
5. Review all eight desktop UI languages and affected settings/control/overlay
   siblings. Stop the preview before the complete canonical check, then perform
   a separate defect-and-claim review and record the exact evidence boundary.

Use no provider requests, audio capture, credential access or production app
replacement for this UI-only work. Browser layout and macOS native evidence do
not establish Windows DPI/multi-monitor or Linux-device acceptance.
