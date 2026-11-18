# Custom recognition display name

Custom speech profiles can have an independent recognition display name, such
as `Whisper`, without renaming the whole recognition/translation configuration.
The settings editor uses the existing automatic name field, including draft/IME
preservation, serialized saves, inline errors and retry. Empty input restores
the localized protocol name. Built-in recognition labels remain unchanged.

`speechRecognitionName` is optional, nonsecret profile metadata. A dedicated
metadata patch trims surrounding whitespace, accepts up to 64 characters and
rejects control characters. Omission preserves the current name; an empty
string clears it. Old catalogs need no migration. Patches retain the current
configuration name, translation aliases, proxies and credentials. Read-only
development profiles and active-session editing keep their existing boundaries.

The subtitle service label, its complete tooltip and the recognition summary
in the configuration list use the same display-name resolver. Protocol icons
and the protocol description in the editor retain their actual identities.
The tray's current-configuration chip continues to identify the whole profile.
Long names retain the existing truncation and tooltip behavior; identically
named recognition and translation endpoints remain two independent stages.

This is desktop presentation metadata, with no change to recognition,
translation contracts, capture, or the shared subtitle reducer. Android does
not expose this desktop alias editor. Verification covers legacy/blank fallback,
both custom protocols, persistence, metadata isolation, failed saves and UI
rendering in the signed credential-free development build.
