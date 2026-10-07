# Configuration menu details and direct navigation

Saved recognition/translation combinations appear on a second line below the
configuration name in the tray and floating-control menus. The compact closed
picker shows the name and provider icon. Reuse the shared Select option's
optional description; keep names readable without wrapping short CJK names into
single-character columns. Detailed menus reserve a wider bounded viewport and
scroll using actual row geometry. Search/typeahead and selected checks retain
configuration identity, not the descriptive text.

The subtitle header's service button opens the active configuration detail,
including its recognition and translation settings, through the existing native
settings navigation channel. A distinct `activeProfile` intent preserves ordinary
service-list and Apple-resource navigation. Repeated requests resolve the current
active profile even if Settings last showed another editor or the provider picker.
Initialization and pending form operations defer the request. A deferred save
failure keeps its draft and retry visible instead of navigating away; a new
explicit request can still leave that editor. Navigation does not
select, create or edit a profile, reveal credentials, download resources or restart
capture. Existing session edit restrictions remain in effect. Focus lands on the
detail heading after navigation.
