# Overlay quick opacity slider and custom background/text colors

Tracks [yuxino/Mimi#432](https://github.com/yuxino/Mimi/issues/432) across both desktop and Android.

## Goals

1. **Quick transparency slider directly on the floating overlay** (desktop and Android):
   - Adjusts subtitle background opacity (`0%` transparent to `100%` solid) in place without opening the main settings window.
   - Sits unobtrusively along the left edge of the expanded subtitle overlay so it clears top/right/corner chrome and never squeezes caption text.
   - Hides together with drag/resize controls when the overlay is locked (desktop Lock position / Android Adjust captions OFF) and remains hidden in immersive mode.
   - Cancels in-progress drag gestures on lock or configuration change without persisting unfinished input.
2. **Custom overlay background color and HEX input for both text and background colors**:
   - Preserves pure black (`#000000`) at 80% opacity as the default background and existing text-color presets/defaults for all existing installations.
   - Desktop adds `subtitle_background_color` (`#RRGGBB`, default `#000000`) to `Preferences` and IPC snapshots/drafts, adds a background color setting row, and extends `SubtitleColorControl` with an inline `#RRGGBB` HEX input alongside swatches and the color picker.
   - Android adds custom `#RRGGBB` text color and overlay background color preferences in `SettingsStore`, a native color picker dialog with an interactive HSV color palette and synced `#RRGGBB` HEX input (`CaptionColorPickerDialog`), and background color controls in `AppearanceControls`.
   - Adapts overlay chrome contrast (controls, drag/resize marks, slider track, dividers, and secondary bilingual reference text) when a high-opacity light background color is chosen so controls remain legible over light surfaces.

## Invariants

- Existing saved preferences without `subtitle_background_color` or Android custom hex keys keep their exact prior appearance (black background, existing text color preset).
- Locking the Android overlay still keeps frame bounds, reading viewport insets (`48dp` top, `8dp` bottom, `16dp` horizontal), and caption pixels unchanged: the left-edge slider draws strictly inside the existing `16dp` left margin and toggles between `VISIBLE` and `INVISIBLE` (never `GONE`).
- Desktop's left-edge slider uses `.overlay-hover-control`, so it stays hidden while unhovered/quiet and is omitted when locked, immersive, or collapsed; it sits inside `.overlay-expanded-canvas`, clear of the outer `6px` left resize handle.
- Instant preference writes use each platform's existing persistence and failure reporting path (desktop coalesced slider queue + failure toast; Android live drag preview + durable commit on release with rollback/toast on write failure).
