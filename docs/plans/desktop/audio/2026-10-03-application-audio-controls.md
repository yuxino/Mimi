# Application audio controls

## Problem and accepted behavior

Settings invokes the live audio-input command, but its Tauri permission group omitted that command. Both switches therefore looked usable while their changes were rejected. Application selection also used two stacked selectors, included macOS helper processes, and was absent from the floating control panel. Searchable menus used `scrollIntoView`, which can scroll the clipped menu ancestor and hide rows behind the search field in WebKit.

Settings and the floating panel must use the same compact searchable source picker, displaying either all applications or the selected application. Enumerate only on an explicit open/refresh action. Keep refresh as a small icon button; retain help in tooltips. On macOS list living regular applications, including hidden apps and apps without open windows; accessory-only menu-bar utilities are excluded from the new selection list. Existing saved targets remain displayed and can be replaced.

## Implementation boundaries

- Permit audio-input switching in Settings and the floating control panel. Permit enumeration and the new target-switch command in these same two surfaces only.
- Switch an application's capture target through the serialized session lifecycle. Preserve confirmed subtitles and paused/idle state; reconnect a listening system-audio session. Seal the old generation before resetting drafts and confirmation watermarks, including when the input type stays unchanged.
- A target change must not silently widen capture to all applications when a selected application exits. Display a recoverable error and let the user explicitly choose another target.
- Do not acquire capture permission merely on mounting settings or the floating panel. UI-only fixtures remain isolated from actual capture, provider networks, and credentials.
- Scroll only the results element for keyboard navigation; hover never scrolls. Keep the search field fixed and let asynchronously loaded results fill the bounded popup.

## Verification

- `./scripts/check.sh` passed: 982 Rust tests (2 ignored), 975 frontend tests, formatting, strict Clippy, ESLint, TypeScript and production build.
- Signed native UI-only smoke test passed: select Test Player from the floating menu, switch system-only → both → microphone-only there, then enable system and disable microphone from Settings. The selected target synchronized to Settings and neither surface showed an operation error. The search field and both options remained fully visible in each surface.
- Deferred enumeration, unchanged popup height while loading, missing targets, cross-window stale enumeration/errors, and failure after native preference persistence have focused frontend coverage. Paused-state capture boundaries have lifecycle coverage; no real microphone/provider capture was started by this smoke test.
- Restored the normal signed development app and checked the real macOS application list. Regular applications were visible, the search field and first result were fully displayed, and the saved all-applications target was preserved.
