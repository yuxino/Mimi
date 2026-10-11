# Desktop settings actions

Make repeated service setup easier without adding a second configuration flow.

- Duplicate a saved profile from its editor, then open the independent copy.
  Keep the current active profile and session preferences unchanged. Copy names,
  model, language preset, custom language declarations, proxies, and both private
  credential slots natively. No credential payload crosses IPC or the clipboard.
  Explain this next to the action with the existing contextual help.
- Generate a localized, unique copy name within the 64-character limit. Enforce
  the existing 20-profile limit and stopped-session mutation guard on both sides.
  Validate before writing; if private storage or catalog persistence fails,
  remove only the new copy's slots and preserve the original configuration.
- Add explicitly enabled copy actions to nonsecret name, endpoint, model and
  proxy fields. Copy the visible value, including an unsaved draft; empty fields
  cannot copy their placeholder. Secret inputs never opt in, including when
  revealed. Use native clipboard access and the shared transient toast.
- Put Duplicate and Use near the configuration heading. Keep deletion at the
  end and retain its existing confirmation. Give list editing a visible label.
- Keep existing independent connection checks, appearance preview, tray and
  overlay settings entry points; avoid duplicating controls already present.

Verification: focused native-store transaction tests, field/UI regressions,
the repository check, real-component browser geometry in eight languages and
both themes, plus signed `/Applications/mimi-dev.app` checks in Chinese,
English and Japanese at normal/minimum widths. UI-only checks prove local UI
and synthetic configuration behavior, not provider connectivity or capture.
Native minimum-size QA uses `MIMI_UI_TEST_COMPACT_SETTINGS=1` with
`./scripts/dev-app.sh --ui-only`. Live launches explicitly reset the fixture.
