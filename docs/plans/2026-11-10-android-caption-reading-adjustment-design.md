# Android caption adjustment switch Implementation Plan

> Execute in the current checkout; preserve unrelated pending edits. No extra worktree.

**Goal:** Implement issue #338 exactly as annotated: remove yellow-marked non-caption chrome and use a switch whose ON state enables move/resize and OFF state passes touches to the underlying app.

**Architecture:** Keep the same `CaptionPanelView`, measured bounds, style, reading anchor and saved normal position in both adjustment states. Add a separately touchable switch window anchored to the panel's top-right corner; embedding the only recovery control inside `FLAG_NOT_TOUCHABLE` would lock the user out. The existing optional immersive presentation remains an independent in-app setting, not the new switch implementation. Both normal adjustment states use the same Android-safe 0.8 window alpha so switching does not dim the rendered captions; no text/background style preferences change.

**Tech Stack:** Kotlin native Android views, WindowManager, existing Material switch and Android resource translations; JVM/native instrumentation regressions.

## User correction / acceptance boundary

The screenshot explicitly says ON = move/resize, OFF = click through. It crosses out route/settings and provider/Immersive footer chrome. The earlier plan to alternate ordinary and compact immersive frames was incorrect and is superseded. Switching must not select another presentation or another saved position.

## Accepted compact visual redesign

The initial functionally verified layout was rejected by the user: hiding two
48dp chrome rows preserved too much empty space, and the full-size switch
competed with captions. The accepted direction is a compact subtitle surface,
not a settings card. Functional test success is not visual acceptance.

- Use a FrameLayout with a full-size transcript viewport, 16dp horizontal and
  20dp top / 16dp bottom insets, and a 12dp surface radius. No toolbar/footer rows.
- Overlay 48dp drag and resize hit targets at the top center / bottom right;
  draw only a 24×3dp top grip and a small bottom-corner resize mark, outside
  the glyph viewport. They consume no layout rows. While adjusting, their hit
  areas can overlap the viewport edge; the rest remains available for reading.
- Use a compact switch drawing within an independent 48dp target, outside the
  top-right edge, with a 4dp gap. Fall back below when at the top screen edge,
  then clamp for constrained displays. Do not move the caption to fit controls.
- New unset defaults: 14% frame height (10–70% selectable), 48dp bottom offset.
  Native visual review rejected 12% because long translated captions reduced to
  a one-row tail, sometimes a single character. At default 16sp/92% width, keep
  enough room for one source row and two translated rows without a large card.
  Minimum readable source/translation lines (one full glyph row each, with a
  64dp viewport floor) still bound actual height; do not reserve three worst-case
  fallback-font rows for every short sentence. Preserve
  every explicit saved size, position, opacity, color, alignment and history setting.
- Keep the same frame, reading anchor, alpha and text layout on ON/OFF. The
  control is a toggle, not normal/immersive mode selection; failures roll back.
- Verify exact viewport geometry, new minimum slider/resize values, saved-size
  preservation, top-edge companion fallback, rotation and same-frame toggle.
- Inspect actual native ON/OFF captures on the same deterministic playback
  backdrop, plus waiting/long text. Clearly label synthetic visual fixtures;
  they do not establish real audio/provider or cross-UID touch acceptance.
- Publish only related code/docs in the PR. Attach review images in its comment,
  not Git. Preserve unrelated work in AGENTS and the existing integration ledger.

## Task 1: Native panel and preview

- Modify `android/app/src/main/java/app/yuxino/mimi/android/CaptionPanelView.kt`: remove route/provider/style/Watch action and its constructor/callback data. Keep drag/resize targets; hide their visual marks while locked without changing measured chrome. Keep preview controls inert.
- Modify `android/app/src/main/java/app/yuxino/mimi/android/SubtitlePreviewView.kt`: construct the simplified panel and stop calculating removed provider/route identity. Preserve actual measured preview parity.
- Keep status, source identity, timestamps, bounded history, intentional reading and original/translation modes.

## Task 2: Independent adjustment switch

- Modify `android/app/src/main/java/app/yuxino/mimi/android/SettingsStore.kt`: persist a separate adjustment-enabled preference, default true, without repurposing immersive or position keys.
- Modify `android/app/src/main/java/app/yuxino/mimi/android/capture/MimiService.kt`: create a minimum 48dp separately touchable switch for the normal panel. ON allows gestures; OFF sets FLAG_NOT_TOUCHABLE and Android-safe alpha, preserving panel identity and geometry. Save before applying; roll back the switch and show a sanitized failure if saving fails.
- Invalidate drag/resize generations when locking, rotating or changing presentation; do not commit interrupted gestures. Keep companion layout within display safe bounds and hide/clean up with the owned overlay.
- Remove the now-unreachable floating appearance editor. MainActivity remains the owner of appearance/service settings. Retain legacy immersive setting and its independent restore control.
- Use `overlay_adjust` in all nine locales; remove unused pending Watch copy.

## Task 3: Regression tests

- Update instrumentation under `android/app/src/androidTest/java/app/yuxino/mimi/android/`: assert removed chrome, real drag/resize, switch direction and separate touchability, identical bounds/style/frame across lock/unlock, preview parity, all locales, cancellation, silence recovery and legacy immersive restore.
- Replace obsolete floating-editor assertions with the same relevant checks in MainActivity or explicit absence assertions. Do not delete unrelated behavioral coverage.
- Run `python3 -B scripts/check-android-overlay-placement.py` and targeted Android JVM/build/lint/instrumentation commands supported by the installed SDK/device. Report missing infrastructure explicitly; synthetic fixtures do not establish provider/audio acceptance.

## Task 4: Documentation and review

- Update `android/README.md`, `android/README.zh-CN.md` and relevant owning UI/parity guides with the exact switch semantics.
- Inspect diff for unrelated edits, secrets, source/content privacy and build artifacts. Preserve existing user changes in AGENTS and integration ledger.
- Record verified revision/build/evidence limitations only after an actual native run. The user authorized a PR and its screenshot attachments; do not merge or release. No screenshot assets in Git.
