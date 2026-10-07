# Android overlay interaction repair

The native compact overlay currently becomes visible before any displayed line
exists, leaving only its padded background. The font action silently cycles
sizes, and settings sliders update preferences without refreshing the service.
These are rendering and interaction defects; shared subtitle/provider rules stay
in their existing owners.

Hide the compact window unless a visible, nonblank caption or actionable capture
status exists. Expanded reading controls remain available while the panel is
open, and immersive exit remains available during silence. Do not substitute
source text for a missing translation or discard confirmed history.

Replace the ambiguous Aa action with a localized size label and current value.
Open a native size slider with the current value and apply changes immediately.
Observe appearance preferences while the overlay exists, refreshing font, color,
opacity, background, history retention and position in place. Mode changes may
rebuild native views, preserving session ownership and bounded subtitle state.
Unregister observers and dismiss dialogs when the service ends.

Settings tabs stack at large system font scales. Overlay actions reflow into a
third row when even two translated actions exceed the available width; check
native bounds rather than assuming two rows are sufficient.

Service editors use visible wrapping labels at large system font scales, with
the same shared field wrapper for speech and text translation. Header and save
action height follow their text. Provider-help links stay in scrollable content
instead of a fixed-height multi-button dialog footer. This preserves complete
labels and reachable actions without reducing the requested font scale.

Use a blank emulator and the same synthetic captions for Before/After captures:
empty initial state, font control, settings slider and compact/expanded captions.
Assert actual native text size, visibility, current numeric label and history
preservation, then exercise long captions, silence, immersive exit, narrow screens,
large fonts and seven interface languages. These UI fixtures do not establish
real-provider recognition or translation quality.

Optional explanations live beside their owning title/control in a help icon.
Native pointer hover and long press reveal the explanation; tapping opens the
same readable scrollable help, including for touch and accessibility users.
Keep one help dialog per activity. Home introduction/privacy details, ordinary
ready/running hints, appearance hints, language-save rules and guide storage/
capture-limit details follow this rule. Keep setup, permission, silent-capture
and storage recovery visible at normal size, along with first-run audio-sending
and service-charge disclosures. Verify native hover and tap separately, all
seven languages, narrow/large-font layouts and comparable Before/After captures.

The Android framework tooltip clips multiline explanations to three lines.
Shared help uses a neutral scrollable native popup, bounded by the available
space above/below its anchor. Hover can move into the popup for scrolling;
focus, long press and tap keep help reachable without a mouse. Dismiss on
pointer/focus exit, navigation and host destruction. Do not classify a stored
full tooltip string as proof that the explanation is visibly complete.
