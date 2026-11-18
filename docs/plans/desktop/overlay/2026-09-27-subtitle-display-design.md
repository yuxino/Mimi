# Subtitle display modes

Issue #48 asks to compare original speech with its translation. Add a persisted
presentation preference: translation (the existing default), bilingual, or
original. Keep this independent of the target language and provider session;
changing the display must neither reconnect nor clear current subtitles. The
existing Original target still disables translation and renders only one copy.

Settings, the overlay's existing control panel, and the tray expose the same
three choices. Settings preview follows the selection. Cmd+Shift+B on macOS and
Ctrl+Shift+B elsewhere cycles translation → bilingual → original. Use the shared
settings update path so other windows and native tray state follow immediately.
Registration failure must not prevent startup; on-screen controls remain usable.
All new copy is localized into English, Simplified Chinese and Japanese.

Committed bilingual subtitles use the source/translation pair already delivered
by the reducer. Current draft snapshots do not carry a shared utterance ID;
never pair a newer recognition draft with an older translated draft in durable
history. Issue #59 showed the gap in waiting for those pairs: the affected
Alibaba live-translate session confirmed only two pairs in forty seconds while
its translation draft streamed, so bilingual mode showed the original alone.
Bilingual mode therefore keeps the previous preview row set as the trailing
rows — recognized original first, then the streaming translation — and a
preview gives way to its committed pair the moment that pair exists. In
same-language mode, show one source preview even if the two streams differ
briefly. Keep draft tails bounded and preserve empty/paused/error
and immersive behavior. This changes presentation only, not opt-in archival
rules.

Validate preference migration and serialization, display selection and pairing,
missing/late translations, long subtitles, same-language behavior, multi-window
synchronization and shortcut switching. Run the canonical repository check and
inspect the signed native dev app. Record actual native controls in English at
1080p or higher; if using UI-only samples, label them explicitly. Keep the HD
original, trim waits, and annotate actual operations without obscuring subtitles.

Native review: give display mode its own full-width settings row, then separate
size and alignment rows. Keep the service lock notice clear of its toolbar and
list; use one subtle row hover, with an inset edit action and no disabled hover.
Keep recording assets local until the user authorizes the release reply.

Use a shared app-styled picker for every in-app dropdown, including existing
language, translation, interface-language and provider configuration options.
Keep popup colors in the existing neutral theme, render the menu above clipping
containers, constrain it to the window, and support keyboard selection, Escape,
Tab, type-ahead and outside dismissal. Native OS tray context menus remain native.
