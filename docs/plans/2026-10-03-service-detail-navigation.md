# Service detail navigation and credential mode

Profile details begin with a normal-size bordered back button beside the profile
identity. Audio input is global session configuration and belongs on the service
overview; it does not precede profile-specific recognition and translation forms.
Returning to the overview restores those input controls. Provider selection also
stays separate from global input configuration.

The local development file supplies only the built-in `alibaba-local-dev`
configuration, displayed as Alibaba Cloud · dev. That configuration keeps its
key read-only and always uses the native Alibaba recognition/translation chain.
It cannot reveal, change, delete, migrate, copy or fall back from that key to
Keychain. Removing the private file removes the preset on the next startup;
ordinary profiles and their stored credentials remain intact.

Other development configurations use the existing development-specific OS
credential service, just as normal profiles do without a local file. Recognition
providers and independent DeepL, DeepLX, ChatMock and compatible translation
destinations stay editable and persist normally. File validation errors affect
only the built-in preset. Neither the file nor the application decides a secret
source by provider alone: each configuration has one authoritative source.

Snapshots expose per-profile credential storage as non-secret metadata. The
built-in preset does not consume one of the 20 user profile slots, cannot be
deleted, and has a static provider identity rather than a disabled selector.
Keep the default configuration distinct from editable profiles in the list and
detail view. Retain save/cancel, separate connection checks and session locking.

This exception remains gated by the opt-in macOS feature, exact development
identifier and non-UI-only execution. Production credentials, provider protocols
and Android behavior remain unchanged. Verify navigation, persistence, preset
isolation and file failures with synthetic frontend/native tests, then inspect
the signed macOS development bundle.
