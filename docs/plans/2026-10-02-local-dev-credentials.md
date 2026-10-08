# Read-only local development credentials (superseded)

This design was retired on 2026-10-08. Development now uses editable app-scoped
local credential files; see [current design](2026-10-08-remove-dev-env-presets.md)
and [development credentials](../development/local-dev-credentials.md). The text
below records the earlier design and is not a setup guide.

Repeated self-signed macOS builds can require API-key Keychain authorization
despite a stable designated requirement. The user explicitly requested a private
local development `.env` alternative; production credentials remain OS-backed.

The optional `local-dev-credentials` Cargo feature defaults off. Selection occurs
only on macOS, with the exact `app.yuxino.mimi.dev` identifier, outside UI-only
mode. All gates precede secret-file metadata access. The canonical dev launcher
enables the feature; the fixed path is `app_config_dir/.env`, including when
reopened through Finder or the Dock.

An existing file selects an immutable `SecretStore` snapshot instead of Keyring.
It supplies only `ALIBABA_API_KEY` for Alibaba profiles using the default MT route;
unsupported providers and independent MT destinations return missing. There is
no migration scan, process-environment fallback, or mixed file/Keychain routing.
Only NotFound selects the original Keyring store. Permission, parsing or I/O
failure stays in file mode and returns a fixed safe error label.

The reader checks regular type, current effective UID, exact 0600 permissions
and a 16 KiB cap. O_NOFOLLOW rejects symlink substitution; O_NONBLOCK prevents a
replacement FIFO from blocking open. Opened descriptor metadata must match the
approved inode/device, ownership, mode and size. Reads remain bounded if the
file grows. No shell evaluation, variable expansion, generic dotenv dependency,
or secret-bearing error text is used.

The file store is read-only. Save/delete/reveal reject before modifying caches
or catalog routes, so secrets cannot cross the reveal IPC. Metadata profile
deletion skips shared file operations; ordinary preferences remain writable.
Reopening is the only refresh mechanism, and existing Keychain items are untouched.

Focused synthetic tests cover gate-before-read, default-build/UI-only exclusion,
absent vs invalid file selection, strict parsing/size/UID/permissions/symlink
checks, unsupported routes, native configuration, rejected writes/reveal, and
metadata/JSON separation. Live acceptance must separately use the signed dev
bundle; automated tests do not read the user's file or call provider services.
