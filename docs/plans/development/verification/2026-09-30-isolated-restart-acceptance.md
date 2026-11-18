# Isolated restart acceptance

Normal UI-only runs now start with empty in-memory settings, without inspecting the installed user's preferences or profile catalog. Production loading and credential storage are unchanged.

Restart acceptance explicitly sets `MIMI_UI_TEST=1` and `MIMI_UI_TEST_PREFERENCES_DIR` to an existing private temporary directory whose basename starts with `mimi-ui-test-`. Only bounded non-secret `preferences.json` persists. Profiles and synthetic credentials remain in memory; the OS credential store and provider networks remain disabled. Symlinks, shared Unix directory permissions, oversized files and malformed JSON fail closed.

This verifies preference persistence with synthetic configuration. It does not establish real credential continuity or real provider authorization across upgrades. The UI-only setting never becomes a credential fallback.
