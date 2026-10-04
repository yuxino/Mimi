# Installed subtitle font selection

Add a desktop subtitle font preference beside the existing subtitle display
choice. The user confirmed a searchable list of fonts installed on the computer,
with a **System default** choice. Preserve the existing neutral settings design,
shared picker, help tooltip and quiet preference saving with failure toasts.

The family choice applies immediately to original and translated subtitle text,
for system and microphone sources, in the standard and immersive floating
window, settings preview and development replay. Metadata, controls, labels and
menus retain the interface font. Existing size, alignment, colors, wrapping,
bounded live-tail display and full history reading continue independently.

Persist only one family name in ordinary preferences. Empty or missing values
select the existing language-aware interface fallback stack. Preserve saved
names when the font becomes unavailable; append the fallback stack for removed
fonts and unsupported characters. Quote and escape the selected name as one
literal CSS font family, including punctuation and generic-family keywords.
Reject control characters and names longer than 256 Unicode characters before
persistence; malformed legacy values render with the default stack.

Enumerate families locally on opening the picker, through the platform's font
catalog. Refresh on reopening so newly installed fonts can appear. Bound,
normalize and deduplicate the returned names; do not read font file contents,
download fonts, introduce a font service or expose local font paths. Keep the
list searchable and keyboard accessible. Loading or enumeration failure must
leave System default and the saved choice usable; late callbacks must respect
the settings window's existing toast lifetime boundaries.

This is desktop presentation scope. Android retains its existing native subtitle
size controls and does not gain an installed-family picker in this change. No
shared subtitle reducer, recognition or translation contract changes are needed.

Verify persistence and legacy defaults, literal CSS names, both subtitle lanes,
both floating presentations, unchanged metadata typography, picker search and
keyboard behavior, enumeration/save failure handling, and localized narrow-window
layout. Run the desktop repository checks and inspect the signed macOS UI-only
app. Keep macOS native font enumeration evidence distinct from Windows/Linux
build checks and unverified native font rendering on those platforms.
