# Android caption reading and adjustment Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make the Android caption overlay show only caption-related content while retaining an explicit switch between touch-through reading and temporary move/resize adjustment.

**Architecture:** Reuse the existing normal `CaptionPanelView` as the adjustment state and `CompactCaptionView` as the reading state. Remove duplicated route/provider/style chrome from the normal panel, keep its existing native drag/resize gestures, and retain the immersive window's `FLAG_NOT_TOUCHABLE`, alpha and separate touchable restore companion. Do not add a third mode or change shared Rust, capture, recognition, translation or saved normal/immersive position semantics.

**Tech Stack:** Android native Kotlin/Java views, `WindowManager`, Android resource XML, JVM and instrumentation fixtures.

---

## Design decisions

- Normal mode is the temporary adjustment state. Remove the language route, provider row/marks and top font-settings button. Keep the top drag handle, bottom-right resize handle and one visible action for entering the reading state. Appearance editing remains in `MainActivity`; the floating quick editor is removed.
- Immersive mode is the reading state. Keep original/translation text, necessary runtime status, optional timestamps and source identity. The caption window remains touch-through at alpha 0.8. Its independent 48dp companion stays clickable and returns to adjustment mode.
- Use localized action names `Watch` and `Adjust captions` rather than adding another preference. Keep current saved mode and separate normal/immersive position keys; do not copy one mode's position into the other.
- Keep MainActivity Start/Stop, notification Stop, service recovery, normal/immersive status, dual-source labels, reading history, accessibility descriptions and keyboard/TalkBack paths. Preview-only controls remain inert.
- Update the accepted single-panel design from `2026-10-11`: this issue intentionally supersedes route/provider/style chrome, not the one-panel rule or native geometry safeguards.

## Task 1: Simplify the normal adjustment panel

**Files:**
- Modify `android/app/src/main/java/app/yuxino/mimi/android/CaptionPanelView.kt`
- Modify `android/app/src/main/java/app/yuxino/mimi/android/capture/MimiService.kt`
- Modify `android/app/src/main/java/app/yuxino/mimi/android/SubtitlePreviewView.kt`

1. Add failing native assertions that `overlay-route`, `overlay-font`, `overlay-providers` and `overlay-model` are absent while `overlay-drag`, `enter-immersive` and `overlay-resize` retain 48dp targets.
2. Run the selected fixture and confirm it fails on the existing chrome.
3. Remove the route/provider/style views and unused constructor/callback data. Keep drag-to-move, resize-to-adjust-bounds, completed-gesture persistence and cancellation behavior unchanged.
4. Remove the floating quick editor and its duplicate-preview geometry. MainActivity appearance controls remain available.
5. Update the preview to construct the same simplified frame and remove stale route/provider identity work.
6. Run the selected fixture and confirm the simplified controls and preview pass.

## Task 2: Clarify reading/adjustment switching

**Files:**
- Modify `android/app/src/main/res/values/strings.xml` and all localized `values-*/strings.xml`
- Modify `android/app/src/main/java/app/yuxino/mimi/android/capture/MimiService.kt`
- Modify `android/app/src/main/java/app/yuxino/mimi/android/CompactCaptionView.kt` only if required by the final companion presentation

1. Add localized `overlay_watch` and `overlay_adjust` strings in all nine interface languages.
2. Use `overlay_watch` for the normal-panel reading action and `overlay_adjust` for the independent immersive companion's accessible action.
3. Keep `FLAG_NOT_TOUCHABLE`, alpha 0.8, separate companion touch handling, existing saved mode and separate position preferences unchanged.
4. Confirm status, source identity, silence-time recovery and preview-only accessibility remain intact.

## Task 3: Update native regression coverage

**Files:**
- Modify `android/app/src/androidTest/java/app/yuxino/mimi/android/OverlayInteractionChecks.kt`
- Modify `android/app/src/androidTest/java/app/yuxino/mimi/android/CaptionPreviewParityChecks.kt`
- Modify `android/app/src/androidTest/java/app/yuxino/mimi/android/InterfaceLanguageChecks.kt`
- Modify `android/app/src/androidTest/java/app/yuxino/mimi/android/UiSmokeInstrumentation.kt`
- Modify `android/app/src/androidTest/java/app/yuxino/mimi/android/OverlayChromeGeometry.kt`
- Modify `android/app/src/androidTest/java/app/yuxino/mimi/android/CaptionStreamingChecks.kt`
- Modify `android/app/src/androidTest/java/app/yuxino/mimi/android/LiveAcceptanceChecks.kt` if its provider-label assertion remains

1. Replace route/provider/quick-editor assertions with simplified-chrome assertions and no-floating-editor checks.
2. Retain actual drag/resize injection, rotation cancellation, reading history, dual-source identity, all nine languages/System, large fonts and preview pixel parity.
3. Retain immersive flags and companion recovery checks. Add an end-to-end fixture touch assertion when the existing harness supports a second recipient view; otherwise record the device-only limit explicitly.
4. Run static XML/key/placeholder checks and `python3 -B scripts/check-android-overlay-placement.py` where applicable.

## Task 4: Documentation and acceptance

**Files:**
- Modify `android/README.md`
- Modify `android/README.zh-CN.md`
- Modify `docs/development/platform-parity.md` if it describes the removed chrome
- Modify `docs/development/ui-guidelines.md` to reference the simplified adjustment/read states

1. Document the simple interaction model: Watch = touch-through captions, Adjust = move/resize; appearance controls live in the app.
2. Run canonical desktop/shared checks only if shared files change; otherwise run Android JVM/lint/build checks through CI and all available local static checks.
3. On an Android device, verify ordinary drag/resize, rotation cancellation, actual underlying-app touch delivery, TalkBack, silence recovery, long captions and all nine languages. Record device/API and evidence limits in the PR.
4. Upload before/after screenshots as PR comments, not repository assets. Do not claim real-provider or audio acceptance from synthetic fixtures.

Expected commits: this design record, then one focused Android implementation/test commit. PR closes #338 only after the simplified interaction and available checks are verified.
