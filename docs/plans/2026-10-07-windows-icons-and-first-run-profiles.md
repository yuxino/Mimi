# Windows application icons and first-run service selection

- Windows application enumeration currently returns no icon data. Read each
  executable's local shell icon, convert it to a 32px PNG with Windows Imaging
  Component, and reuse the existing per-icon and snapshot byte budgets. Release
  native handles and COM resources on every path. An unreadable icon remains an
  optional neutral fallback; process identity and audio capture stay unchanged.
- A new installation starts with an empty service catalog and no active profile.
  Confirming the first added provider creates and selects that profile. An empty
  catalog is valid across restart and allows general preferences to save.
  Missing profiles report setup required, while existing credential storage
  failures retain their unavailable status. UI-only startup mirrors the empty
  first-run catalog without reading user data.
- Existing catalogs remain untouched. A pre-catalog installation with existing
  preferences retains its explicit legacy Alibaba migration path; development
  presets and UI fixtures stay separate from production first-run defaults.
- Verify fresh startup, restart, adding the first provider, existing/legacy
  catalogs, and failed persistence. Exercise Windows icon conversion in native
  Windows CI, including invalid paths and repeated extraction; browser fixtures
  do not establish Windows native behavior.
