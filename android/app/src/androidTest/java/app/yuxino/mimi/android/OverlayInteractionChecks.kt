package app.yuxino.mimi.android

import android.app.Activity
import android.app.Instrumentation
import android.content.Intent
import android.graphics.Bitmap
import android.os.Bundle
import android.os.SystemClock
import android.os.Handler
import android.os.Looper
import java.util.concurrent.FutureTask
import java.util.concurrent.TimeUnit
import java.util.concurrent.CountDownLatch
import android.view.MotionEvent
import android.view.InputDevice
import android.view.Gravity
import android.view.View
import android.view.inspector.WindowInspector
import android.widget.SeekBar
import android.widget.TextView
import android.view.WindowManager
import com.google.android.material.materialswitch.MaterialSwitch
import android.util.TypedValue
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.provider.SubtitleBus
import java.io.File
import kotlin.math.abs

/** Native one-panel adjustment, reading continuity and main-page appearance controls; no capture/providers. */
internal class OverlayInteractionChecks(private val test: Instrumentation) {
    private val context get() = test.targetContext
    private fun root() = WindowInspector.getGlobalWindowViews().first { it.tag == "mimi-overlay" }
    private fun caption() = root().findViewWithTag<TextView>("expanded-translation")
    private fun captionPixels(sp: Int) = TypedValue.applyDimension(
        TypedValue.COMPLEX_UNIT_SP, sp.toFloat(), context.resources.displayMetrics,
    )
    private fun surface(): Pair<Int, Float> {
        val panel = root().findViewWithTag<View>("expanded-subtitles")
        return checkNotNull((panel.background as android.graphics.drawable.GradientDrawable).color).defaultColor to panel.alpha
    }
    private fun backgroundPixel(): Int {
        idle(); SystemClock.sleep(350)
        val frame = checkNotNull(nativeFrame(root()))
        val scale = root().findViewWithTag<View>("expanded-subtitles").scaleY
        val image = checkNotNull(test.uiAutomation.takeScreenshot())
        return try {
            image.getPixel(frame.centerX(), frame.top + (4 * context.resources.displayMetrics.density * scale).toInt().coerceAtLeast(1))
        } finally { image.recycle() }
    }
    private fun compositorPixels(frame: android.graphics.Rect, excluded: List<android.graphics.Rect> = emptyList()): IntArray {
        idle(); SystemClock.sleep(350)
        val image = checkNotNull(test.uiAutomation.takeScreenshot())
        return try {
            val pixels = IntArray(frame.width() * frame.height())
            image.getPixels(pixels, 0, frame.width(), frame.left, frame.top, frame.width(), frame.height())
            for (y in 0 until frame.height()) for (x in 0 until frame.width()) {
                if (excluded.any { it.contains(frame.left + x, frame.top + y) }) pixels[y * frame.width() + x] = 0
            }
            pixels
        } finally { image.recycle() }
    }
    private fun captionCompositorPixels(): IntArray {
        val panel = root().findViewWithTag<CaptionPanelView>("expanded-subtitles")
        val excluded = listOf("overlay-adjust-mark", "overlay-drag", "overlay-resize").map { tag ->
            val view = panel.findViewWithTag<View>(tag)
            val origin = nativeOrigin(view)
            android.graphics.Rect(origin[0], origin[1], origin[0] + view.width, origin[1] + view.height)
        }
        return compositorPixels(checkNotNull(nativeFrame(root())), excluded)
    }
    private fun nativeFrame(view: View): android.graphics.Rect? {
        val window = view.rootView
        if (window.width <= 0 || window.height <= 0) return null
        if (window.tag != "mimi-overlay" && window.tag != "exit-immersive" && window.tag != "overlay-adjust") {
            val origin = IntArray(2).also(window::getLocationOnScreen)
            return android.graphics.Rect(origin[0], origin[1], origin[0] + window.width, origin[1] + window.height)
        }
        val wm = MimiService::class.java.getDeclaredField("windowManager").apply { isAccessible = true }
            .get(root().context) as android.view.WindowManager
        val safe = if (android.os.Build.VERSION.SDK_INT >= 30) {
            val metrics = wm.currentWindowMetrics
            val insets = metrics.windowInsets.getInsetsIgnoringVisibility(
                android.view.WindowInsets.Type.systemBars() or android.view.WindowInsets.Type.displayCutout())
            android.graphics.Rect(metrics.bounds.left + insets.left, metrics.bounds.top + insets.top,
                metrics.bounds.right - insets.right, metrics.bounds.bottom - insets.bottom)
        } else android.graphics.Rect().also(window::getWindowVisibleDisplayFrame)
        val params = window.layoutParams as android.view.WindowManager.LayoutParams
        val left = if (params.gravity and Gravity.HORIZONTAL_GRAVITY_MASK == Gravity.RIGHT)
            safe.right - window.width - params.x else safe.left + (safe.width() - window.width) / 2 + params.x
        val top = safe.bottom - window.height - params.y
        return android.graphics.Rect(left, top, left + window.width, top + window.height)
    }
    private fun nativeOrigin(view: View): IntArray {
        val location = IntArray(2).also(view::getLocationInWindow)
        val rootLocation = IntArray(2).also(view.rootView::getLocationInWindow)
        val frame = checkNotNull(nativeFrame(view)) { "Native window bounds are unavailable" }
        location[0] += frame.left - rootLocation[0]; location[1] += frame.top - rootLocation[1]
        return location
    }
    private fun tapVisible(view: View, xFraction: Float = .5f, yFraction: Float = .5f) {
        waitFor { view.isShown && view.width > 0 && view.height > 0 && nativeFrame(view) != null }
        val origin = IntArray(2)
        onUi {
            val visible = android.graphics.Rect()
            check(view.getGlobalVisibleRect(visible) && visible.width() == view.width && visible.height() == view.height) {
                "Native action is clipped"
            }
            nativeOrigin(view).copyInto(origin)
        }
        val time = SystemClock.uptimeMillis()
        for (action in listOf(MotionEvent.ACTION_DOWN, MotionEvent.ACTION_UP)) {
            val event = MotionEvent.obtain(time, SystemClock.uptimeMillis(), action,
                origin[0] + view.width * xFraction, origin[1] + view.height * yFraction, 0)
            event.source = InputDevice.SOURCE_TOUCHSCREEN
            try { check(test.uiAutomation.injectInputEvent(event, true)) } finally { event.recycle() }
        }
        idle()
    }
    private fun onUi(action: () -> Unit) {
        val work = FutureTask { action() }
        Handler(Looper.getMainLooper()).post(work)
        work.get(10, TimeUnit.SECONDS)
        idle()
    }
    private fun idle() {
        val completed = CountDownLatch(1)
        test.waitForIdle { completed.countDown() }
        check(completed.await(10, TimeUnit.SECONDS)) { "Native UI did not become idle" }
    }
    private fun waitFor(details: () -> String = { "" }, predicate: () -> Boolean) {
        repeat(60) {
            var ready = false
            onUi { ready = predicate() }
            if (ready) return
            SystemClock.sleep(100)
        }
        val caller = Throwable().stackTrace.first { it.methodName == "run" }
        var diagnostic = ""
        onUi { diagnostic = details() }
        error("Overlay interaction did not settle at line ${caller.lineNumber}: $diagnostic")
    }
    private fun capture(name: String) {
        test.sendStatus(0, Bundle().apply { putString("stream", "OVERLAY_CHECK $name\n") })
        idle()
        SystemClock.sleep(350) // Wait for WindowManager/compositor bounds after overlay resizing.
        val bitmap = checkNotNull(test.uiAutomation.takeScreenshot())
        try {
            val internal = File(context.filesDir, "ui-preview").apply {
                check(isDirectory || mkdirs()) { "Cannot create private instrumentation evidence directory" }
            }
            // Secondary Android users may not expose external storage through adb's shell namespace.
            for (directory in listOf(checkNotNull(context.getExternalFilesDir("ui-preview")), internal)) {
                File(directory, "interaction-$name.png").outputStream().use {
                    check(bitmap.compress(Bitmap.CompressFormat.PNG, 100, it))
                }
            }
        } finally { bitmap.recycle() }
    }

    private fun bringIntoView(view: View) {
        onUi { view.requestRectangleOnScreen(android.graphics.Rect(0, 0, view.width, view.height), true) }
    }
    private fun setSlider(home: MainActivity, id: Int, value: Int) {
        val seek = home.findViewById<SeekBar>(id)
        bringIntoView(seek)
        val origin = IntArray(2)
        onUi {
            val bounds = android.graphics.Rect()
            check(seek.getGlobalVisibleRect(bounds) && bounds.height() == seek.height) { "Slider is clipped" }
            nativeOrigin(seek).copyInto(origin)
        }
        val x = origin[0] + seek.paddingLeft + (seek.width - seek.paddingLeft - seek.paddingRight) *
            (value - seek.min).toFloat() / (seek.max - seek.min)
        val time = SystemClock.uptimeMillis()
        for (action in listOf(MotionEvent.ACTION_DOWN, MotionEvent.ACTION_UP)) {
            val event = MotionEvent.obtain(time, SystemClock.uptimeMillis(), action, x, origin[1] + seek.height / 2f, 0)
            event.source = InputDevice.SOURCE_TOUCHSCREEN
            try { check(test.uiAutomation.injectInputEvent(event, true)) } finally { event.recycle() }
        }
        idle()
    }
    private fun move(view: View, dx: Int, dy: Int) {
        waitFor { view.isShown && nativeFrame(view) != null }
        val origin = IntArray(2)
        onUi { nativeOrigin(view).copyInto(origin) }
        val density = view.resources.displayMetrics.density
        val x = origin[0] + view.width / 2f
        val y = origin[1] + view.height / 2f
        onUi {
            val local = IntArray(2).also(view::getLocationInWindow)
            val rootLocal = IntArray(2).also(view.rootView::getLocationInWindow)
            val visible = android.graphics.Rect(); view.getGlobalVisibleRect(visible)
            test.sendStatus(0, Bundle().apply { putString("stream", "NATIVE_GESTURE ${view.tag} origin=${origin.contentToString()} local=${local.contentToString()} root=${rootLocal.contentToString()} frame=${nativeFrame(view)} visible=$visible size=${view.width}x${view.height}\n") })
        }
        val time = SystemClock.uptimeMillis()
        for ((action, fraction) in listOf(MotionEvent.ACTION_DOWN to 0f, MotionEvent.ACTION_MOVE to .5f,
            MotionEvent.ACTION_MOVE to 1f, MotionEvent.ACTION_UP to 1f)) {
            val event = MotionEvent.obtain(time, SystemClock.uptimeMillis(), action,
                x + dx * density * fraction, y + dy * density * fraction, 0)
            event.source = InputDevice.SOURCE_TOUCHSCREEN
            try { check(test.uiAutomation.injectInputEvent(event, true)) } finally { event.recycle() }
            SystemClock.sleep(30)
            if (action == MotionEvent.ACTION_MOVE && view.tag == "overlay-drag") {
                onUi { checkAdjustmentSwitchOnScreen() }
            }
        }
        idle()
    }
    private fun running(value: Boolean) {
        MimiService::class.java.getDeclaredField("isRunning").apply { isAccessible = true; set(null, value) }
    }
    private fun aligned(view: TextView, choice: CaptionAlignment) =
        view.gravity and Gravity.RELATIVE_HORIZONTAL_GRAVITY_MASK == choice.gravity
    private fun previewRows(home: MainActivity): List<TextView> = listOf("source", "translation", "status").map { name ->
        SubtitlePreviewView::class.java.getDeclaredField(name).apply { isAccessible = true }
            .get(home.findViewById<SubtitlePreviewView>(R.id.subtitle_preview)) as TextView
    }
    private var underlyingTaps = 0

    private fun adjustment() = WindowInspector.getGlobalWindowViews().first { it.tag == "overlay-adjust" } as MaterialSwitch

    private fun checkAdjustmentRoundTrip() {
        val window = root()
        val panel = window.findViewWithTag<CaptionPanelView>("expanded-subtitles")
        val toggle = adjustment()
        val params = window.layoutParams as WindowManager.LayoutParams
        val flags = params.flags
        val alpha = params.alpha
        val frame = nativeFrame(window)
        fun placement() = listOf(params.x, params.y, params.width, params.height, params.gravity, params.type)
        val savedPlacement = placement()
        fun geometry(view: View): List<Int> {
            val location = IntArray(2).also(view::getLocationInWindow)
            return listOf(location[0], location[1], view.width, view.height)
        }
        val handles = listOf("overlay-drag-mark", "overlay-resize").map { panel.findViewWithTag<View>(it) }
        val handleBounds = handles.map(::geometry)
        val rows = listOf(panel.status, panel.source, panel.translation, panel.history, panel.timestamp)
        val rowBounds = rows.map(::geometry)
        val texts = rows.map { it.text.toString() }
        val typography = rows.map { listOf(it.textSize, it.currentTextColor, it.gravity, it.maxLines, it.typeface) }
        val style = surface()
        val compositorPixel = backgroundPixel()
        val captionPixels = captionCompositorPixels()
        val switchPixels = compositorPixels(checkNotNull(nativeFrame(toggle)))
        fun preferences() = listOf(SettingsStore.fontSize(context), SettingsStore.overlayOpacity(context),
            SettingsStore.overlayBgAlpha(context), SettingsStore.overlayWidthPercent(context), SettingsStore.overlayHeightPercent(context),
            SettingsStore.overlayXOffset(context, true), SettingsStore.expandedYOffset(context), SettingsStore.overlayYOffset(context), SettingsStore.displayMode(context),
            SettingsStore.captionAlignment(context), SettingsStore.subtitleDividers(context), SettingsStore.subtitleTimestamps(context),
            SettingsStore.historyLines(context), SettingsStore.translationColorIndex(context), SettingsStore.sourceLang(context),
            SettingsStore.targetLang(context), SettingsStore.immersiveSubtitles(context))
        val saved = preferences()
        check(toggle.isChecked && SettingsStore.overlayAdjustable(context)) { "ON must mean adjustable" }
        check(flags and WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE == 0)
        repeat(2) {
            tapVisible(toggle)
            waitFor { !toggle.isChecked && !SettingsStore.overlayAdjustable(context) &&
                params.flags and WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE != 0 }
            onUi {
                check(root() === window && window.findViewWithTag<View>("expanded-subtitles") === panel)
                check(placement() == savedPlacement) { "Lock moved or resized the native caption window" }
                check(nativeFrame(window) == frame && handles.map(::geometry) == handleBounds && rows.map(::geometry) == rowBounds)
                check(handles.all { it.visibility == View.INVISIBLE }) { "OFF must hide handles without removing their bounds" }
                check(params.flags == (flags or WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE))
                check(params.alpha == .8f && surface() == style) { "Lock changed saved appearance instead of only Android window alpha" }
                check(rows.map { it.text.toString() } == texts && rows.map { listOf(it.textSize, it.currentTextColor, it.gravity, it.maxLines, it.typeface) } == typography)
                check(preferences() == saved && !SettingsStore.immersiveSubtitles(context))
                check(toggle.rootView !== window && toggle.isShown && toggle.isClickable && toggle.isEnabled)
                val target = (48 * toggle.resources.displayMetrics.density).toInt()
                check(toggle.width >= target && toggle.height >= target)
                check((toggle.layoutParams as WindowManager.LayoutParams).flags and WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE != 0)
                check((toggle.layoutParams as WindowManager.LayoutParams).flags and WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE == 0) {
                    "Separate switch cannot unlock the pass-through caption"
                }
                checkOverlayChromeAlignment(panel)
                checkAdjustmentSwitchOnScreen()
            }
            check(backgroundPixel() == compositorPixel) { "Lock visibly dimmed the caption compositor surface" }
            check(captionCompositorPixels().contentEquals(captionPixels)) { "Lock changed caption pixels outside controls/handles" }
            check(!compositorPixels(checkNotNull(nativeFrame(toggle))).contentEquals(switchPixels)) { "Locked switch paint did not change state" }
            val beforePassThrough = underlyingTaps
            tapVisible(panel)
            waitFor { underlyingTaps == beforePassThrough + 1 }
            check(preferences() == saved) { "Pass-through input changed caption preferences" }
            capture("panel-locked-$it")
            tapVisible(toggle) // Actual injected input, not merely a programmatic checked assignment.
            waitFor { toggle.isChecked && SettingsStore.overlayAdjustable(context) && params.flags == flags }
            onUi {
                check(params.alpha == alpha && surface() == style && nativeFrame(window) == frame && placement() == savedPlacement)
                check(root() === window && window.findViewWithTag<View>("expanded-subtitles") === panel)
                check(handles.all { it.visibility == View.VISIBLE } && handles.map(::geometry) == handleBounds)
                check(rows.map(::geometry) == rowBounds && preferences() == saved)
                checkNoFloatingAppearanceEditor(panel)
            }
            check(backgroundPixel() == compositorPixel) { "Unlock changed the caption compositor surface" }
            onUi { checkAdjustmentSwitchOnScreen() }
            check(captionCompositorPixels().contentEquals(captionPixels)) { "Unlock changed caption pixels outside controls/handles" }
            check(compositorPixels(checkNotNull(nativeFrame(toggle))).contentEquals(switchPixels)) { "Unlocked switch paint did not restore" }
            val beforeInteractive = underlyingTaps
            tapVisible(panel)
            check(underlyingTaps == beforeInteractive) { "Adjustable caption unexpectedly passed input through" }
        }
        checkInsetSwitchCorners()
        check(SettingsStore.flushPendingWritesForTests(context))
    }

    private fun checkInsetSwitchCorners() {
        val window = root()
        val panel = window.findViewWithTag<CaptionPanelView>("expanded-subtitles")
        val toggle = adjustment()
        val frame = nativeFrame(window)
        fun bounds() = listOf(SettingsStore.overlayWidthPercent(context), SettingsStore.overlayHeightPercent(context),
            SettingsStore.overlayXOffset(context, true), SettingsStore.expandedYOffset(context))
        val saved = bounds()
        val text = listOf(panel.source.text.toString(), panel.translation.text.toString())
        val pixels = captionCompositorPixels()
        check(toggle.isChecked)
        // All four corners are outside the small painted track, but inside the 48dp input target.
        for ((x, y) in listOf(.05f to .05f, .95f to .05f, .05f to .95f, .95f to .95f)) {
            repeat(2) { iteration ->
                val before = underlyingTaps
                tapVisible(toggle, x, y)
                val expected = iteration == 1
                waitFor { toggle.isChecked == expected && SettingsStore.overlayAdjustable(context) == expected }
                onUi {
                    checkAdjustmentSwitchOnScreen()
                    check(root() === window && nativeFrame(window) == frame && bounds() == saved) {
                        "Inset corner tap dragged or resized captions"
                    }
                    check(underlyingTaps == before) { "Inset switch corner passed through to the underlying activity" }
                    check(listOf(panel.source.text.toString(), panel.translation.text.toString()) == text)
                    val flags = (window.layoutParams as WindowManager.LayoutParams).flags
                    check((flags and WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE == 0) == expected)
                }
                check(captionCompositorPixels().contentEquals(pixels)) { "Inset corner tap changed caption pixels" }
            }
        }
        capture("panel-inset-switch-corners")
    }

    private fun checkLockCancelsGestures() {
        for (tag in listOf("overlay-drag", "overlay-resize")) {
            val window = root()
            val frame = nativeFrame(window)
            fun bounds() = listOf(SettingsStore.overlayWidthPercent(context), SettingsStore.overlayHeightPercent(context),
                SettingsStore.overlayXOffset(context, true), SettingsStore.expandedYOffset(context))
            val saved = bounds()
            val handle = window.findViewWithTag<View>(tag)
            val down = SystemClock.uptimeMillis()
            fun touch(action: Int, delta: Float) = onUi {
                val event = MotionEvent.obtain(down, SystemClock.uptimeMillis(), action,
                    handle.width / 2f + delta, handle.height / 2f + delta, 0)
                try { handle.dispatchTouchEvent(event) } finally { event.recycle() }
            }
            touch(MotionEvent.ACTION_DOWN, 0f)
            touch(MotionEvent.ACTION_MOVE, 55f)
            tapVisible(adjustment())
            waitFor { !adjustment().isChecked && nativeFrame(root()) == frame }
            check(bounds() == saved) { "Lock persisted an unfinished $tag gesture" }
            tapVisible(adjustment())
            waitFor { adjustment().isChecked }
            // Events from the pre-lock generation must not resume or commit after unlocking.
            touch(MotionEvent.ACTION_MOVE, 90f)
            touch(MotionEvent.ACTION_UP, 90f)
            onUi {
                check(root() === window && nativeFrame(window) == frame && bounds() == saved) {
                    "Stale $tag input resumed after unlocking"
                }
                checkAdjustmentSwitchOnScreen()
                checkOverlayChromeAlignment(window)
            }
        }
        capture("panel-lock-cancelled-gestures")
    }

    private fun checkLockedPersistence() {
        tapVisible(adjustment())
        waitFor { !SettingsStore.overlayAdjustable(context) && !adjustment().isChecked }
        val frame = nativeFrame(root())
        onUi {
            SubtitleBus.onSourceDraft("Synthetic locked source update.", "en")
            SubtitleBus.onTranslationDraft("锁定时仍继续更新。")
        }
        waitFor { caption().text.toString() == "锁定时仍继续更新。" }
        check(nativeFrame(root()) == frame) { "Locked draft changed the caption frame" }
        onUi { SettingsStore.setImmersiveSubtitles(context, true) }
        context.startService(Intent(context, MimiService::class.java).setAction(MimiService.ACTION_APPLY_APPEARANCE))
        waitFor { WindowInspector.getGlobalWindowViews().any { it.tag == "exit-immersive" } }
        check(!SettingsStore.overlayAdjustable(context)) { "Legacy immersive mode overwrote adjustment preference" }
        tapVisible(WindowInspector.getGlobalWindowViews().first { it.tag == "exit-immersive" })
        waitFor { WindowInspector.getGlobalWindowViews().any { it.tag == "overlay-adjust" } }
        check(!SettingsStore.immersiveSubtitles(context) && !adjustment().isChecked)
        check((root().layoutParams as WindowManager.LayoutParams).flags and WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE != 0)
        check(SettingsStore.flushPendingWritesForTests(context))
        context.stopService(Intent(context, MimiService::class.java))
        waitFor { WindowInspector.getGlobalWindowViews().none { it.tag in listOf("mimi-overlay", "overlay-adjust", "exit-immersive") } }
        context.startService(Intent(context, MimiService::class.java).setAction(MimiService.ACTION_UI_PREVIEW).putExtra("preview_over_own_ui", true))
        waitFor { WindowInspector.getGlobalWindowViews().any { it.tag == "mimi-overlay" } &&
            WindowInspector.getGlobalWindowViews().any { it.tag == "overlay-adjust" && it.isShown } }
        onUi {
            running(true)
            check(!SettingsStore.overlayAdjustable(context) && !adjustment().isChecked)
            val params = root().layoutParams as WindowManager.LayoutParams
            check(params.flags and WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE != 0 && params.alpha == .8f)
            check(nativeFrame(root()) == frame) { "Recreated locked frame lost its saved bounds" }
            checkOverlayChromeAlignment(root())
        }
        capture("panel-recreated-locked")
        tapVisible(adjustment())
        waitFor { SettingsStore.overlayAdjustable(context) && adjustment().isChecked &&
            (root().layoutParams as WindowManager.LayoutParams).flags and WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE == 0 }
    }

    private fun checkCompactHeightDefaults() {
        // The caller already requires an idle, unconfigured fixture user. Touch only this appearance key.
        SettingsStore.overlayHeightPercent(context)
        val preferences = SettingsStore::class.java.getDeclaredField("prefs").apply { isAccessible = true }
            .get(null) as android.content.SharedPreferences
        val key = "overlay_height_percent"
        val existed = preferences.contains(key)
        val saved = preferences.getInt(key, SettingsStore.DEFAULT_OVERLAY_HEIGHT)
        try {
            check(preferences.edit().remove(key).commit())
            check(SettingsStore.overlayHeightPercent(context) == 17) { "Unset height must use the inset-control 17% default" }
            check(preferences.edit().putInt(key, 32).commit())
            check(SettingsStore.overlayHeightPercent(context) == 32) { "Legacy saved height was replaced by the compact default" }
            check(SettingsStore.flushPendingWritesForTests(context))
            check(preferences.getInt(key, -1) == 32 && SettingsStore.overlayHeightPercent(context) == 32)
        } finally {
            val restore = preferences.edit()
            if (existed) restore.putInt(key, saved) else restore.remove(key)
            check(restore.commit())
        }
    }

    /** A review pair uses ordinary caption paint and a labeled, deterministic playback backdrop. */
    private fun captureCompactReview(host: CaptureProbeActivity) {
        val history = SettingsStore.historyLines(context)
        try {
            onUi {
                host.setContentView(CaptionReviewBackdrop(host))
                SettingsStore.setOverlayBounds(context, 92, SettingsStore.DEFAULT_OVERLAY_HEIGHT, 0, 48)
                SettingsStore.setFontSize(context, 16) // Match the fresh-install reading size.
                SettingsStore.setOverlayBgAlpha(context, 80)
                SettingsStore.setOverlayOpacity(context, 100)
                SettingsStore.setTranslationColorIndex(context, 1)
                SettingsStore.setCaptionAlignment(context, CaptionAlignment.CENTER)
                SettingsStore.setDisplayMode(context, CaptionDisplayMode.BILINGUAL)
                SettingsStore.setHistoryLines(context, 0)
                SettingsStore.setSubtitleDividers(context, false)
                SettingsStore.setSubtitleTimestamps(context, false)
                SubtitleBus.clear()
                SubtitleBus.onSourceDraft("The coast grows quieter as the sun sets.", "en")
                SubtitleBus.onTranslationDraft("日落时分，海岸渐渐安静下来。")
            }
            context.startService(Intent(context, MimiService::class.java).setAction(MimiService.ACTION_APPLY_APPEARANCE))
            waitFor {
                val panel = root().findViewWithTag<CaptionPanelView>("expanded-subtitles")
                panel.isShown && panel.translation.text.toString() == "日落时分，海岸渐渐安静下来。" &&
                    abs(panel.translation.textSize - captionPixels(16)) < 1 && adjustment().isChecked &&
                    (root().layoutParams as WindowManager.LayoutParams).y == (48 * context.resources.displayMetrics.density).toInt()
            }
            val window = root()
            val panel = window.findViewWithTag<CaptionPanelView>("expanded-subtitles")
            val frame = checkNotNull(nativeFrame(window))
            val scroll = panel.transcriptScroll
            fun layout() = listOf<View>(scroll, panel.source, panel.translation).map {
                listOf(it.left, it.top, it.width, it.height, it.scrollX, it.scrollY)
            }
            val originalLayout = layout()
            val text = listOf(panel.source.text.toString(), panel.translation.text.toString())
            val style = surface()
            onUi {
                checkOverlayChromeAlignment(panel)
                checkAdjustmentSwitchOnScreen()
                val dp = panel.resources.displayMetrics.density
                check(abs(panel.height - scroll.height - 56 * dp) <= 2) { "Inset control chrome must remain 48dp top / 8dp bottom" }
                check(scroll.height >= 64 * dp - 2) {
                    "Default caption viewport is too small: frame=${panel.height} viewport=${scroll.height}"
                }
                check(frame.contains(checkNotNull(nativeFrame(adjustment())))) {
                    "Compact review switch escaped the subtitle frame"
                }
                if (android.os.Build.VERSION.SDK_INT >= 30) {
                    val metrics = context.getSystemService(WindowManager::class.java).currentWindowMetrics
                    val insets = metrics.windowInsets.getInsetsIgnoringVisibility(
                        android.view.WindowInsets.Type.systemBars() or android.view.WindowInsets.Type.displayCutout())
                    val safeHeight = metrics.bounds.height() - insets.top - insets.bottom
                    val previousFootprint = safeHeight * .14f + 52 * dp
                    check(frame.height() <= previousFootprint + 2) {
                        "Inset review occupies more height than the former 14% panel plus external 48dp switch/4dp gap"
                    }
                }
                for (row in listOf(panel.source, panel.translation)) {
                    val glyphs = checkNotNull(row.layout)
                    check(glyphs.lineCount <= row.maxLines) { "Short review caption was truncated" }
                    check(row.top - scroll.scrollY >= 0 && row.bottom - scroll.scrollY <= scroll.height) {
                        "Short review caption is outside the reading viewport"
                    }
                }
            }
            val reviewPixels = captionCompositorPixels()
            capture("review-adjustable")
            tapVisible(adjustment())
            waitFor { !SettingsStore.overlayAdjustable(context) && !adjustment().isChecked &&
                (window.layoutParams as WindowManager.LayoutParams).flags and WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE != 0 }
            onUi {
                check(root() === window && window.findViewWithTag<CaptionPanelView>("expanded-subtitles") === panel)
                check(nativeFrame(window) == frame && layout() == originalLayout && surface() == style)
                check(listOf(panel.source.text.toString(), panel.translation.text.toString()) == text)
                checkOverlayChromeAlignment(panel)
                checkAdjustmentSwitchOnScreen()
            }
            check(captionCompositorPixels().contentEquals(reviewPixels)) { "Review ON/OFF changed caption compositor pixels" }
            capture("review-locked")
            onUi {
                SubtitleBus.clear()
                SubtitleBus.onSourceDraft("Along the shore, the last light catches the water while the distant hills slowly fade into the evening sky.", "en")
                SubtitleBus.onTranslationDraft("沿着海岸，最后一缕阳光落在水面上，远处的山丘渐渐融入暮色。海风吹过，这段旅程仍在继续。")
            }
            waitFor { panel.translation.text.toString() == "沿着海岸，最后一缕阳光落在水面上，远处的山丘渐渐融入暮色。海风吹过，这段旅程仍在继续。" }
            idle()
            onUi {
                check(nativeFrame(window) == frame) { "Long captions expanded the compact review frame" }
                checkAdjustmentSwitchOnScreen()
                check(panel.translation.lineCount >= 2 && panel.translation.maxLines >= 2) {
                    "Fresh default must retain two translated rows for long captions"
                }
                for (row in listOf(panel.source, panel.translation)) {
                    val glyphs = checkNotNull(row.layout)
                    val first = (glyphs.lineCount - row.maxLines).coerceAtLeast(0)
                    val origin = row.top - scroll.scrollY + row.compoundPaddingTop - row.scrollY
                    check(origin + glyphs.getLineTop(first) >= 0 &&
                        origin + glyphs.getLineBottom(glyphs.lineCount - 1) <= scroll.height) {
                        "Long review caption glyphs are clipped in the reading viewport"
                    }
                }
            }
            capture("review-long-caption")
            onUi { SubtitleBus.clear() }
            waitFor { panel.status.isShown }
            check(nativeFrame(window) == frame) { "Waiting changed the compact review frame" }
            capture("review-waiting")
        } finally {
            onUi { SettingsStore.setHistoryLines(context, history) }
        }
    }

    private fun checkPrimaryAppearance(home: MainActivity) {
        onUi { openCaptionStyles(home) }
        for (value in listOf(12, 24, 17)) {
            setSlider(home, R.id.font_size, value)
            val sourceSp = if (SettingsStore.originalTextOnly(context) || SettingsStore.displayMode(context) == CaptionDisplayMode.ORIGINAL)
                value.toFloat() else (value * .9f).coerceAtLeast(12f)
            val expected = listOf(sourceSp, value.toFloat(), value.toFloat()).map {
                TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_SP, it, home.resources.displayMetrics)
            }
            waitFor(details = {
                "primary_font expectedSp=$value expectedPx=$expected saved=${SettingsStore.fontSize(context)} " +
                    "slider=${home.findViewById<SeekBar>(R.id.font_size).progress} " +
                    "previewPx=${previewRows(home).map { it.textSize }} " +
                    "serviceWindows=${WindowInspector.getGlobalWindowViews().count { it.tag == "mimi-overlay" }}"
            }) {
                SettingsStore.fontSize(context) == value && previewRows(home).zip(expected).all { (row, px) -> abs(row.textSize - px) < 1 }
            }
        }
        for ((id, values, read) in listOf(
            Triple(R.id.overlay_width, listOf(55, 92), { SettingsStore.overlayWidthPercent(context) }),
            Triple(R.id.overlay_height, listOf(10, 12, SettingsStore.DEFAULT_OVERLAY_HEIGHT, 20, 70, 30), { SettingsStore.overlayHeightPercent(context) }),
            Triple(R.id.overlay_bg_alpha, listOf(90, 37), { SettingsStore.overlayBgAlpha(context) }),
            Triple(R.id.overlay_opacity, listOf(100, 60), { SettingsStore.overlayOpacity(context) }),
        )) for (value in values) {
            setSlider(home, id, value)
            waitFor(details = { "primary_slider id=${home.resources.getResourceEntryName(id)} expected=$value saved=${read()} control=${home.findViewById<SeekBar>(id).progress}" }) {
                read() == value
            }
        }
        for (choice in CaptionAlignment.entries) {
            val spinner = home.findViewById<android.widget.Spinner>(R.id.caption_alignment)
            bringIntoView(spinner); onUi { spinner.setSelection(choice.ordinal) }
            waitFor(details = { "primary_alignment expected=$choice saved=${SettingsStore.captionAlignment(context)} control=${spinner.selectedItemPosition} rowGravity=${previewRows(home).map { it.gravity }}" }) {
                SettingsStore.captionAlignment(context) == choice && previewRows(home).all { aligned(it, choice) }
            }
        }
        val green = home.findViewById<View>(android.R.id.content).findViewWithTag<View>("caption-color-3")
        bringIntoView(green); tapVisible(green)
        waitFor(details = { "primary_color expected=3 saved=${SettingsStore.translationColorIndex(context)}" }) { SettingsStore.translationColorIndex(context) == 3 }
        checkNoFloatingAppearanceEditor(home.window.decorView)
    }

    // The first badge sample can create a profile; restoring empty values would retain that profile and translator draft.
    private fun providerMarkChecks(home: MainActivity) = ProfileSettingsSnapshot(context).use {
        val provider = app.yuxino.mimi.android.provider.ServiceProvider.DASHSCOPE
        val saved = SettingsStore.configuration(context, provider)
        val model = SettingsStore.qwenMtModel(context)
        val synthetic = app.yuxino.mimi.android.provider.ServiceConfiguration(provider,
            mapOf("apiKey" to "synthetic-ui-fixture-never-sent"), saved.endpoint, saved.model)
        for (independent in listOf(false, true)) {
            val translator = if (independent) app.yuxino.mimi.android.provider.TranslationConfiguration(
                provider = app.yuxino.mimi.android.provider.TextTranslationProvider.DEEPL,
                apiKey = "synthetic-ui-fixture-never-sent",
            ) else app.yuxino.mimi.android.provider.TranslationConfiguration(
                provider = app.yuxino.mimi.android.provider.TextTranslationProvider.BUILTIN,
            )
            check(SettingsStore.saveConfiguration(context, synthetic, translator, true, model))
            check(SettingsStore.activateProvider(context, provider))
            context.stopService(Intent(context, MimiService::class.java))
            waitFor { WindowInspector.getGlobalWindowViews().none { it.tag == "mimi-overlay" } }
            context.startService(Intent(context, MimiService::class.java).setAction(MimiService.ACTION_UI_PREVIEW)
                .putExtra("preview_over_own_ui", true))
            waitFor { WindowInspector.getGlobalWindowViews().any { it.tag == "mimi-overlay" } }
            val page = test.startActivitySync(Intent(context, ServiceSettingsActivity::class.java)
                .putExtra("provider", provider.id).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as ServiceSettingsActivity
            onUi { page.finish() }
            waitFor { home.hasWindowFocus() }
            onUi { SubtitleBus.onSourceDraft("Synthetic provider badge check.", "en"); SubtitleBus.onTranslationDraft("合成译文。") }
            waitFor { home.findViewById<View>(R.id.translation_provider_icon).isShown == independent }
            waitFor { root().findViewWithTag<View>("expanded-subtitles").isShown }
            onUi {
                checkNoFloatingAppearanceEditor(root())
                val preview = home.findViewById<SubtitlePreviewView>(R.id.subtitle_preview).frame as CaptionPanelView
                checkNoFloatingAppearanceEditor(preview)
                check(home.findViewById<View>(R.id.provider_icon).isShown)
                checkOverlayChromeAlignment(root())
            }
            capture(if (independent) "two-provider-main-icons" else "one-provider-main-icon")
            onUi { home.findViewById<android.widget.RadioGroup>(R.id.settings_tabs).check(R.id.tab_caption) }
        }
    }


    fun run(arguments: Bundle?) {
        val mode = SettingsStore.displayMode(context)
        val width = SettingsStore.overlayWidthPercent(context)
        val height = SettingsStore.overlayHeightPercent(context)
        val alignment = SettingsStore.captionAlignment(context)
        val font = SettingsStore.fontSize(context)
        val immersive = SettingsStore.immersiveSubtitles(context)
        val adjustable = SettingsStore.overlayAdjustable(context)
        val color = SettingsStore.translationColorIndex(context)
        val opacity = SettingsStore.overlayOpacity(context)
        val background = SettingsStore.overlayBgAlpha(context)
        val dividers = SettingsStore.subtitleDividers(context)
        val timestamps = SettingsStore.subtitleTimestamps(context)
        val x = SettingsStore.overlayXOffset(context, true)
        val y = SettingsStore.expandedYOffset(context)
        val prefs = context.getSharedPreferences("first_run", 0)
        val hadSeen = prefs.contains("seen"); val seen = prefs.getBoolean("seen", false)
        val hadImmersiveSeen = prefs.contains("immersive_seen"); val immersiveSeen = prefs.getBoolean("immersive_seen", false)
        var home: MainActivity? = null
        var host: CaptureProbeActivity? = null
        var failure: Throwable? = null
        try {
            check(!MimiService.isRunning && !SettingsStore.isConfigured(context)) { "Use a blank idle emulator" }
            onUi { checkCompactHeightDefaults() }
            onUi {
                SettingsStore.setImmersiveSubtitles(context, false)
                SettingsStore.setOverlayAdjustable(context, true)
                SettingsStore.setOverlayBounds(context, 92, 30, 0, 0)
                SettingsStore.setFontSize(context, 16)
                SettingsStore.setOverlayBgAlpha(context, 37)
                SettingsStore.setOverlayOpacity(context, 60)
                SettingsStore.setDisplayMode(context, CaptionDisplayMode.BILINGUAL)
                SettingsStore.setSubtitleDividers(context, false); SettingsStore.setSubtitleTimestamps(context, false)
                check(prefs.edit().putBoolean("seen", true).putBoolean("immersive_seen", true).commit())
            }
            home = test.startActivitySync(Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as MainActivity
            for (choice in CaptionAlignment.entries) {
                onUi { SettingsStore.setCaptionAlignment(context, choice) }
                waitFor { previewRows(home).all { aligned(it, choice) } }
            }
            capture("main-alignment-right")
            context.startService(Intent(context, MimiService::class.java).setAction(MimiService.ACTION_UI_PREVIEW).putExtra("preview_over_own_ui", true))
            waitFor { WindowInspector.getGlobalWindowViews().any { it.tag == "mimi-overlay" } }
            host = test.startActivitySync(Intent(context, CaptureProbeActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as CaptureProbeActivity
            onUi { host!!.setContentView(View(context).apply {
                setBackgroundColor(android.graphics.Color.WHITE)
                setOnTouchListener { _, event ->
                    if (event.actionMasked == MotionEvent.ACTION_UP) underlyingTaps++
                    true
                }
            }) }
            val window = root()
            onUi { running(true); SubtitleBus.clear() }
            waitFor { root().findViewWithTag<TextView>("expanded-status").isShown }
            check(root().findViewWithTag<View>("collapse-overlay") == null)
            onUi { checkOverlayChromeAlignment(root()) }
            val initialSurface = surface()
            check(initialSurface == (android.graphics.Color.argb(37 * 255 / 100, 0, 0, 0) to .6f)) {
                "First caption frame ignored saved background/opacity"
            }
            val initialPixel = backgroundPixel()
            val panel = root().findViewWithTag<CaptionPanelView>("expanded-subtitles")
            capture("panel-waiting")
            checkAdjustmentRoundTrip()
            val status = panel.status
            onUi { SubtitleBus.onStatus("Synthetic status fixture.") }
            waitFor { status.text.toString() == "Synthetic status fixture." }
            onUi { SubtitleBus.onSourceDraft("Synthetic live preview.", "en"); SubtitleBus.onTranslationDraft("合成实时预览。") }
            waitFor { panel.source.text.toString() == "Synthetic live preview." && panel.translation.text.toString() == "合成实时预览。" }
            checkAdjustmentRoundTrip()
            onUi { SubtitleBus.clear() }
            waitFor { status.text.toString() == InterfaceLanguage.context(context).getString(R.string.overlay_listening_empty) }
            for (value in listOf(12, 24, 17)) {
                onUi { SettingsStore.setFontSize(context, value) }
                waitFor { abs(status.textSize - captionPixels(value)) < 1 }
                check(surface() == initialSurface && backgroundPixel() == initialPixel) { "Font adjustment changed caption background" }
            }
            val initialWidth = root().width
            onUi {
                SettingsStore.setOverlayBounds(context, 55, 30, 0, 128)
                SettingsStore.setOverlayOpacity(context, 100)
                SubtitleBus.onSourceDraft("Synthetic source.", "en")
                SubtitleBus.onTranslationDraft("合成字幕示例。")
            }
            waitFor { caption().isShown && root().width < initialWidth }
            onUi { checkOverlayChromeAlignment(root()) }
            capture("panel-before-direct-resize")
            val frame = checkNotNull(nativeFrame(root()))
            val bitmap = checkNotNull(test.uiAutomation.takeScreenshot())
            try {
                val middle = frame.centerX()
                val inside = android.graphics.Color.red(bitmap.getPixel(middle, frame.top + 2))
                val outside = android.graphics.Color.red(bitmap.getPixel(middle, frame.top - 2))
                check(outside > 200 && outside - inside >= 30) {
                    "Native caption frame did not match compositor pixels: $frame, inside=$inside outside=$outside"
                }
            } finally { bitmap.recycle() }
            val beforeWidth = root().width; val beforeHeight = root().height
            move(root().findViewWithTag("overlay-resize"), 24, 32)
            onUi { test.sendStatus(0, Bundle().apply { putString("stream", "SIZE_GEOMETRY before=${beforeWidth}x$beforeHeight after=${root().width}x${root().height} saved=${SettingsStore.overlayWidthPercent(context)}/${SettingsStore.overlayHeightPercent(context)}\n") }) }
            waitFor { root().width > beforeWidth && root().height >= beforeHeight }
            onUi {
                if (root().height == beforeHeight) {
                    val availableHeight = if (android.os.Build.VERSION.SDK_INT >= 30) {
                        val metrics = context.getSystemService(android.view.WindowManager::class.java).currentWindowMetrics
                        val insets = metrics.windowInsets.getInsetsIgnoringVisibility(
                            android.view.WindowInsets.Type.systemBars() or android.view.WindowInsets.Type.displayCutout())
                        metrics.bounds.height() - insets.top - insets.bottom
                    } else android.graphics.Rect().also(root()::getWindowVisibleDisplayFrame).height()
                    // An already larger bilingual minimum cannot grow toward a
                    // lower 70% preference. Otherwise both dimensions must move.
                    check(beforeHeight >= availableHeight * 70 / 100 && SettingsStore.overlayHeightPercent(context) == 70) {
                        "Resize failed to increase height despite available space"
                    }
                }
            }
            check(SettingsStore.overlayWidthPercent(context) > 55 && SettingsStore.overlayHeightPercent(context) > 30)
            val beforeY = SettingsStore.expandedYOffset(context)
            // Large-font landscape frames can already be at the top clamp.
            // Drag toward the available space, then verify the actual saved move.
            val direction = if (beforeY > 0) 32 else -32
            move(root().findViewWithTag("overlay-drag"), 0, direction)
            waitFor { SettingsStore.expandedYOffset(context) != beforeY }
            onUi { checkOverlayChromeAlignment(root()) }
            capture("panel-direct-resize-and-drag")
            checkLockCancelsGestures()
            val captionPair = SubtitleBus.displaySource to SubtitleBus.displayTranslation
            for (display in CaptionDisplayMode.entries) {
                onUi { SettingsStore.setDisplayMode(context, display) }
                waitFor { caption().isShown == display.translationVisible &&
                    root().findViewWithTag<View>("expanded-source").isShown == display.sourceVisible }
                check(captionPair == (SubtitleBus.displaySource to SubtitleBus.displayTranslation))
            }
            onUi { SettingsStore.setDisplayMode(context, CaptionDisplayMode.BILINGUAL) }
            onUi { SettingsStore.setSubtitleDividers(context, true) }
            SystemClock.sleep(350); idle() // Finish the preceding display-mode layouts.
            val fixedHeight = root().height
            val currentPair = SubtitleBus.displaySource to SubtitleBus.displayTranslation
            context.startService(Intent(context, MimiService::class.java).setAction(MimiService.ACTION_UI_PREVIEW_HISTORY))
            waitFor { SubtitleBus.historySnapshot().isNotEmpty() && root().findViewWithTag<View>("history-divider").isShown }
            // The seed also replaces English with Japanese current text. Hold
            // the current glyphs constant to isolate retention from font metrics.
            onUi {
                SubtitleBus.onSourceDraft(currentPair.first, "en")
                SubtitleBus.onTranslationDraft(currentPair.second)
            }
            waitFor { panel.source.text.toString() == currentPair.first && panel.translation.text.toString() == currentPair.second && root().height == fixedHeight }
            check(root().height == fixedHeight) { "History changed the user's window height: $fixedHeight -> ${root().height}" }
            capture("panel-history-fixed-height")
            onUi {
                SubtitleBus.onSourceDraft("Synthetic long caption for native reading gestures. ".repeat(12), "en")
                SubtitleBus.onTranslationDraft("合成长句用于验证双语分配和全文回看。".repeat(12))
            }
            waitFor { panel.source.lineCount > panel.source.maxLines && panel.translation.lineCount > panel.translation.maxLines }
            move(panel.transcriptScroll, 0, 32)
            waitFor { !panel.followingLive && panel.source.maxLines == Int.MAX_VALUE && panel.translation.maxLines == Int.MAX_VALUE }
            // Main-loop idle does not mean ScrollView's inertial fling has ended.
            // Compare updates only after the reader's deliberate gesture settles.
            var stable = 0
            var lastPosition = panel.transcriptScroll.scrollY
            repeat(30) {
                if (stable < 3) {
                    SystemClock.sleep(100); idle()
                    val position = panel.transcriptScroll.scrollY
                    stable = if (position == lastPosition) stable + 1 else 0
                    lastPosition = position
                }
            }
            check(stable >= 3) { "Reading gesture did not finish its native fling" }
            val readingPosition = panel.transcriptScroll.scrollY
            onUi { SubtitleBus.onTranslationDraft("合成长句更新时应保留主动回看的位置。".repeat(14)) }
            idle()
            check(panel.transcriptScroll.scrollY == readingPosition) { "A subtitle update stole the history reader's position: $readingPosition -> ${panel.transcriptScroll.scrollY}" }
            capture("panel-full-text-reading")
            onUi { panel.transcriptScroll.fullScroll(View.FOCUS_DOWN) }
            // fullScroll uses Android's smooth animation; a new gesture would
            // otherwise cancel it before the reader reaches the live edge.
            waitFor { panel.transcriptScroll.scrollY >= panel.transcriptScroll.getChildAt(0).height - panel.transcriptScroll.height }
            move(panel.transcriptScroll, 0, -24)
            waitFor { panel.followingLive && panel.source.maxLines != Int.MAX_VALUE && panel.translation.maxLines != Int.MAX_VALUE }
            capture("panel-return-to-bilingual-live")
            val savedX = SettingsStore.overlayXOffset(context, true); val savedY = SettingsStore.expandedYOffset(context)
            repeat(2) {
                onUi { SettingsStore.setImmersiveSubtitles(context, true) }
                context.startService(Intent(context, MimiService::class.java).setAction(MimiService.ACTION_APPLY_APPEARANCE))
                waitFor { WindowInspector.getGlobalWindowViews().any { it.tag == "exit-immersive" } }
                onUi {
                    val restore = WindowInspector.getGlobalWindowViews().first { it.tag == "exit-immersive" }
                    check(restore.isClickable && (restore.layoutParams as android.view.WindowManager.LayoutParams).flags and
                        android.view.WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE == 0) { "Legacy immersive exit remained touch-through" }
                }
                for (choice in CaptionAlignment.entries) {
                    onUi { SettingsStore.setCaptionAlignment(context, choice) }
                    waitFor { (root().findViewWithTag<View>("compact-subtitle") as android.view.ViewGroup).getChildAt(2).let {
                        it is TextView && aligned(it, choice) } }
                }
                tapVisible(WindowInspector.getGlobalWindowViews().first { it.tag == "exit-immersive" })
                waitFor { root().findViewWithTag<View>("expanded-subtitles").isShown }
                check(root() === window && SettingsStore.overlayXOffset(context, true) == savedX && SettingsStore.expandedYOffset(context) == savedY)
            }
            onUi { checkOverlayChromeAlignment(root()) }
            capture("panel-after-immersive")
            checkLockedPersistence()
            if (arguments?.getString("overlay_visual_review") == "true") captureCompactReview(checkNotNull(host))
            onUi { running(false); host!!.finish(); host = null }
            waitFor { home.hasWindowFocus() }
            // This fixture explicitly shows an overlay over Mimi's own UI.
            // Stop that fixture before testing ordinary primary-page touches.
            context.stopService(Intent(context, MimiService::class.java))
            waitFor { WindowInspector.getGlobalWindowViews().none { it.tag in listOf("mimi-overlay", "overlay-adjust", "exit-immersive") } }
            onUi {
                home.findViewById<android.widget.RadioGroup>(R.id.settings_tabs).check(R.id.tab_caption)
                // The history fixture enabled dividers above. Establish the
                // intended starting state before exercising both tap directions.
                SettingsStore.setSubtitleDividers(context, false)
                SettingsStore.setSubtitleTimestamps(context, false)
            }
            checkPrimaryAppearance(home)
            for ((id, read) in listOf(R.id.subtitle_dividers to { SettingsStore.subtitleDividers(context) },
                R.id.subtitle_timestamps to { SettingsStore.subtitleTimestamps(context) })) {
                val control = home.findViewById<android.widget.CompoundButton>(id)
                waitFor { !read() && !control.isChecked }
                bringIntoView(control); tapVisible(control)
                waitFor { read() && control.isChecked }
                tapVisible(control); waitFor { !read() && !control.isChecked }
            }
            capture("primary-reading-options")
            providerMarkChecks(home)
            context.stopService(Intent(context, MimiService::class.java))
            waitFor { WindowInspector.getGlobalWindowViews().none { it.tag == "mimi-overlay" } }
            context.startService(Intent(context, MimiService::class.java).setAction(MimiService.ACTION_UI_PREVIEW))
            waitFor { WindowInspector.getGlobalWindowViews().any { it.tag == "mimi-overlay" } && !root().isShown }
            host = test.startActivitySync(Intent(context, CaptureProbeActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as CaptureProbeActivity
            waitFor { root().isShown }
            val existing = root()
            onUi { host!!.finish(); host = null }
            waitFor { home.hasWindowFocus() && !root().isShown }
            check(root() === existing)
            check(MimiService.captureProjectionForTests == null && !MimiService.isRunning)
        } catch (error: Throwable) {
            failure = error
            capture("failure")
        }
        finally {
            onUi {
                running(false); context.stopService(Intent(context, MimiService::class.java))
                host?.finish(); home?.finish()
                SettingsStore.setDisplayMode(context, mode)
                SettingsStore.setOverlayBounds(context, width, height, x, y)
                SettingsStore.setCaptionAlignment(context, alignment)
                SettingsStore.setFontSize(context, font); SettingsStore.setImmersiveSubtitles(context, immersive)
                SettingsStore.setOverlayAdjustable(context, adjustable)
                SettingsStore.setTranslationColorIndex(context, color)
                SettingsStore.setOverlayBgAlpha(context, background); SettingsStore.setOverlayOpacity(context, opacity)
                SettingsStore.setSubtitleDividers(context, dividers); SettingsStore.setSubtitleTimestamps(context, timestamps)
                val restore = prefs.edit()
                if (hadSeen) restore.putBoolean("seen", seen) else restore.remove("seen")
                if (hadImmersiveSeen) restore.putBoolean("immersive_seen", immersiveSeen) else restore.remove("immersive_seen")
                check(restore.commit())
            }
            check(SettingsStore.flushPendingWritesForTests(context))
        }
        test.finish(if (failure == null) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
            putString("stream", if (failure == null) "Native overlay interaction passed: one panel, actual size gestures, lock/unlock frame and flags, retained main appearance controls, waiting typography, alignment, bounded history, both main provider icon cases and legacy immersion restoration; no capture/providers.\n"
                else "Native overlay interaction failed: ${failure.javaClass.simpleName}: ${failure.message}\n")
        })
    }
}
