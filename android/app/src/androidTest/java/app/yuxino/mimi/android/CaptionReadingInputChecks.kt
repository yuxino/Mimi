package app.yuxino.mimi.android

import android.app.Instrumentation
import android.content.Context
import android.graphics.Bitmap
import android.os.SystemClock
import android.view.MotionEvent
import android.view.KeyEvent
import android.view.View
import android.view.ViewConfiguration
import android.view.ViewTreeObserver
import android.view.WindowManager
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/** Native reading inputs on an attached synthetic panel; the caller owns its window and cleanup. */
internal class CaptionReadingInputChecks(
    private val test: Instrumentation,
    private val panel: CaptionPanelView,
    private val preview: SubtitlePreviewView,
    private val report: JSONArray,
) {
    private val context get() = test.targetContext
    private var nativeDraws = 0

    private fun ui(action: () -> Unit) {
        var error: Throwable? = null
        test.runOnMainSync { try { action() } catch (failure: Throwable) { error = failure } }
        error?.let { throw it }
    }

    fun run(sourceShort: String, sourceLongest: String, translationShort: String, translationLong: String) {
        val scale = context.resources.displayMetrics.density
        fun dp(value: Int) = (value * scale).toInt()
        val manager = context.getSystemService(Context.WINDOW_SERVICE) as WindowManager
        val listener = ViewTreeObserver.OnDrawListener { nativeDraws++ }
        ui { panel.viewTreeObserver.addOnDrawListener(listener) }
        try {
            // A screen-reader scroll calls the framework action without touch events.
            // Use short current lanes and enough confirmed history for an actual scroll range.
            val accessibilityHistory = (1..12).joinToString("\n\n") { "Synthetic confirmed history row $it." }
            fun awaitAccessibilityViewport(label: String, predicate: () -> Boolean) {
                var stable = 0
                for (attempt in 0 until 150) {
                    var ready = false
                    ui { ready = !panel.isLayoutRequested && !panel.transcriptScroll.isLayoutRequested && predicate() }
                    stable = if (ready) stable + 1 else 0
                    if (stable == 3) return
                    SystemClock.sleep(20)
                }
                error("accessibility_viewport_timeout_$label")
            }
            fun screenshot(name: String) {
                val bitmap = checkNotNull(test.uiAutomation.takeScreenshot())
                try {
                    File(context.getExternalFilesDir("ui-scroll"), "$name.png").outputStream().use {
                        check(bitmap.compress(Bitmap.CompressFormat.PNG, 100, it))
                    }
                } finally { bitmap.recycle() }
            }
            var preparedDraw = 0
            ui {
                preparedDraw = nativeDraws
                panel.showContent(CaptionFrameContent(sourceShort, translationShort, history = accessibilityHistory), CaptionDisplayMode.BILINGUAL)
                panel.invalidate()
            }
            awaitAccessibilityViewport("history") {
                val scroll = panel.transcriptScroll
                nativeDraws > preparedDraw && scroll.canScrollVertically(-1) && !scroll.canScrollVertically(1)
            }
            var targetY = 0
            ui {
                val scroll = panel.transcriptScroll
                check(panel.followingLive)
                val action = android.view.accessibility.AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD
                check(scroll.createAccessibilityNodeInfo().actionList.any { it.id == action }) { "native_backward_action_unavailable" }
                targetY = (scroll.scrollY - (scroll.height - scroll.paddingTop - scroll.paddingBottom)).coerceAtLeast(0)
                check(scroll.performAccessibilityAction(action, null)) { "native_backward_action_rejected" }
            }
            awaitAccessibilityViewport("backward") { panel.transcriptScroll.scrollY == targetY }
            screenshot("accessibility-history-before-update")
            var readingY = 0
            var wasFollowingDuringRead = false
            var updatedDraw = 0
            ui {
                val scroll = panel.transcriptScroll
                check(scroll.canScrollVertically(1)) { "native_backward_action_did_not_move" }
                readingY = scroll.scrollY
                wasFollowingDuringRead = panel.followingLive
                updatedDraw = nativeDraws
                panel.showContent(CaptionFrameContent("The next synthetic caption arrives.", "下一条合成字幕到达。",
                    history = "$accessibilityHistory\n\nOne more synthetic history row."), CaptionDisplayMode.BILINGUAL)
                panel.invalidate()
            }
            awaitAccessibilityViewport("updated") { nativeDraws > updatedDraw }
            screenshot("accessibility-history-after-update")
            ui {
                val scroll = panel.transcriptScroll
                check(!wasFollowingDuringRead && !panel.followingLive && scroll.scrollY == readingY) {
                    "accessibility_history_interrupted: followingBefore=$wasFollowingDuringRead; followingAfter=${panel.followingLive}; beforeY=$readingY; afterY=${scroll.scrollY}; bottom=${!scroll.canScrollVertically(1)}"
                }
            }

            val backward = android.view.accessibility.AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD
            val forward = android.view.accessibility.AccessibilityNodeInfo.ACTION_SCROLL_FORWARD
            val up = android.view.accessibility.AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_UP.id
            val down = android.view.accessibility.AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_DOWN.id
            fun recordAccessibility(label: String) = ui {
                val current = panel; val scroll = current.transcriptScroll
                report.put(JSONObject().put("case", label).put("following", current.followingLive)
                    .put("offset", scroll.scrollY).put("canUp", scroll.canScrollVertically(-1))
                    .put("canDown", scroll.canScrollVertically(1)))
            }
            fun awaitAccessibilityDraw(label: String, action: () -> Unit) {
                var beforeDraw = 0
                ui { beforeDraw = nativeDraws; action(); panel.invalidate() }
                var previousY = -1
                awaitAccessibilityViewport(label) {
                    val y = panel.transcriptScroll.scrollY
                    val ready = nativeDraws > beforeDraw && y == previousY
                    previousY = y
                    ready
                }
            }
            fun scrollAccessibility(action: Int, label: String) {
                var beforeY = 0
                awaitAccessibilityDraw(label) {
                    val scroll = panel.transcriptScroll
                    beforeY = scroll.scrollY
                    check(scroll.performAccessibilityAction(action, null)) { "accessibility_action_rejected_$label" }
                }
                ui { check(panel.transcriptScroll.scrollY != beforeY || panel.followingLive) { "accessibility_action_did_not_move_$label" } }
            }
            fun returnAccessibilityLive(label: String) {
                for (step in 0 until 32) {
                    var following = false; ui { following = panel.followingLive }
                    if (following) {
                        ui { check(!panel.transcriptScroll.canScrollVertically(1)) { "accessibility_follow_not_at_bottom_$label" } }
                        return
                    }
                    scrollAccessibility(if (step % 2 == 0) down else forward, "$label-$step")
                }
                error("accessibility_return_to_live_timeout_$label")
            }
            fun showAccessibility(label: String, original: String = sourceShort, translated: String = translationShort,
                                  history: String = accessibilityHistory) = awaitAccessibilityDraw(label) {
                panel.showContent(CaptionFrameContent(original, translated, history = history), CaptionDisplayMode.BILINGUAL)
            }
            fun rejectAccessibility(actions: List<Int>, label: String) {
                awaitAccessibilityDraw(label) {
                    val current = panel; val scroll = current.transcriptScroll
                    val before = listOf(scroll.scrollY, current.source.scrollY, current.translation.scrollY,
                        if (current.followingLive) 1 else 0)
                    for (action in actions) {
                        check(!scroll.performAccessibilityAction(action, null)) { "accessibility_rejected_action_handled_$label-$action" }
                        check(before == listOf(scroll.scrollY, current.source.scrollY, current.translation.scrollY,
                            if (current.followingLive) 1 else 0)) { "accessibility_rejected_action_changed_state_$label-$action" }
                    }
                }
                recordAccessibility(label)
            }
            fun historyAnchor(): Pair<Int, Int> {
                val current = panel; val history = current.history
                val layout = checkNotNull(history.layout)
                val top = current.transcriptScroll.scrollY - history.top - history.compoundPaddingTop
                check(top in 0 until layout.height) { "accessibility_anchor_is_not_history" }
                val line = layout.getLineForVertical(top)
                return line to (layout.getLineTop(line) - top)
            }

            // Both native action pairs keep reading until a successful move reaches the end.
            scrollAccessibility(up, "up")
            ui { check(!panel.followingLive && panel.transcriptScroll.canScrollVertically(1)) }
            scrollAccessibility(forward, "forward-before-end")
            ui { check(!panel.followingLive && panel.transcriptScroll.canScrollVertically(1)) { "accessibility_forward_resumed_too_early" } }
            recordAccessibility("forward-before-end")
            returnAccessibilityLive("down-to-end")
            recordAccessibility("down-to-end")
            showAccessibility("next-caption-follows", history = "$accessibilityHistory\n\nA new confirmed caption.")
            recordAccessibility("next-caption-follows")
            var followFailure: String? = null
            ui {
                val current = panel; val scroll = current.transcriptScroll
                if (!current.followingLive || scroll.canScrollVertically(1)) {
                    followFailure = "accessibility_new_caption_not_followed: following=${current.followingLive}; y=${scroll.scrollY}; max=${scroll.getChildAt(0).height - scroll.height}; draws=$nativeDraws; layout=${current.isLayoutRequested}/${scroll.isLayoutRequested}"
                }
            }
            followFailure?.let { error(it) }
            rejectAccessibility(listOf(-1), "invalid-live")
            ui { panel.transcriptScroll.isEnabled = false }
            rejectAccessibility(listOf(backward, up, forward, down), "disabled-live")
            ui { panel.transcriptScroll.isEnabled = true }

            // A rejected forward must not silently set a future return-to-live intent.
            scrollAccessibility(backward, "reading-rejections")
            rejectAccessibility(listOf(-1), "invalid-reading")
            ui { panel.transcriptScroll.isEnabled = false }
            rejectAccessibility(listOf(forward, down, backward, up), "disabled-reading")
            ui { panel.transcriptScroll.isEnabled = true }
            awaitAccessibilityDraw("programmatic-bottom") {
                val scroll = panel.transcriptScroll
                scroll.scrollTo(0, scroll.getChildAt(0).height)
            }
            ui { check(!panel.followingLive && !panel.transcriptScroll.canScrollVertically(1)) { "accessibility_rejection_armed_future_follow" } }
            rejectAccessibility(listOf(forward, down), "rejected-bottom-reading")
            var bottomReadingY = 0; ui { bottomReadingY = panel.transcriptScroll.scrollY }
            showAccessibility("rejected-bottom-new-caption", history = "$accessibilityHistory\n\nA new confirmed caption.\n\nA later confirmed caption.")
            ui { check(!panel.followingLive && panel.transcriptScroll.scrollY == bottomReadingY) { "accessibility_rejected_bottom_lost_reading" } }
            recordAccessibility("rejected-bottom-new-caption")
            returnAccessibilityLive("resume-after-rejection")
            showAccessibility("no-range-live", original = "Short.", translated = "简短。", history = "")
            ui { check(!panel.transcriptScroll.canScrollVertically(-1) && !panel.transcriptScroll.canScrollVertically(1)) }
            rejectAccessibility(listOf(backward, up, forward, down), "no-range-live")
            showAccessibility("restore-history")
            scrollAccessibility(up, "no-range-reading-entry")
            showAccessibility("no-range-reading", original = "Short.", translated = "简短。", history = "")
            ui { check(!panel.followingLive && !panel.transcriptScroll.canScrollVertically(-1) && !panel.transcriptScroll.canScrollVertically(1)) }
            rejectAccessibility(listOf(backward, up, forward, down), "no-range-reading")
            showAccessibility("restore-reading-history")
            returnAccessibilityLive("resume-after-no-range")

            // Opening the full current lanes must preserve a real visible history anchor.
            val longOriginal = sourceLongest.repeat(3)
            val longTranslation = translationLong.repeat(3)
            showAccessibility("long-live", longOriginal, longTranslation)
            ui {
                check(panel.source.lineCount > panel.source.maxLines && panel.source.scrollY > 0) { "accessibility_long_fixture_not_cropped" }
            }
            scrollAccessibility(backward, "long-history")
            var longAnchor: Pair<Int, Int>? = null
            ui {
                check(!panel.followingLive)
                for (row in listOf(panel.source, panel.translation)) {
                    check(row.maxLines >= row.lineCount && row.scrollY == 0) { "accessibility_reading_did_not_expose_full_lane" }
                }
                longAnchor = historyAnchor()
            }
            showAccessibility("long-history-update", longOriginal, longTranslation,
                "$accessibilityHistory\n\nA later synthetic history row.")
            ui { check(!panel.followingLive && historyAnchor() == longAnchor) { "accessibility_long_history_anchor_moved" } }
            recordAccessibility("long-history-anchor")
            screenshot("accessibility-long-history")
            returnAccessibilityLive("long-return")
            ui { check(panel.source.maxLines < panel.source.lineCount && !panel.transcriptScroll.canScrollVertically(1)) }
            recordAccessibility("long-return")

            // Opposite actions can finish synchronously before the reading layout is drawn.
            // If the first forward has no range yet, repeat backward/forward immediately;
            // native return values decide the path instead of assuming animation timing.
            awaitAccessibilityDraw("rapid-opposite-actions") {
                val current = panel; val scroll = current.transcriptScroll
                val oldBottom = scroll.scrollY
                check(current.followingLive && !scroll.canScrollVertically(1))
                check(scroll.performAccessibilityAction(backward, null))
                if (!scroll.performAccessibilityAction(forward, null)) {
                    check(scroll.performAccessibilityAction(up, null))
                    check(scroll.performAccessibilityAction(down, null))
                }
                check(scroll.scrollY == oldBottom) { "accessibility_rapid_fixture_missed_old_bottom" }
                check(!current.followingLive) { "accessibility_rapid_return_used_stale_layout" }
            }
            ui {
                check(!panel.followingLive && panel.transcriptScroll.canScrollVertically(1)) { "accessibility_rapid_return_lost_full_reading" }
                check(panel.source.maxLines >= panel.source.lineCount && panel.source.scrollY == 0)
            }
            recordAccessibility("rapid-opposite-actions")
            returnAccessibilityLive("rapid-return")
            // A later accepted action owns its observed history position even while the
            // first backward's full-lane layout is still pending.
            var rapidHistoryAnchor: Pair<Int, Int>? = null
            var rapidHistoryY = 0
            awaitAccessibilityDraw("rapid-history-position") {
                val current = panel; val scroll = current.transcriptScroll
                check(current.followingLive && !scroll.canScrollVertically(1))
                check(scroll.performAccessibilityAction(backward, null))
                check(scroll.performAccessibilityAction(up, null))
                check(scroll.performAccessibilityAction(backward, null))
                check(scroll.performAccessibilityAction(down, null))
                check(!current.followingLive)
                rapidHistoryY = scroll.scrollY
                rapidHistoryAnchor = historyAnchor()
            }
            ui {
                check(!panel.followingLive && panel.transcriptScroll.scrollY == rapidHistoryY &&
                    historyAnchor() == rapidHistoryAnchor) { "accessibility_rapid_history_position_overwritten" }
                for (row in listOf(panel.source, panel.translation)) {
                    check(row.maxLines >= row.lineCount && row.scrollY == 0) { "accessibility_rapid_history_lane_not_expanded" }
                }
            }
            recordAccessibility("rapid-history-position")
            returnAccessibilityLive("rapid-history-return")
            showAccessibility("final-caption-follows", history = "$accessibilityHistory\n\nFinal synthetic history row.")
            ui { check(panel.followingLive && !panel.transcriptScroll.canScrollVertically(1)) }
            recordAccessibility("final-caption-follows")

            // A slow drag consists of many sub-threshold MOVE events. Keep one
            // gesture clock and draw between moves so native touch slop applies.
            val slowDownTime = SystemClock.uptimeMillis()
            var slowStartY = 0
            fun slowTouch(action: Int, step: Int) {
                val event = MotionEvent.obtain(slowDownTime, SystemClock.uptimeMillis(), action,
                    dp(100).toFloat(), dp(24).toFloat() + step * scale, 0)
                try {
                    check(panel.transcriptScroll.dispatchTouchEvent(event)) { "slow_drag_event_rejected_$step" }
                } finally { event.recycle() }
            }
            ui { slowStartY = panel.transcriptScroll.scrollY; slowTouch(MotionEvent.ACTION_DOWN, 0) }
            for (step in 1..36) {
                awaitAccessibilityDraw("slow-drag-$step") { slowTouch(MotionEvent.ACTION_MOVE, step) }
            }
            awaitAccessibilityDraw("slow-drag-cancel") { slowTouch(MotionEvent.ACTION_CANCEL, 36) }
            recordAccessibility("slow-drag")
            ui {
                val current = panel; val scroll = current.transcriptScroll
                check(!current.followingLive && scroll.scrollY < slowStartY) {
                    "slow_drag_lost_reading: following=${current.followingLive}; beforeY=$slowStartY; afterY=${scroll.scrollY}"
                }
            }
            returnAccessibilityLive("slow-drag-return")

            // Real pointer dispatch reaches the nonfocusable, adjustable overlay.
            // Observe framework movement without replacing its scroll listener.
            val wheelLocation = IntArray(2)
            var wheelEvents = 0
            var wheelStartY = 0
            var wheelMinY = Int.MAX_VALUE
            val wheelScroll = ViewTreeObserver.OnScrollChangedListener {
                wheelMinY = minOf(wheelMinY, panel.transcriptScroll.scrollY)
            }
            ui {
                val params = panel.layoutParams as WindowManager.LayoutParams
                params.flags = params.flags and WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE.inv()
                manager.updateViewLayout(panel, params)
                val scroll = panel.transcriptScroll
                scroll.getLocationOnScreen(wheelLocation)
                wheelStartY = scroll.scrollY
                scroll.setOnGenericMotionListener { _, event ->
                    if (event.actionMasked == MotionEvent.ACTION_SCROLL) wheelEvents++
                    false
                }
                scroll.viewTreeObserver.addOnScrollChangedListener(wheelScroll)
            }
            test.waitForIdleSync()
            val wheelTime = SystemClock.uptimeMillis()
            val wheel = MotionEvent.obtain(wheelTime, wheelTime, MotionEvent.ACTION_SCROLL, 1,
                arrayOf(MotionEvent.PointerProperties().apply { id = 0; toolType = MotionEvent.TOOL_TYPE_MOUSE }),
                arrayOf(MotionEvent.PointerCoords().apply {
                    x = wheelLocation[0] + dp(100).toFloat(); y = wheelLocation[1] + dp(60).toFloat()
                    setAxisValue(MotionEvent.AXIS_VSCROLL, 1f)
                }), 0, 0, 1f, 1f, 0, 0, android.view.InputDevice.SOURCE_MOUSE, 0)
            try { check(test.uiAutomation.injectInputEvent(wheel, true)) { "wheel_injection_rejected" } }
            finally { wheel.recycle() }
            awaitAccessibilityDraw("wheel") {}
            ui {
                val scroll = panel.transcriptScroll
                scroll.setOnGenericMotionListener(null)
                scroll.viewTreeObserver.removeOnScrollChangedListener(wheelScroll)
                check(wheelEvents == 1) { "wheel_did_not_reach_overlay: events=$wheelEvents" }
                check(scroll.scrollY < wheelStartY) {
                    "wheel_movement_overwritten: beforeY=$wheelStartY; minY=$wheelMinY; afterY=${scroll.scrollY}; following=${panel.followingLive}"
                }
            }
            recordAccessibility("wheel")
            var wheelReadingY = 0
            ui { check(!panel.followingLive); wheelReadingY = panel.transcriptScroll.scrollY }
            showAccessibility("wheel-caption-update", history = "$accessibilityHistory\n\nAnother synthetic caption.")
            ui { check(!panel.followingLive && panel.transcriptScroll.scrollY == wheelReadingY) { "wheel_history_interrupted" } }
            returnAccessibilityLive("wheel-return")

            // A mode change can remove the captured row before reading layout;
            // its old crop must not shift the remaining translation.
            showAccessibility("pending-hidden-row", longOriginal, longTranslation, history = "")
            awaitAccessibilityDraw("hidden-row") {
                check(panel.source.scrollY > 0 && panel.transcriptScroll.scrollY == 0)
                check(dp(4) < ViewConfiguration.get(context).scaledTouchSlop)
                val now = SystemClock.uptimeMillis()
                for ((action, y) in listOf(MotionEvent.ACTION_DOWN to dp(24),
                    MotionEvent.ACTION_MOVE to dp(28), MotionEvent.ACTION_CANCEL to dp(28))) {
                    val event = MotionEvent.obtain(now, now, action, dp(100).toFloat(), y.toFloat(), 0)
                    try { panel.transcriptScroll.dispatchTouchEvent(event) } finally { event.recycle() }
                }
                check(!panel.followingLive)
                panel.showContent(CaptionFrameContent(longOriginal, longTranslation), CaptionDisplayMode.TRANSLATION)
            }
            ui {
                check(panel.source.visibility == View.GONE && !panel.followingLive)
                check(panel.transcriptScroll.scrollY == 0) {
                    "hidden_row_crop_restored: y=${panel.transcriptScroll.scrollY}; sourceCrop=${panel.source.scrollY}"
                }
            }
            recordAccessibility("hidden-row")
            checkPreviewKeyboard(accessibilityHistory)
        } finally {
            ui { panel.viewTreeObserver.removeOnDrawListener(listener) }
        }
    }

    private fun checkPreviewKeyboard(history: String) {
        val current = preview.frame as CaptionPanelView
        val scroll = current.transcriptScroll
        var draws = 0
        var keyEvents = 0
        var wasInTouchMode = false
        val listener = ViewTreeObserver.OnDrawListener { draws++ }
        fun settle(label: String, action: () -> Unit = {}) {
            var before = 0
            ui { before = draws; action(); current.invalidate() }
            var stable = 0
            var lastY = -1
            repeat(150) {
                var ready = false
                ui {
                    ready = draws > before && !current.isLayoutRequested && !scroll.isLayoutRequested && scroll.scrollY == lastY
                    lastY = scroll.scrollY
                }
                stable = if (ready) stable + 1 else 0
                if (stable == 3) return
                SystemClock.sleep(20)
            }
            error("preview_key_timeout_$label")
        }
        ui {
            wasInTouchMode = current.isInTouchMode
            panel.visibility = View.GONE
            current.viewTreeObserver.addOnDrawListener(listener)
            scroll.setOnKeyListener { _, _, event -> if (event.action == KeyEvent.ACTION_DOWN) keyEvents++; false }
        }
        try {
            settle("history") {
                preview.showContent(CaptionFrameContent("Synthetic preview source.", "合成预览字幕。", history = history))
                check(scroll.requestFocusFromTouch()) { "preview_scroll_focus_rejected" }
            }
            ui { check(scroll.hasFocus() && current.isShown && scroll.canScrollVertically(-1)) }
            settle("backward") { check(scroll.performAccessibilityAction(android.view.accessibility.AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD, null)) }
            for (step in 0 until 16) {
                var following = false; ui { following = current.followingLive }
                if (following) break
                settle("return-$step") { check(scroll.performAccessibilityAction(android.view.accessibility.AccessibilityNodeInfo.ACTION_SCROLL_FORWARD, null)) }
            }
            ui { check(current.followingLive && !scroll.canScrollVertically(1)) }
            var beforeY = 0
            ui { beforeY = scroll.scrollY }
            test.sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_UP)
            settle("up")
            ui {
                check(keyEvents == 1) { "preview_key_not_received" }
                check(scroll.scrollY < beforeY) { "preview_key_movement_overwritten: beforeY=$beforeY; afterY=${scroll.scrollY}" }
                report.put(JSONObject().put("case", "preview-key-up").put("offset", scroll.scrollY).put("following", current.followingLive))
            }
            settle("wheel-to-end") {
                val now = SystemClock.uptimeMillis()
                val wheel = MotionEvent.obtain(now, now, MotionEvent.ACTION_SCROLL, 1,
                    arrayOf(MotionEvent.PointerProperties().apply { id = 0; toolType = MotionEvent.TOOL_TYPE_MOUSE }),
                    arrayOf(MotionEvent.PointerCoords().apply { setAxisValue(MotionEvent.AXIS_VSCROLL, -20f) }),
                    0, 0, 1f, 1f, 0, 0, android.view.InputDevice.SOURCE_MOUSE, 0)
                try { check(scroll.dispatchGenericMotionEvent(wheel)) } finally { wheel.recycle() }
                check(!scroll.canScrollVertically(1))
                beforeY = scroll.scrollY
            }
            // Cross the framework's 250ms animation gap, so the next key starts
            // a fresh smooth scroll rather than taking its synchronous shortcut.
            SystemClock.sleep(300)
            test.sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_UP)
            settle("up-after-wheel")
            ui {
                check(keyEvents == 2)
                check(scroll.scrollY < beforeY) { "preview_new_key_animation_discarded: beforeY=$beforeY; afterY=${scroll.scrollY}" }
                check(!current.followingLive)
                report.put(JSONObject().put("case", "preview-key-after-wheel").put("offset", scroll.scrollY).put("following", current.followingLive))
            }
            ui { beforeY = scroll.scrollY }
            test.sendKeyDownUpSync(KeyEvent.KEYCODE_PAGE_UP)
            settle("page-up")
            ui { check(!current.followingLive && scroll.scrollY < beforeY) }
            test.sendKeyDownUpSync(KeyEvent.KEYCODE_MOVE_HOME)
            settle("home")
            ui { check(!current.followingLive && scroll.scrollY == 0) }
            test.sendKeyDownUpSync(KeyEvent.KEYCODE_MOVE_END)
            settle("end")
            ui { check(current.followingLive && !scroll.canScrollVertically(1)); beforeY = scroll.scrollY }
            val keyTime = SystemClock.uptimeMillis()
            test.sendKeySync(KeyEvent(keyTime, keyTime, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_SPACE, 0, KeyEvent.META_SHIFT_ON))
            test.sendKeySync(KeyEvent(keyTime, SystemClock.uptimeMillis(), KeyEvent.ACTION_UP, KeyEvent.KEYCODE_SPACE, 0, KeyEvent.META_SHIFT_ON))
            settle("shift-space")
            ui {
                check(!current.followingLive && scroll.scrollY < beforeY) { "preview_shift_space_did_not_enter_reading" }
                val readingY = scroll.scrollY
                check(!scroll.executeKeyEvent(KeyEvent(KeyEvent.ACTION_UP, KeyEvent.KEYCODE_DPAD_DOWN)))
                check(!scroll.executeKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_UNKNOWN)))
                check(!current.followingLive && scroll.scrollY == readingY) { "preview_rejected_key_changed_reading" }
                report.put(JSONObject().put("case", "preview-page-home-end-space").put("offset", scroll.scrollY).put("following", current.followingLive))
            }
        } finally {
            try {
                ui { scroll.setOnKeyListener(null); current.viewTreeObserver.removeOnDrawListener(listener) }
            } finally { test.setInTouchMode(wasInTouchMode) }
        }
    }
}
