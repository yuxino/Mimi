# Desktop settings actions

Make repeated service setup easier without adding a second configuration flow.

- Duplicate a saved profile from its list row or editor, then open the independent copy.
  Keep the current active profile and session preferences unchanged. Copy names,
  model, language preset, custom language declarations, proxies, and both private
  credential slots natively. No credential payload crosses IPC or the clipboard.
  Keep the copy action plain; it does not need a separate help paragraph.
- Generate a localized, unique copy name within the 64-character limit. Enforce
  the existing 20-profile limit and stopped-session mutation guard on both sides.
  Validate before writing; if private storage or catalog persistence fails,
  remove only the new copy's slots and preserve the original configuration.
- Add explicitly enabled copy actions to nonsecret name, endpoint, model and
  proxy fields. Copy the visible value, including an unsaved draft; empty fields
  cannot copy their placeholder. Secret inputs never opt in, including when
  revealed. Use native clipboard access and the shared transient toast.
- Expose Use, Edit, Copy and Delete on every list row with short visible labels.
  Mark the current configuration In Use independently of the Use button, which
  can reapply its saved languages after a temporary change. Unusable profiles
  cannot be applied.
  Keep the identity area informational instead of switching on a row click.
  At narrow widths, wrap the actions below the identity without clipping names.
  Reuse the same handlers in the editor. Retain deletion confirmation and the
  last-profile guard, and target the requested row even if another profile was
  previously opened. Copies open for editing without becoming active.
- Keep existing independent connection checks, appearance preview, tray and
  overlay settings entry points; avoid duplicating controls already present.

Verification: focused native-store transaction tests, field/UI regressions,
the repository check, real-component browser geometry in eight languages and
both themes, plus signed `/Applications/mimi-dev.app` checks in Chinese,
English and Japanese at normal/minimum widths. UI-only checks prove local UI
and synthetic configuration behavior, not provider connectivity or capture.
Native minimum-size QA uses `MIMI_UI_TEST_COMPACT_SETTINGS=1` with
`./scripts/dev-app.sh --ui-only`. Live launches explicitly reset the fixture.
