# Android settings and live caption controls

The user reported missing live display-mode controls, fixed expanded placement,
poor settings organization, missing provider artwork and an oversized immersive
exit. They explicitly requested width adjustment and a mobile settings redesign.

## Native UI

Use one primary activity with three visible sections: Captions, Services and
More. A separate Home/settings hierarchy made common actions hard to locate. Put
language selection, one live/sample preview, display mode and immersion in
Captions. Keep one preview visible while the native controls scroll independently, above
them in portrait and alongside them on a wide screen. Common style controls are
directly visible, without an accordion, in both Captions and the floating dialog.
Use the same native controls and never add a second preview or primary tab.
Color choices are direct swatches with a selected ring; wrap on narrow surfaces
to preserve 48dp touch targets. The floating style dialog avoids a large redundant
title and uses its own bounded workspace; native checks must keep the rendered
source and translation rows visible at large fonts in landscape.
Keep service selection/configuration and its ongoing session lock in Services;
interface language, diagnostics and the setup guide in More. Keep the state and
Start/Stop footer visible in every section, use one row when actual localized
labels fit and stack otherwise, preserve each section's scroll
position, and restore the selected section after locale recreation. Android Back
returns to Captions from another section. Preserve existing neutral tokens and
brand artwork. Add provider marks to Captions and the expanded overlay as well
as keeping those in service editors and pickers.

One native appearance component owns display mode, width, font size, color,
background opacity and caption opacity. Both the Captions page and floating
panel use it. Preferences update existing service views immediately, with
sanitized failure feedback and no session restart.
Original/translation/bilingual are presentation choices; they never change
recognition, provider requests, shared subtitle state or retained paired
history. Recognition configured without translation continues to display
original text. Existing Android bilingual presentation remains the default.

Expanded captions have a dedicated drag handle, separate from scrolling and
buttons. Compact and expanded positions are saved separately in dp; safe-frame
clamps never overwrite saved placement. Width is adjustable in settings and the
floating controls, with a direct resize handle in the expanded panel. Keep
usable minimum widths and fit long controls by stacking them. Immersion centers
the text, uses a medium-weight translation at the chosen size, a smaller source
and a tight contrast shadow. Preview and overlay share this typography. The 24dp
visual exit remains inside a 48dp touch target adjacent to captions. Returning
restores the normal reading typography and placement.

While a session is running, show the service-edit restriction beside a visible
Stop action. Opening settings never silently stops playback/capture. Appearance
continues to be editable. Update the guard when the session ends.

## User flows

On returning to Mimi, live captions appear in the single Captions preview
instead of covering its controls. Floating windows temporarily hide above Mimi
activities while capture, provider sessions and paired state continue; leaving
Mimi restores the same floating views and reading position. Activity transitions
cannot flash a caption over credential fields. Keep sample content labeled when
idle. Before speech and during a silent gap, the running external overlay shows
the desktop's localized listening-empty text. It remains reachable for controls;
fresh subtitles replace the waiting text. Capture failures and provider recovery
retain their actionable feedback, and Stop removes the floating window. Caption
previews follow one latest line per language in a stable viewport. Reserve the
supported scripts' maximum font metrics so width, size and live-text changes
cannot move a slider under the user's finger. Full captions remain in the
reading panel; compact/immersive overlays keep their two/three-line limits. Keep
Start/Stop pinned while recovery instructions scroll with Caption content.

The expanded panel uses one top row for collapse, drag/language and appearance,
and one bottom row for immersion, provider/model details and resize. Keep its
immersion action visibly labeled in the interface language; a caption icon alone
does not explain the mode. Use a simple icon and text action, with pressed/focus
feedback rather than a persistent circular background. The collapse chevron points upwards. At narrow widths
or large fonts, put immersion on its own footer row, with provider marks and
resize below. Keep every action at least 48dp. Optional model text can ellipsize,
with its full label accessible via Captions. Keep the scroll viewport readable
in short landscape windows. At
normal fonts the panel uses at most 72% of the safe height; large-font landscape
uses at most 88%. These limits are checked together with the actual reading
viewport, rather than retaining an old height cap that leaves controls but no
caption. The small immersion restore target remains reachable after rotation and
font/content changes.

Streaming updates refresh the Caption preview only and coalesce pending preview
work. Stable help bindings survive repeated health/status updates.
Help popups dismiss when their native window loses focus, when a dialog opens,
and when the selected section changes; an old keyboard focus cannot show an
explanation over a different window or section. Native secret
fields explicitly restore password masking after single-line configuration; test
typing and both eye states using unsaved synthetic drafts.

Acceptance covers first setup, choosing an already configured service, switching
provider while active, viewing the current caption in Captions, changing display
and appearance while another app plays, moving/resizing away from video
controls, entering/leaving immersion and stopping after a capture warning.
Product actions must remain reachable at large system fonts and in landscape. Do
not interpret successful persistence, host geometry or a synthetic fixture as
real playback acceptance or a smoothness measurement.

## Verification and delivery

Add native interaction acceptance for display filtering, saved width/live view
geometry, expanded dragging/resizing, compact anchor preservation, service lock
and stop, and immersive exit. Review all eight locales, large fonts, narrow and
landscape bounds, light/dark themes, all three sections and editors. Synthetic
native checks do not replace real capture/provider evidence.

Use a separate stable Android debug app ID for development acceptance,
preserving the installed release app and its encrypted credentials. CI packages
the debug app and instrumentation APK together; shared Rust/JNI and APK
verification remain mandatory. Run the canonical repository check and Android CI
before presenting a reviewable PR. This mobile redesign needs user confirmation
before merging main; it does not replace already-published v1.5.23 assets.
