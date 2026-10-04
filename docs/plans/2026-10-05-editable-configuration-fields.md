# Read and edit complete configuration values

Stored nonsecret configuration belongs in the editable value, never in placeholder
text. A placeholder is a short localized instruction for an empty field; a sample
URL or model must not masquerade as an effective saved/default configuration.
Intentional defaults (such as ChatMock's local endpoint) remain actual values.

All persistent nonsecret text fields use the shared ConfigInput's explicit inline
expand action: service/profile names, provider endpoints, models, deployments,
application IDs and global/profile proxy URLs. Expanded editing wraps long values
inside the existing field group and edits the same controlled draft. Collapsing
retains edits. It does not save, fetch or promote a placeholder to a value.
Preserve selection, validation focus, disabled/read-only states, max length and
single-line value semantics. Moving between the field and its expansion must not
trigger proxy auto-save.

Saved secret fields stay masked until explicitly revealed through their existing
local-only action. The expansion feature is opt-in and never follows the input's
`type`, since a revealed key also uses `type="text"`.

The sibling audit covers every product input. Settings contains persistent text
configuration; tray/overlay only share transient searchable pickers. Searches,
color controls, ranges, replay number fields and the read-only diagnostic report
keep their existing purpose-specific controls. Names intentionally left empty
continue to use their existing default label without storing it as a custom name.

Verify long saved values, empty fields, actual edits, collapse/re-expand, clipboard
and length limits, keyboard/IME input, rejected saves and group focus. Check
minimum/default widths in Chinese, English and Japanese, plus the signed native
settings app. Do not leave a UI-only fixture running as the user's normal app;
verify the launch mode and restore the ordinary idle development app after QA.
