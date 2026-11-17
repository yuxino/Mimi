# Floating subtitle translation service

The expanded subtitle window shows its actual recognition and translation
services on the right, using existing provider marks and short names. Independent
services appear in recognition-to-translation order with a compact arrow; a
built-in route following its recognition service appears once. Hover/focus reveals
the profile and recognition/translation services; clicking opens Settings with
the overlay's existing guarded action and sanitized failure feedback.

Use credential-free settings metadata. Preserve the recognition identity beside
the independent text-translation route and its custom name. Legacy DeepLX
profiles use Alibaba recognition plus DeepLX translation. Original language mode
and custom recognition without a translation route show only the actual speech
service; the tooltip identifies translation as Original only. A display-only
Original preference does not disable the actual translation route. Matching
names or compatible protocols do not prove independently configured services
are the same provider. Do not infer models or endpoints.

Keep the existing dark overlay, shared typography and provider assets.
Use 580 × 240 logical pixels for the initial expanded frame. Honor a saved
frame and the existing minimum height for the configured subtitle layout.
The language capsule and action strip share the first row; metadata uses the
second. At narrow widths, omit optional actions before they overlap the capsule.
Retain the width-independent native and browser resize minima, including their
service-row allowance, so narrowing cannot clip bilingual or timestamped
dual-source text. Preserve the older saved-frame
validation floor while expanding its current minimum in place. Bound longer
names with single-line ellipsis, reserve readable text space for both stages and
keep logos from shrinking; reveal complete names in the tooltip. Never let a long
translation alias consume the recognition label. When reading history, retain
both provider icons beside Back to live. Keep the
subtitle origin stable when toggling immersive mode; hide service chrome in
immersive and collapsed presentations.

On the regular expanded canvas, mouse-capable windows show the action strip,
service names, measured timings and the language capsule on hover or
keyboard focus. Keep the listening wave, text, captions, translation
progress/recovery and Back to live visible. The capsule stays visible for
recording feedback and when required to unlock the overlay. Pin controls
during errors and while the control panel is open. Devices without fine hover
keep the controls visible. With captions present, release the hidden toolbar's
space. A visible status, action failure or Back to live needs only one short
row. Keep the same timeline and bottom edge so live following and history
anchors survive the viewport change. Concealed controls retain keyboard access
and do not take pointer input. The idle listening indicator retains its layout.
The language capsule and action strip align on the first row. A slimmer
metadata row holds service and measured timing. Empty chrome moves the window
without a visible handle; fold and expand through the explicit buttons.
Keep settings for audio input, speech translation and subtitles grouped in the
language panel, with a full-width configuration name and a separate Settings
action. Open pickers use an opaque surface so the labels and switches below
don't compete with the choices.

Subtitle preference switches use the whole row as their native button target,
including the label and blank space. Help remains an independent focusable
control and never changes the preference. Keep the same disabled save guard
for pointer and keyboard activation. Long language labels wrap within their
text column while the help stays beside the text, rather than on an isolated row.

Detached surfaces exchange exposure changes through targeted Tauri events and
retain their own hit testing and native geometry. Local DOM and macOS pointer
evidence are independent. Focus loss updates focus-based exposure without
treating a stationary pointer as departed. A brief hold on real departure
covers crossing between the neighboring native views; leaving both surfaces
clears the chrome.

Validation: focused route/action/state tests, the repository check, and
`scripts/verify-overlay-service-layout.js` exercise browser combinations of three
languages, both system color schemes, built-in/independent/custom speech routes,
long aliases and four widths, plus minimum-height cases that check actual subtitle
glyphs and timestamps across system, microphone and dual inputs. The harness
imports the actual module URLs already loaded by the page, including Vite's
version queries, and verifies that the mounted UI received its synthetic state
before checking layout. A query-free import can create a second Zustand or
language store even after a full page reload. Refuse native-bridge pages and
keep these fixtures entirely in the browser's memory. Native development verification and its limits belong
in the integration run ledger. No provider, capture, recording or credential
behavior changes are involved.
