# Service settings: recognition and translation

A service profile describes a complete subtitle pipeline. Speech recognition and text translation must appear as distinct stages rather than treating text translators as audio providers.

Alibaba profiles (including historical DeepLX profiles) show always-visible **Speech recognition** and **Text translation** sections. Recognition remains Alibaba Cloud. Text translation selects the built-in route or a supported independent destination, with the corresponding identity mark. The [provider setup guide](../provider-setup.md) owns the current destination list. Realtime providers with integrated audio translation show **Speech translation**, without an unsupported independent text selector.

Keep the existing profile model, credential storage scopes, save payload, connection checks, and active-session lock. The shared credential state describes the whole pipeline; explicit connection checks also report recognition and text translation independently. Removing credentials still removes the profile's recognition and translation credentials together. No service is added and no wire protocol changes.

Connection, update, remove, cancel, and save controls align to the right with consistent compact sizing and existing action icons. Storage and compatibility explanations move to small hover/focus help controls. Tooltip descriptions stay available to assistive technology; validation errors remain visible next to the affected input.

Use local documented brand assets. The legacy DeepLX mark is the SVG published by its documentation site; generic OpenAI-compatible destinations use a neutral language icon.

Verify saved/missing profiles, legacy routes, keyboard selection, draft clearing and failed-save retention, active-session locking, tooltip dismissal and viewport positioning, and light/dark native settings. Run the canonical repository check and signed UI-only development app.


## UI revision

Keep secondary descriptions out of the page's small-print rows. Page headers and shared settings rows combine their secondary description and hint into one compact hover/focus help control. Service type requirements, inactive-profile language guidance, storage notes, and proxy scope/limitations use the same controls. Provider choices use one name and logo; custom recognition choices appear last, with protocol details in help. Errors, confirmation messages, and explicit check outcomes use normal body text.

Settings has no header session switch or session-status row; session controls remain in the tray. Desktop-managed Linux shortcut commands remain available from General in a normal dialog with the three exact read-only commands. Adding a service first previews its identity in a standard modal and only creates a profile after explicit confirmation. Cancellation leaves the catalog unchanged. Profile, credential, and saved-history deletion use the same modal with a red confirmation action, keyboard focus containment, Escape cancellation, and focus restoration.

Recognition and text translation have independent explicit checks. Complete validated editor drafts can be checked before saving; incomplete drafts keep the check disabled. Follow the [draft-check contract](2026-10-05-draft-connection-checks.md). Results are scoped to stage/profile/route and invalidate when the matching configuration changes. Show measured check duration only for real requests: recognition setup and a fixed short text request are different measurements, neither is end-to-end subtitle latency. UI preview mode never contacts providers and explains why a connection check did not run.
