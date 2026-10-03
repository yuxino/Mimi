# Diagnostics layout

The diagnostics page uses the existing settings typography and neutral theme
tokens. It separates the human-readable snapshot from the report used for
support, so inspecting recent events does not require opening raw JSON.

- Session status, API round trip and translation duration form a three-column
  overview with labels above values. Unavailable measurements remain explicit;
  zero milliseconds is still a measurement. Narrow windows stack the overview.
- Refresh, GitHub feedback and copy use the same compact button treatment and
  align right. The privacy help icon shares this toolbar instead of occupying
  a separate line. Timing explanations remain accessible hover/focus help.
- The newest six allowlisted events remain in chronological order in a compact
  timeline. An empty journal has a visible empty state. Failure categories keep
  the existing localized labels and use the settings error color.
- Raw diagnostic data has an explicit chevron and is collapsed initially.
  Opening it does not change event visibility. Its selectable contents have a
  bounded height, and clipboard failure still opens the read-only manual-copy
  field automatically.

Report preparation, size validation, whitelist parsing, request serialization,
notifications and the reviewed GitHub destination retain their existing paths.
No capture, credential or provider behavior changes. English, Chinese and
Japanese receive the same raw-data and empty-event labels.

Verify the existing diagnostics regressions, the repository check and the
signed UI-only development app. Inspect light/dark themes, empty events,
populated events, errors, collapsed/expanded raw data and narrow layouts.
