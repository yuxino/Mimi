# Shared language menus

The subtitle panel and tray still filtered the active service's language catalog
to five old shortcuts, while settings exposed the full supported range. The
user requested consistent choices and one maintenance path.

All three surfaces now consume `sourceLanguagesForSettings`. Remove the shortcut
filter rather than maintaining another copied language array. This preserves
the backend's profile/provider/translation-route/target-stamped capabilities and
the existing fallback for older snapshots. Alibaba with Lite offers 24 source
languages plus automatic; Original offers 30 plus automatic. DeepL, custom
translation and other speech providers keep their actual independent ranges.

`LanguageSelect` shares filtering and localized search/empty-state behavior for
large catalogs. The existing bounded Select popup scrolls inside the available
native window rather than expanding the panel to fit every language. Small
settings catalogs retain direct choices; one-source services retain the existing
summary/disabled presentation. Every supported source remains selectable in
each surface where a source choice exists.

All pickers use explicit source selection: choosing Chinese preserves the
selected target, and leaving Chinese with Original selected keeps Original.
Remove the old Chinese shortcut's implicit target rewriting; otherwise the six
ASR-only languages can appear in the full Original catalog but be silently
rejected after the shortcut changes the target to Chinese translation. Providers
without an Original mode still normalize unsupported or same-language targets
as before. Source labels are shared with settings, without a misleading Chinese
Original label.

The repair does not add targets to the floating panel or change translation
routing, model parameters or credentials. Active-session switching and settings
broadcasts continue to use the existing lifecycle path.

Verification covers full source menus, supported extended-language selection,
search and keyboard behavior, Chinese/English/Japanese UI copy, native capability
stamps, Original versus translated routes and narrower provider boundaries.
Run the canonical check and inspect the signed development app's settings,
subtitle control panel and tray; fixture-only verification does not claim
additional language recognition quality or make provider requests.

## Acceptance

- Complete `scripts/check.sh`: 800 Rust tests passed, one ignored; 721 frontend
  tests passed across 76 files, with formatting, strict clippy, typecheck, lint,
  build and diff checks. The removed legacy shortcut tests are replaced by
  explicit-target and ASR-only Original-route coverage.
- Signed canonical `/Applications/mimi-dev.app` UI-only inspection: settings
  search for `fr` selected French; the subtitle panel reflected it. The short
  panel's full translated-source menu reached Hungarian at the end, scrolled
  within its own bounds, and searching/selecting `de` changed the source to
  German while retaining the Chinese target. Settings reflected that change.
- Chinese with Original selected switched through the floating menu's `no`
  search to Norwegian, and the capsule confirmed Norwegian / Original. The
  selection was no longer silently rejected or rewritten to Chinese translation.
- Settings/panel native inspection and three-locale DOM regressions are distinct
  evidence. Tray full-list/search/selection regressions passed in DOM tests;
  the computer-use surface could not expose the system tray for pixel acceptance.
  No claim of physical tray acceptance is made for this change.
- UI-only mode used isolated in-memory preferences, no provider network, API-key
  read or system-audio capture. It exited before ordinary development launch;
  user preferences and the formal installation remain separate. Publication
  remains held under the user's earlier instruction.
