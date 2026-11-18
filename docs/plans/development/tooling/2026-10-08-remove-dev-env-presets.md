# Retire environment-backed development presets

The user requested one development app initialized from the current production
configuration, with independent data afterward. Production already uses editable
private local credential files, so the separate read-only `.env` path is obsolete.

- Remove the Cargo feature, parser, preset injection, read-only editor branches,
  diagnostic labels and obsolete setup instructions. Never read `.env` at runtime.
- Keep `/Applications/mimi-dev.app`, its development bundle identifier and pinned
  signing identity. Production and development retain separate app directories.
- Preserve saved legacy preset metadata as editable ordinary profiles. Do not
  import keys from `.env`; retain only metadata/legacy-import compatibility.
- Manual paid probes remain ignored and test-only under `development-debugger`.
  Require an existing completed local credential store and use the active profile.
- A user-requested initialization copies profile metadata, preferences and local
  credentials once, translating the credential service namespace. Preserve prior
  development user data; never alter the production source or create shared files.

Verify ordinary saved credential edits across restart, ignored malformed `.env`,
legacy-profile compatibility, the full automated check, development-feature
compilation, stable installed identity and native configuration availability.
No recording, capture or paid service request is needed for this change.
