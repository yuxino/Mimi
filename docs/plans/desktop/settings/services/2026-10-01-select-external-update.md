# External settings picker updates

Refs #93, #88.

Linux native UI-only candidate db44d57 showed the bilingual preview and overlay
updating after a shortcut, while the already mounted settings picker retained
its translation-only label. Navigating away and back refreshed the label.
The source already passes the live settings snapshot to the picker; no extra
backend broadcast or persistent setting is needed.

Keep the trigger mounted to preserve focus and replace only its value-dependent
label node. This targets retained WebKit painting without rebuilding the whole
settings page. Native confirmation is required: jsdom already updates the old
closed label and cannot prove the native rendering symptom fixed.

Separately, a focused regression reproduces the open menu's keyboard cursor
staying on the old option after an external value change. Synchronize that
cursor with the selected option when the value changes; normal arrow/typeahead
navigation remains local while the value is unchanged.

Verify repeated external original/translation/bilingual changes, visible label,
preview, overlay, open-menu selection, keyboard commit and retained focus. Use
isolated UI-only data; no provider, audio capture, Keychain or history access.
