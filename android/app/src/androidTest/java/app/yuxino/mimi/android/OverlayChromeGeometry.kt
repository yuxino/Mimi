package app.yuxino.mimi.android

import android.graphics.Rect
import android.view.View
import kotlin.math.abs

/** Normal captions contain reading content and adjustment handles, never a second settings surface. */
internal fun checkNoFloatingAppearanceEditor(panel: View) {
    for (tag in listOf("overlay-route", "overlay-font", "overlay-model", "overlay-providers",
        "enter-immersive", "collapse-overlay", "overlay-font-slider", "overlay-appearance-scroll")) {
        check(panel.findViewWithTag<View>(tag) == null) { "Obsolete floating control remains: $tag" }
    }
    check(android.view.inspector.WindowInspector.getGlobalWindowViews().none {
        it.findViewWithTag<View>("overlay-font-slider") != null ||
            it.findViewWithTag<View>("overlay-appearance-scroll") != null
    }) { "Floating appearance editor remains attached" }
}

/** Compare native layout bounds; ancestor preview scaling must not mix scaled origins with native widths. */
internal fun checkOverlayChromeAlignment(panel: View) {
    val caption = checkNotNull(panel.findViewWithTag<View>("expanded-subtitles"))
    fun bounds(view: View): Rect {
        var x = 0
        var y = 0
        var current = view
        while (current !== caption) {
            x += current.left
            y += current.top
            val parent = checkNotNull(current.parent as? View) { "Handle is not a caption descendant" }
            x -= parent.scrollX
            y -= parent.scrollY
            current = parent
        }
        return Rect(x, y, x + view.width, y + view.height)
    }
    val frame = bounds(caption)
    check(caption.findViewWithTag<View>("overlay-toolbar") == null &&
        caption.findViewWithTag<View>("overlay-actions") == null) { "Reserved adjustment rows returned" }
    val drag = checkNotNull(caption.findViewWithTag<View>("overlay-drag"))
    val mark = checkNotNull(drag.findViewWithTag<View>("overlay-drag-mark"))
    val resize = checkNotNull(caption.findViewWithTag<View>("overlay-resize"))
    val scroll = checkNotNull(caption.findViewWithTag<View>("subtitle-transcript-scroll"))
    val density = panel.resources.displayMetrics.density
    val target = (48 * density).toInt()
    check(drag.width == target && drag.height == target && resize.width == target && resize.height == target) {
        "Adjustment targets must remain 48dp without full-width rows"
    }
    val viewport = bounds(scroll)
    val chrome = frame.height() - scroll.height + scroll.paddingTop + scroll.paddingBottom
    check(frame.contains(viewport) && chrome <= (36 * density).toInt() + 2) {
        "Caption space is reserved for controls: frame=$frame viewport=$viewport verticalChrome=$chrome"
    }
    check(drag.visibility != View.GONE && mark.visibility != View.GONE && resize.visibility != View.GONE) { "Lock removed handle bounds" }
    check(abs(bounds(mark).exactCenterX() - frame.exactCenterX()) <= 1f) {
        "Drag mark is not centered: frame=$frame mark=${bounds(mark)} drag=${bounds(drag)}"
    }
    check(frame.contains(bounds(drag)) && frame.contains(bounds(resize))) { "Adjustment handle exceeds caption frame" }
    check(!Rect.intersects(bounds(drag), bounds(resize))) { "Adjustment handles overlap" }
    checkNoFloatingAppearanceEditor(caption)
}

/** The unlock control stays actionable and inside the current display after relayout/rotation. */
internal fun checkAdjustmentSwitchOnScreen(): com.google.android.material.materialswitch.MaterialSwitch {
    val toggle = android.view.inspector.WindowInspector.getGlobalWindowViews().single { it.tag == "overlay-adjust" }
        as com.google.android.material.materialswitch.MaterialSwitch
    val params = toggle.layoutParams as android.view.WindowManager.LayoutParams
    check(toggle.isShown && toggle.isEnabled && toggle.isClickable)
    check(params.flags and android.view.WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE == 0)
    check(params.flags and android.view.WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE != 0)
    val minimum = (48 * toggle.resources.displayMetrics.density).toInt()
    check(toggle.width >= minimum && toggle.height >= minimum)
    if (android.os.Build.VERSION.SDK_INT >= 30) {
        val metrics = toggle.context.getSystemService(android.view.WindowManager::class.java).currentWindowMetrics
        val insets = metrics.windowInsets.getInsetsIgnoringVisibility(
            android.view.WindowInsets.Type.systemBars() or android.view.WindowInsets.Type.displayCutout())
        val safe = Rect(metrics.bounds.left + insets.left, metrics.bounds.top + insets.top,
            metrics.bounds.right - insets.right, metrics.bounds.bottom - insets.bottom)
        val right = safe.right - params.x
        val bottom = safe.bottom - params.y
        val switchBounds = Rect(right - toggle.width, bottom - toggle.height, right, bottom)
        check(safe.contains(switchBounds)) { "Independent unlock switch is outside the current safe display" }
        val caption = android.view.inspector.WindowInspector.getGlobalWindowViews().single { it.tag == "mimi-overlay" }
        val captionParams = caption.layoutParams as android.view.WindowManager.LayoutParams
        val captionLeft = safe.left + (safe.width() - caption.width) / 2 + captionParams.x
        val captionBottom = safe.bottom - captionParams.y
        val captionBounds = Rect(captionLeft, captionBottom - caption.height, captionLeft + caption.width, captionBottom)
        val required = toggle.height + (4 * toggle.resources.displayMetrics.density).toInt()
        if (captionBounds.top - safe.top >= required || safe.bottom - captionBounds.bottom >= required) {
            check(!Rect.intersects(switchBounds, captionBounds)) {
                "Adjustment switch covers captions despite available outside space: caption=$captionBounds switch=$switchBounds"
            }
        }
    }
    return toggle
}
