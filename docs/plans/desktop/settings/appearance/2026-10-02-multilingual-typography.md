# Multilingual typography and narrow settings labels

The English Linux service screenshot shows two related defects: fixed 76/112 px
label columns wrap or clip ordinary field names, and the document still claims
Chinese even after the user switches the interface to English or Japanese.
Other desktop surfaces each declare a separate font stack; WebKitGTK can then
choose different Latin and CJK faces for the same UI and its native controls.

Use one platform-aware UI font stack across Settings, tray, subtitle controls
and overlay text. Keep the document language synchronized with the effective
interface language before first paint and after a live language change; order
the Linux Noto CJK fallbacks for that language. Native controls and portaled
menus inherit the same stack. Do not bundle a new font or alter user-selected
subtitle size and line budgeting.

For service details, use readable sentence-case labels and columns wide enough
for English and Japanese names. Stack label and control when the settings
window is narrow rather than squeezing a word into a small column. Keep long
error guidance wrapping within its container and show one primary credential
error when a failed save already provides the actionable detail.

Verify Chinese, English and Japanese at the default 760 px settings width and
the 520 px minimum, plus tray/overlay controls. Check the unavailable-store
state separately from a normal saved credential. Android uses native system
font fallback and a separate layout; inspect it for fixed-width text conflicts,
but do not claim device acceptance without an emulator or physical device.

On Android, the service-list action was fixed to 72 dp even in English and at
larger system font scales. Size it to its translated text with a 72 dp minimum;
leave the service name column flexible.

The tray has only 308 px of content width. Let its long labels wrap instead of
silently truncating them, and use short, translated display-mode names inside
the picker while keeping full names in Settings.
