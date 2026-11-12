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
    val switchMark = checkNotNull(caption.findViewWithTag<com.google.android.material.materialswitch.MaterialSwitch>("overlay-adjust-mark"))
    val density = panel.resources.displayMetrics.density
    val target = (48 * density).toInt()
    check(drag.width == target && drag.height == target && resize.width == target && resize.height == target) {
        "Adjustment targets must remain 48dp without full-width rows"
    }
    val viewport = bounds(scroll)
    val chrome = frame.height() - scroll.height + scroll.paddingTop + scroll.paddingBottom
    check(frame.contains(viewport) && abs(chrome - (56 * density).toInt()) <= 2 &&
        viewport.top == (48 * density).toInt() && frame.bottom - viewport.bottom == (8 * density).toInt() &&
        viewport.left == (16 * density).toInt() && frame.right - viewport.right == (16 * density).toInt()) {
        "Inset controls changed the reading insets: frame=$frame viewport=$viewport verticalChrome=$chrome"
    }
    val switchBounds = bounds(switchMark)
    check(switchMark.width == target && switchMark.height == target && frame.contains(switchBounds)) {
        "Decorative switch must retain the full 48dp inset target geometry"
    }
    check(switchBounds.top == 0 && frame.right - switchBounds.right == (4 * density).toInt())
    check(!Rect.intersects(switchBounds, viewport) && !Rect.intersects(switchBounds, bounds(drag)) &&
        !Rect.intersects(switchBounds, bounds(resize))) { "Inset switch overlaps reading or adjustment targets" }
    val track = checkNotNull(switchMark.trackDrawable)
    val thumb = checkNotNull(switchMark.thumbDrawable)
    check(track.intrinsicWidth == (32 * density).toInt() && track.intrinsicHeight == (20 * density).toInt() &&
        thumb.intrinsicWidth == (16 * density).toInt()) { "Inset switch lost its compact track/thumb paint" }
    check(switchMark.visibility == View.VISIBLE && !switchMark.isClickable && !switchMark.isFocusable &&
        switchMark.importantForAccessibility == View.IMPORTANT_FOR_ACCESSIBILITY_NO) {
        "Decorative switch must not duplicate native input/accessibility"
    }
    check(switchMark.isChecked == SettingsStore.overlayAdjustable(panel.context)) { "Painted adjustment state is stale" }
    check(drag.visibility != View.GONE && mark.visibility != View.GONE && resize.visibility != View.GONE) { "Lock removed handle bounds" }
    check(abs(bounds(mark).exactCenterY() - 24 * density) <= 1f) { "Drag mark is not vertically centered in the inset controls" }
    check(abs(bounds(mark).exactCenterX() - frame.exactCenterX()) <= 1f) {
        "Drag mark is not centered: frame=$frame mark=${bounds(mark)} drag=${bounds(drag)}"
    }
    check(frame.contains(bounds(drag)) && frame.contains(bounds(resize))) { "Adjustment handle exceeds caption frame" }
    check(!Rect.intersects(bounds(drag), bounds(resize))) { "Adjustment handles overlap" }
    val resizeGlyph = bounds(resize).apply {
        left += resize.paddingLeft; top += resize.paddingTop
        right -= resize.paddingRight; bottom -= resize.paddingBottom
    }
    check(abs(resizeGlyph.width() - 12 * density) <= 1 && abs(resizeGlyph.height() - 12 * density) <= 1 &&
        !Rect.intersects(resizeGlyph, viewport)) { "Resize paint overlaps caption glyphs: glyph=$resizeGlyph viewport=$viewport" }
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
    check(toggle.width == minimum && toggle.height == minimum) { "Independent inset switch must have a 48dp square hit target" }
    val proxyImage = android.graphics.Bitmap.createBitmap(toggle.width, toggle.height, android.graphics.Bitmap.Config.ARGB_8888)
    try {
        toggle.draw(android.graphics.Canvas(proxyImage))
        val pixels = IntArray(toggle.width * toggle.height)
        proxyImage.getPixels(pixels, 0, toggle.width, 0, 0, toggle.width, toggle.height)
        val painted = pixels.count { android.graphics.Color.alpha(it) != 0 }
        check(painted == 0) {
            "Input proxy paints a second switch/compositor layer: pixels=$painted; focused=${toggle.isFocused}; " +
                "windowFocus=${toggle.hasWindowFocus()}; defaultFocus=${toggle.defaultFocusHighlightEnabled}; " +
                "background=${toggle.background?.javaClass?.simpleName}; foreground=${toggle.foreground?.javaClass?.simpleName}"
        }
    } finally { proxyImage.recycle() }
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
        check(captionBounds.contains(switchBounds)) {
            "Independent switch escaped its caption frame: caption=$captionBounds switch=$switchBounds"
        }
        check(switchBounds.top == captionBounds.top &&
            captionBounds.right - switchBounds.right == (4 * toggle.resources.displayMetrics.density).toInt()) {
            "Independent switch lost its top-right inset"
        }
        fun screenBounds(view: View): Rect {
            val origin = IntArray(2).also(view::getLocationInWindow)
            val rootOrigin = IntArray(2).also(caption::getLocationInWindow)
            val left = captionBounds.left + origin[0] - rootOrigin[0]
            val top = captionBounds.top + origin[1] - rootOrigin[1]
            return Rect(left, top, left + view.width, top + view.height)
        }
        val mark = checkNotNull(caption.findViewWithTag<com.google.android.material.materialswitch.MaterialSwitch>("overlay-adjust-mark"))
        check(switchBounds == screenBounds(mark)) { "Switch input and painted target rectangles differ" }
        check(mark.isChecked == toggle.isChecked && toggle.isChecked == SettingsStore.overlayAdjustable(toggle.context)) {
            "Switch input, paint and saved state differ"
        }
        for (tag in listOf("subtitle-transcript-scroll", "overlay-drag", "overlay-resize")) {
            check(!Rect.intersects(switchBounds, screenBounds(checkNotNull(caption.findViewWithTag<View>(tag))))) {
                "Independent inset switch intercepts $tag"
            }
        }
    }
    return toggle
}
