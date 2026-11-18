# Reported UI regressions

- Expanded macOS controls must collapse on an outside click even when the nonactivating
  panel never became key. Observe local and other-application mouse-down events only
  while expanded; do not consume the event, activate Mimi, record coordinates, or request
  keyboard monitoring. `block2` is already in the dependency graph; a direct macOS-only
  dependency lets AppKit own the native callback blocks. Remove monitors when collapsed.
- Refs #44: timestamps stay on the left, so every non-immersive alignment reserves the
  left gutter. Centered text retains symmetric padding. Right-aligned text no longer
  reserves an unused timestamp gutter on the right.
- Refs #43: observe timeline viewport resizing and repin after layout, even without new
  subtitle events. Font/alignment/display changes also re-evaluate the latest position.
- Refs #42: switch UI copy in place instead of reloading the WebView. Keep a per-window
  language snapshot and notify the React root when backend settings change it. Copy and
  language tables resolve at render time; mounted updater state and native resource
  handles survive language changes. Shared localStorage is not the rendered-language
  source of truth, because another window may have already updated it.

No provider, recording, credential, or release behavior changes. Issue reports distinguish
controlled/native fixture reproduction from live provider sessions.
