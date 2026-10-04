# Platform-specific settings layout regression

Refs: https://github.com/yuxino/mimi/issues/132

The Windows 10 IoT LTSC screenshots show an output label squeezed into four
one-character lines and a picker/status sentence misaligned below application
capture. `WindowsAudioSource` alone nested both pieces inside an inline span;
the shared row used an unbounded `auto` control track. Earlier multilingual
label work (`dc2143f`) did not exercise this platform-only path.

Keep picker rows on the shared bounded grid, reserve a readable label minimum,
and stack both application and output rows at the existing narrow breakpoint.
`SettingsRow.feedback` places ongoing state across the full row below its
controls. Idle instructions move to existing help; missing outputs, failed
enumeration and active sound/data states remain visible. Capture locking,
selection persistence and safe save-failure toasts keep their existing behavior.
Keep 16 px between directly adjacent setting rows without changing separator
spacing. The first repaired preview still had touching picker borders; label
width and no-overflow assertions must be accompanied by screenshot review.
The next screenshot review exposed detached storage help below the profile
list. Anchor generic help to the configuration count, service/provider names,
credential labels, proxy labels and applicable actions. Diagnostic privacy
explanation joins the page-heading help. Reuse labeled credential storage help
across the three editors and preserve their input description IDs; do not change
credential access or persistence behavior. Remove the obsolete footer styles.
Saved-value controls also need visible localized action text: their eye icon
alone did not explain the difference between a stored key and a replacement
draft. Keep the action on the field-label or stage-heading line. Preserve the
existing explicit read, cancel, hide, blur and navigation clearing behavior.

Windows uses endpoint-specific WASAPI loopback, so it exposes output selection.
macOS ScreenCaptureKit captures the system/application mix independently of an
output endpoint. Native `windows_audio_status` returns `None` off Windows;
the absent macOS picker is intentional, not a layout defect.

The reported serif Chinese glyphs require inspecting fonts on that Windows
image. Improve the shared sans stack by ordering Chinese families before
Japanese ones outside Japanese UI, including Windows non-UI family names and
available Noto alternatives. Shared tooltips now use the same token. No font
download, new dependency or change to subtitle font choice is needed.

Regression verification uses the real settings components with a synthetic,
credential-free bridge in `scripts/fixtures/settings-layout.html`.
`scripts/verify-settings-layout.js` checks frame/row overflow, label/control
overlap, squeezed labels, feedback placement and platform capability visibility
across five widths, three languages, two themes and device states, including
all six settings categories, three service editors and provider selection /
confirmation and synthetic saved-value previews (690 cases). It also rejects
detached help footers and verifies the gap between contextual help and its label. Run it with
an ego-browser managed Page while `npm run dev` serves this checkout. These
fixtures verify geometry, not Windows capture or installed Windows glyphs.
Also inspect the signed macOS UI-only app, the sibling settings rows, tray and
subtitle controls. Keep #132 open until the affected Windows release is retested.
