# Subtitle font saves across page replacement

The subtitle settings page conditionally mounts `SubtitleFontControl`. Its old
component-local queue preserved a selected font after navigation, but a replacement
control owned a second queue. With A saving and B queued, leaving and returning to
select C produced native writes A, C, B. The older B replaced the user's newer C.
The failure reproduces through the actual store and settings-save coordinator;
only the native persistence boundary is synthetic.

## Ownership

- One module-scoped `SubtitleFontChoiceOwner` per WebView owns at most one
  in-flight save and one latest queued choice. New controls and simultaneous
  instances subscribe to that same pending value. A newer C replaces unsent B.
- Unmounting removes presentation subscriptions and clears that view's preview.
  It does not discard a valid queued B when no newer choice exists. The owner
  retains no DOM element or preview callback, and releases save/error callbacks
  when their bounded work finishes.
- Each choice keeps its existing save function and toast reservation. An old
  view cannot revive a notification after navigation; a failed current choice
  still reports the sanitized error and uses the store's existing rollback.
- A successful A satisfies an A→B→A return only when the latest live settings
  still contain A. The production picker supplies a required `getState()` read,
  including after unmount. An external D can invalidate the first A's command
  response; choosing A after D must then send A again. Failed A likewise does
  not satisfy the return. False/null save outcomes are failures.
- Pending local intent remains the displayed choice through old native echoes
  and other settings snapshots. Once the owner is idle, current settings again
  determine the displayed font. There is no mutation ID, so the owner does not
  infer whether an equal-valued external snapshot came from its own save.

This changes only the font control's submission owner. It does not change the
settings coordinator, ordinary transaction ordering, native persistence, font
enumeration, tray/overlay settings, or the two numeric slider preview rules.
Search found one production font-family picker, in subtitle settings; native
and overlay consumers continue to read the ordinary shared settings snapshot.

## Evidence and limits

Baseline `f949d610` and the new component use the same actual
`useStore.saveSettings`, snapshot listeners and `SettingsSaveCoordinator` in the
maintained ownership tests. Both successful and failed first-save negative
controls produce A, C, B with the old component; the new owner produces A, C.
Two additional old-component negative controls lose the latest A after an
external D invalidates A's old response, both while visible and after leaving.
Other cases cover B surviving navigation, concurrent instances, cross-remount
ABA, failed-current rollback/toast, native broadcasts/external values and
StrictMode preview restoration. Six direct owner cases cover queue bounds,
failure, ABA and reentrant subscriptions; the original 15 control cases remain.

Focused coverage is 31 passing tests. Quick frontend checks pass typecheck,
selected lint and 120 related tests in 10 files. Existing resume tests print
React act warnings but pass; assertions and timeouts were not changed. No real
provider, credential, audio or user-setting mutation was performed. Browser and
signed native interaction acceptance remain separate.

Root acceptance combined the unchanged font implementation with the bounded
resize dispatcher and rebased onto `90fa8cb` without changing the tested tree.
Complete frontend validation passed 2,333 tests in 144 files, lint, the
typechecked production build and all 17 output checks. A real Chromium
SettingsView preview navigated away and back during A/B/C saves: both successful
and failed first A wrote only A,C; leaving without a newer C retained A,B. The
actual control/store/navigation use synthetic native font and persistence
boundaries. Formal store listeners cover old echoes and external ABA separately;
no installed-device font, packaging or signed native acceptance is claimed.
