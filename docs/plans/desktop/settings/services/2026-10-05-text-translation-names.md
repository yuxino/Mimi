# Independent text translation names

Allow each desktop profile's independent translation route to keep an optional
64-character display name. Store the names as nonsecret profile metadata keyed
by route, separate from credentials. Blank names remove the override. Historical
profiles retain localized provider labels, and switching routes never borrows a
name from another route.

The translation editor saves its name independently, including before credentials
exist. Explicit Save/Enter provides a transient success toast; failed saves keep
the draft and sanitized inline retry feedback. IME composition does not submit.
The settings list, route picker and subtitle service label share one display
helper; provider icons and transport identity remain unchanged. Original-only
subtitles retain their Original label. The tray still identifies whole profiles.

This is desktop display metadata, with no change to shared subtitle/provider wire
contracts. Android retains its existing service labels. Verify old snapshots,
per-route/profile isolation, clearing, read-only presets, save rejection, Unicode
bounds and persistence without credential access, then canonical checks and signed
macOS UI-only inspection.
