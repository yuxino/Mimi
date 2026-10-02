# Service settings: recognition and translation

A service profile describes a complete subtitle pipeline. Speech recognition and text translation must appear as distinct stages rather than treating text translators as audio providers.

Alibaba profiles (including historical DeepLX profiles) show always-visible **Speech recognition** and **Text translation** sections. Recognition remains Alibaba Cloud. Text translation selects Alibaba Cloud, DeepL, DeepLX, or an OpenAI-compatible Chat Completions destination, with the corresponding identity mark. Realtime providers with integrated audio translation show **Speech translation**, without an unsupported independent text selector.

Keep the existing profile model, credential storage scopes, save payload, connection checks, and active-session lock. The shared credential state and connection test describe the whole pipeline. Removing credentials still removes the profile's recognition and translation credentials together. No service is added and no wire protocol changes.

The private read-only development file mode shows both identities and a disabled translation selector, without mounting password, reveal, save, or credential removal controls. It never falls back to Keychain or extends the file format.

Connection, update, remove, cancel, and save controls align to the right with consistent compact sizing and existing action icons. Storage and compatibility explanations move to small hover/focus help controls. Tooltip descriptions stay available to assistive technology; validation errors remain visible next to the affected input.

Use local documented brand assets. The legacy DeepLX mark is the SVG published by its documentation site; generic OpenAI-compatible destinations use a neutral language icon.

Verify saved/missing/readonly profiles, legacy routes, keyboard selection, draft clearing and failed-save retention, active-session locking, tooltip dismissal and viewport positioning, and light/dark native settings. Run the canonical repository check and signed UI-only development app.
