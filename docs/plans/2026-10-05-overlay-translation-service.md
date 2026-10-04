# Floating subtitle translation service

The expanded subtitle window shows the selected translation service on the
right, using the existing provider mark and a short name. Hover/focus reveals
the profile and recognition/translation services; clicking opens Settings with
the overlay's existing guarded action and sanitized failure feedback.

Use credential-free settings metadata. Prefer the independent text-translation
route over the recognition provider, including legacy DeepLX profiles. Original
language mode and custom recognition without a translation route show the
localized Original label with a captions icon. A display-only Original preference
does not disable the actual translation route. Do not infer models or endpoints.

Keep the existing dark overlay, shared typography and provider assets. The
metadata row reserves the centered drag target at normal widths. Below 552 px,
put metadata below that target, using 20 px of additional header height so both
timing values remain readable. Native and browser resize minima reserve this
20 px at every width (single-source minimum 156 px), so narrowing cannot clip
bilingual or timestamped dual-source text. Preserve the older saved-frame
validation floor while expanding its current minimum in place. Bound longer names and reveal them in the tooltip.
When reading history, retain the provider icon beside Back to live. Keep the
subtitle origin stable when toggling immersive mode; hide service chrome in
immersive and collapsed presentations. Paused, locked and error states retain it.

Validation: focused route/action/state tests, the repository check, and
`scripts/verify-overlay-service-layout.js` exercise 72 browser combinations of
language, route and width, plus 18 minimum-height cases that check actual subtitle
glyphs and timestamps across single and dual inputs. The harness verifies that
the mounted UI received its synthetic state before checking layout, so stale Vite
modules after a rebase fail immediately. Native development verification and its limits belong
in the integration run ledger. No provider, capture, recording or credential
behavior changes are involved.
