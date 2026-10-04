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

Keep the existing dark overlay, shared typography and provider assets. The
metadata row reserves the centered drag target at normal widths. Below 552 px,
put metadata below that target, using 20 px of additional header height so both
timing values remain readable. Native and browser resize minima reserve this
20 px at every width (single-source minimum 156 px), so narrowing cannot clip
bilingual or timestamped dual-source text. Preserve the older saved-frame
validation floor while expanding its current minimum in place. Bound longer
names with single-line ellipsis, reserve readable text space for both stages and
keep logos from shrinking; reveal complete names in the tooltip. Never let a long
translation alias consume the recognition label. When reading history, retain
both provider icons beside Back to live. Keep the
subtitle origin stable when toggling immersive mode; hide service chrome in
immersive and collapsed presentations. Paused, locked and error states retain it.

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
