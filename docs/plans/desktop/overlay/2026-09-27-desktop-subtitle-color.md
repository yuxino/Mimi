# Desktop subtitle color

Port the five Android subtitle color presets to desktop: white (default), teal,
yellow, green, and pink, using the same RGB values and display order.

Offer accessible preset swatches followed by a custom-color swatch in the same
row. The final swatch opens the system color picker; do not show a separate
custom-color row or a HEX field. Use a plus icon when a preset is selected and
the chosen color when it is custom. Colors matching a preset highlight only that
preset. Keep the picker keyboard-accessible with a localized name and visible
focus outline.

Store the selected preset or custom RGB color in Preferences through the existing
settings snapshot/draft contract. Preserve existing preset strings; serialize custom
colors as uppercase `#RRGGBB`. Validate at both the color-picker and native IPC
boundaries. Reject names, shorthand, alpha, and invalid hex. Missing preferences
default to white. Color
changes are presentation-only and remain available during an active session;
they do not change provider configuration or restart audio/translation.

Use the selected color for translation and original-only subtitles in both card
and immersive layouts. Bilingual source lines keep neutral white and their
existing opacity. Preserve draft and history fading, timestamps, and shadows.
The settings preview uses a neutral dark surface to keep all presets visible in
both application themes. Labels are localized in Chinese, English, and Japanese.

Verify legacy defaults, palette serialization, persistent reload without provider
changes, optimistic settings updates, and actual Timeline rendering in both
layouts. Run the canonical repository checks and the signed UI-only development
app before publishing.
