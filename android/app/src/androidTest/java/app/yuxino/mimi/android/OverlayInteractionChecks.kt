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
import android.util.TypedValue
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.provider.SubtitleBus
import java.io.File
import kotlin.math.abs

/** Native one-panel size, alignment and direct live editing; no capture/providers. */
internal class OverlayInteractionChecks(private val test: Instrumentation) {
    private val context get() = test.targetContext
    private fun root() = WindowInspector.getGlobalWindowViews().first { it.tag == "mimi-overlay" }
    private fun caption() = root().findViewWithTag<TextView>("expanded-translation")
    private fun captionPixels(sp: Int) = TypedValue.applyDimension(
        TypedValue.COMPLEX_UNIT_SP, sp.toFloat(), caption().resources.displayMetrics,
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
    private fun nativeFrame(view: View): android.graphics.Rect? {
        val window = view.rootView
        if (window.width <= 0 || window.height <= 0) return null
        if (window.tag != "mimi-overlay" && window.tag != "exit-immersive") {
            val origin = IntArray(2).also(window::getLocationOnScreen)
            return android.graphics.Rect(origin[0], origin[1], origin[0] + window.width, origin[1] + window.height)
        }
        val wm = MimiService::class.java.getDeclaredField("windowManager").apply { isAccessible = true }
            .get(window.context) as android.view.WindowManager
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
    private fun tapVisible(view: View) {
        waitFor { view.isShown && view.width > 0 && view.height > 0 && nativeFrame(view) != null }
        val origin = IntArray(2)
        onUi {
            val visible = android.graphics.Rect()
            check(view.getGlobalVisibleRect(visible) && visible.width() == view.width && visible.height() == view.height) {
                "Floating appearance action is clipped"
            }
            nativeOrigin(view).copyInto(origin)
        }
        val time = SystemClock.uptimeMillis()
        for (action in listOf(MotionEvent.ACTION_DOWN, MotionEvent.ACTION_UP)) {
            val event = MotionEvent.obtain(time, SystemClock.uptimeMillis(), action,
                origin[0] + view.width / 2f, origin[1] + view.height / 2f, 0)
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
    private fun waitFor(predicate: () -> Boolean) {
        repeat(60) {
            var ready = false
            onUi { ready = predicate() }
            if (ready) return
            SystemClock.sleep(100)
        }
        val caller = Throwable().stackTrace.first { it.methodName == "run" }
        error("Overlay interaction did not settle at line ${caller.lineNumber}")
    }
    private fun capture(name: String) {
        test.sendStatus(0, Bundle().apply { putString("stream", "OVERLAY_CHECK $name\n") })
        idle()
        SystemClock.sleep(350) // Wait for WindowManager/compositor bounds after overlay resizing.
        val bitmap = checkNotNull(test.uiAutomation.takeScreenshot())
        File(checkNotNull(context.getExternalFilesDir("ui-preview")), "interaction-$name.png").outputStream().use {
            bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)
        }
        bitmap.recycle()
    }

    private fun editor() = WindowInspector.getGlobalWindowViews().first {
        it.findViewWithTag<SeekBar>("overlay-font-slider") != null
    }
    private fun bringIntoView(view: View) {
        onUi { view.requestRectangleOnScreen(android.graphics.Rect(0, 0, view.width, view.height), true) }
    }
    private fun setSlider(id: Int, value: Int) {
        val seek = editor().findViewById<SeekBar>(id)
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
    private fun assertEditorAndCaptionSeparated() {
        // Nonfocusable overlays can retain a stale View/accessibility origin.
        // Use native window attributes and verify the frame against compositor pixels.
        waitFor {
            val panelFrame = nativeFrame(root())
            val editorFrame = nativeFrame(editor())
            panelFrame != null && editorFrame != null && panelFrame.bottom <= editorFrame.top
        }
        onUi { check(editor().findViewById<View>(R.id.subtitle_preview) == null) { "Quick settings duplicated the preview" } }
    }

    private fun assertCompleteInertPreview() {
        assertEditorAndCaptionSeparated()
        val window = root()
        val dialog = editor()
        val panel = window.findViewWithTag<CaptionPanelView>("expanded-subtitles")
        val geometry = nativeFrame(window)
        val bounds = listOf(SettingsStore.overlayWidthPercent(context), SettingsStore.overlayHeightPercent(context),
            SettingsStore.overlayXOffset(context, true), SettingsStore.expandedYOffset(context))
        onUi {
            for (tag in listOf("overlay-toolbar", "overlay-drag-mark", "overlay-route", "overlay-actions", "overlay-providers")) {
                check(panel.findViewWithTag<View>(tag).isShown) { "Quick preview hides $tag" }
            }
            check(panel.scaleX == panel.scaleY && panel.scaleY > 0f && panel.scaleY <= 1f)
            check(panel.height * panel.scaleY <= window.height + 1) { "Quick preview clips the complete frame" }
            check(panel.transcriptScroll.height >= (64 * context.resources.displayMetrics.density).toInt()) {
                "Quick preview changed the native reading viewport"
            }
        }
        for (tag in listOf("overlay-drag", "overlay-font", "enter-immersive", "overlay-providers", "overlay-resize")) {
            val control = panel.findViewWithTag<View>(tag)
            var x = 0f; var y = 0f
            onUi {
                check(control.isShown && control.isEnabled && !control.isClickable && !control.isFocusable) {
                    "Quick preview action $tag is not decorative"
                }
                val origin = nativeOrigin(control)
                x = origin[0] + control.width * panel.scaleX / 2
                y = origin[1] + control.height * panel.scaleY / 2
            }
            val time = SystemClock.uptimeMillis()
            for ((action, offset) in listOf(MotionEvent.ACTION_DOWN to 0f, MotionEvent.ACTION_MOVE to 12f,
                MotionEvent.ACTION_UP to 12f)) {
                val delta = if (tag == "overlay-drag" || tag == "overlay-resize") offset * context.resources.displayMetrics.density else 0f
                val event = MotionEvent.obtain(time, SystemClock.uptimeMillis(), action, x + delta, y - delta, 0)
                event.source = InputDevice.SOURCE_TOUCHSCREEN
                try { check(test.uiAutomation.injectInputEvent(event, true)) } finally { event.recycle() }
            }
            idle()
            onUi { control.performClick() } // Also gate programmatic/accessibility actions.
            check(root() === window && editor() === dialog && nativeFrame(window) == geometry &&
                !SettingsStore.immersiveSubtitles(context)) { "Quick preview action $tag changed a window or mode" }
            check(bounds == listOf(SettingsStore.overlayWidthPercent(context), SettingsStore.overlayHeightPercent(context),
                SettingsStore.overlayXOffset(context, true), SettingsStore.expandedYOffset(context))) {
                "Quick preview gesture changed saved bounds"
            }
        }
    }
    private fun providerMarkChecks(home: MainActivity) {
        val provider = app.yuxino.mimi.android.provider.ServiceProvider.DASHSCOPE
        val saved = SettingsStore.configuration(context, provider)
        val translation = SettingsStore.translationConfiguration(context)
        val deepL = SettingsStore.translationConfiguration(context, app.yuxino.mimi.android.provider.TextTranslationProvider.DEEPL).let {
            if (it.apiKey == "synthetic-ui-fixture-never-sent") it.copy(apiKey = "") else it
        }
        val model = SettingsStore.qwenMtModel(context)
        val active = SettingsStore.provider(context)
        val synthetic = app.yuxino.mimi.android.provider.ServiceConfiguration(provider,
            mapOf("apiKey" to "synthetic-ui-fixture-never-sent"), saved.endpoint, saved.model)
        try {
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
                    val marks = root().findViewWithTag<android.widget.LinearLayout>("overlay-providers")
                    check(marks.childCount == if (independent) 3 else 2)
                    val preview = home.findViewById<SubtitlePreviewView>(R.id.subtitle_preview).frame as CaptionPanelView
                    val previewMarks = preview.findViewWithTag<android.widget.LinearLayout>("overlay-providers")
                    check(previewMarks.childCount == marks.childCount && previewMarks.contentDescription == marks.contentDescription)
                    check(previewMarks.width == marks.width) { "Preview provider group differs from the floating frame" }
                    checkOverlayChromeAlignment(root())
                }
                capture(if (independent) "two-provider-marks" else "one-provider-mark")
                tapVisible(root().findViewWithTag("overlay-font"))
                waitFor { !root().findViewWithTag<View>("overlay-font").isClickable }
                assertCompleteInertPreview()
                capture(if (independent) "quick-two-provider-marks" else "quick-one-provider-mark")
                tapVisible(editor().findViewById(android.R.id.button1))
                waitFor { root().findViewWithTag<View>("overlay-providers").isClickable }
                tapVisible(root().findViewWithTag("overlay-providers"))
                waitFor { home.hasWindowFocus() && home.findViewById<android.widget.RadioGroup>(R.id.settings_tabs)
                    .checkedRadioButtonId == R.id.tab_service }
                onUi { home.findViewById<android.widget.RadioGroup>(R.id.settings_tabs).check(R.id.tab_caption) }
            }
        } finally {
            check(SettingsStore.saveConfiguration(context, saved, deepL, false, model))
            check(SettingsStore.saveConfiguration(context, saved, translation, true, model))
            SettingsStore.setProvider(context, active)
            check(SettingsStore.flushPendingWritesForTests(context))
        }
    }


    fun run(arguments: Bundle?) {
        val mode = SettingsStore.displayMode(context)
        val width = SettingsStore.overlayWidthPercent(context)
        val height = SettingsStore.overlayHeightPercent(context)
        val alignment = SettingsStore.captionAlignment(context)
        val font = SettingsStore.fontSize(context)
        val immersive = SettingsStore.immersiveSubtitles(context)
        val color = SettingsStore.translationColorIndex(context)
        val opacity = SettingsStore.overlayOpacity(context)
        val background = SettingsStore.overlayBgAlpha(context)
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
            onUi {
                SettingsStore.setImmersiveSubtitles(context, false)
                SettingsStore.setOverlayBounds(context, 92, 30, 0, 0)
                SettingsStore.setFontSize(context, 16)
                SettingsStore.setOverlayBgAlpha(context, 37)
                SettingsStore.setOverlayOpacity(context, 60)
                SettingsStore.setDisplayMode(context, CaptionDisplayMode.BILINGUAL)
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
            onUi { host!!.setContentView(View(context).apply { setBackgroundColor(android.graphics.Color.WHITE) }) }
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
            val route = panel.findViewWithTag<TextView>("overlay-route").text.toString()
            val provider = panel.findViewWithTag<View>("overlay-providers").contentDescription
            val ordinaryHeight = panel.height
            capture("panel-waiting")
            tapVisible(root().findViewWithTag("overlay-font"))
            capture("quick-settings-open")
            waitFor { WindowInspector.getGlobalWindowViews().any { it.findViewWithTag<SeekBar>("overlay-font-slider") != null } }
            waitFor { !root().findViewWithTag<View>("overlay-font").isClickable }
            assertCompleteInertPreview()
            check(panel === root().findViewWithTag<View>("expanded-subtitles") && panel.height == ordinaryHeight &&
                panel.findViewWithTag<TextView>("overlay-route").text.toString() == route &&
                panel.findViewWithTag<View>("overlay-providers").contentDescription == provider) {
                "Opening quick settings replaced or altered the live frame"
            }
            val status = root().findViewWithTag<TextView>("expanded-status")
            onUi { SubtitleBus.onStatus("Synthetic status fixture.") }
            waitFor { status.text.toString() == "Synthetic status fixture." }
            capture("quick-settings-live-status")
            onUi {
                SubtitleBus.onSourceDraft("Synthetic live preview.", "en")
                SubtitleBus.onTranslationDraft("合成实时预览。")
            }
            waitFor { panel.source.text.toString() == "Synthetic live preview." && panel.translation.text.toString() == "合成实时预览。" }
            capture("quick-settings-live-content")
            assertCompleteInertPreview()
            onUi { SubtitleBus.clear() }
            waitFor { status.text.toString() == InterfaceLanguage.context(context).getString(R.string.overlay_listening_empty) }
            for (value in listOf(12, 24, 17)) {
                setSlider(R.id.font_size, value)
                waitFor { SettingsStore.fontSize(context) == value && abs(status.textSize - captionPixels(value)) < 1 }
                check(surface() == initialSurface && backgroundPixel() == initialPixel) { "Font adjustment changed caption background" }
                assertEditorAndCaptionSeparated()
            }
            setSlider(R.id.overlay_opacity, 100)
            waitFor { surface().second == 1f }
            val opaquePixel = backgroundPixel()
            for (value in listOf(24, 12, 17)) {
                setSlider(R.id.font_size, value)
                waitFor { SettingsStore.fontSize(context) == value }
                check(surface().first == initialSurface.first && backgroundPixel() == opaquePixel) {
                    "Font adjustment changed background at 100% opacity"
                }
            }
            setSlider(R.id.overlay_opacity, 60)
            waitFor { surface() == initialSurface }
            check(backgroundPixel() == initialPixel) { "Opacity round-trip did not restore the original pixels" }
            val initialWidth = root().width
            setSlider(R.id.overlay_width, 55)
            waitFor { SettingsStore.overlayWidthPercent(context) == 55 && root().width < initialWidth }
            setSlider(R.id.overlay_width, 92)
            waitFor { root().width == initialWidth }
            setSlider(R.id.overlay_height, 20)
            waitFor { SettingsStore.overlayHeightPercent(context) == 20 }
            val shortHeight = panel.height
            setSlider(R.id.overlay_height, 70)
            waitFor { SettingsStore.overlayHeightPercent(context) == 70 }
            onUi {
                test.sendStatus(0, Bundle().apply { putString("stream", "HEIGHT_GEOMETRY short=$shortHeight tall=${panel.height}\n") })
            }
            waitFor { panel.height > shortHeight }
            assertCompleteInertPreview()
            setSlider(R.id.overlay_height, 30)
            for (choice in CaptionAlignment.entries) {
                val spinner = editor().findViewById<android.widget.Spinner>(R.id.caption_alignment)
                bringIntoView(spinner)
                onUi { spinner.setSelection(choice.ordinal) }
                waitFor { SettingsStore.captionAlignment(context) == choice && aligned(status, choice) }
            }
            val green = editor().findViewWithTag<View>("caption-color-3")
            bringIntoView(green); tapVisible(green)
            waitFor { SettingsStore.translationColorIndex(context) == 3 && status.currentTextColor == SettingsStore.translationColor(context) }
            assertEditorAndCaptionSeparated()
            capture("quick-settings-live-green")
            setSlider(R.id.overlay_bg_alpha, 90)
            waitFor { SettingsStore.overlayBgAlpha(context) == 90 }
            setSlider(R.id.overlay_opacity, 60)
            waitFor { abs(root().findViewWithTag<View>("expanded-subtitles").alpha - .6f) < .01f }
            tapVisible(editor().findViewById(android.R.id.button1))
            waitFor { root().findViewWithTag<View>("overlay-font").isClickable && panel.scaleX == 1f && panel.scaleY == 1f }
            check(root() === window && SettingsStore.expandedYOffset(context) == 0) { "Closing editor changed saved placement or window" }
            capture("panel-style-applied")
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
                check(android.graphics.Color.red(bitmap.getPixel(middle, frame.top + 2)) < 160 &&
                    android.graphics.Color.red(bitmap.getPixel(middle, frame.top - 2)) > 200) {
                    "Native caption frame did not match compositor pixels: $frame"
                }
            } finally { bitmap.recycle() }
            val beforeWidth = root().width; val beforeHeight = root().height
            move(root().findViewWithTag("overlay-resize"), 24, 32)
            onUi { test.sendStatus(0, Bundle().apply { putString("stream", "SIZE_GEOMETRY before=${beforeWidth}x$beforeHeight after=${root().width}x${root().height} saved=${SettingsStore.overlayWidthPercent(context)}/${SettingsStore.overlayHeightPercent(context)}\n") }) }
            waitFor { root().width > beforeWidth && root().height > beforeHeight }
            check(SettingsStore.overlayWidthPercent(context) > 55 && SettingsStore.overlayHeightPercent(context) > 30)
            val beforeY = SettingsStore.expandedYOffset(context)
            // Large-font landscape frames can already be at the top clamp.
            // Drag toward the available space, then verify the actual saved move.
            val direction = if (beforeY > 0) 32 else -32
            move(root().findViewWithTag("overlay-drag"), 0, direction)
            waitFor { SettingsStore.expandedYOffset(context) != beforeY }
            onUi { checkOverlayChromeAlignment(root()) }
            capture("panel-direct-resize-and-drag")
            val captionPair = SubtitleBus.displaySource to SubtitleBus.displayTranslation
            for (display in CaptionDisplayMode.entries) {
                onUi { SettingsStore.setDisplayMode(context, display) }
                waitFor { caption().isShown == display.translationVisible &&
                    root().findViewWithTag<View>("expanded-source").isShown == display.sourceVisible }
                check(captionPair == (SubtitleBus.displaySource to SubtitleBus.displayTranslation))
            }
            onUi { SettingsStore.setDisplayMode(context, CaptionDisplayMode.BILINGUAL) }
            val fixedHeight = root().height
            context.startService(Intent(context, MimiService::class.java).setAction(MimiService.ACTION_UI_PREVIEW_HISTORY))
            waitFor { SubtitleBus.historySnapshot().isNotEmpty() && root().findViewWithTag<View>("history-divider").isShown }
            check(root().height == fixedHeight) { "History changed the user's window height" }
            capture("panel-history-fixed-height")
            val savedX = SettingsStore.overlayXOffset(context, true); val savedY = SettingsStore.expandedYOffset(context)
            repeat(2) {
                tapVisible(root().findViewWithTag("enter-immersive"))
                waitFor { WindowInspector.getGlobalWindowViews().any { it.tag == "exit-immersive" } }
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
            onUi { running(false); host!!.finish(); host = null }
            waitFor { home.hasWindowFocus() }
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
                SettingsStore.setTranslationColorIndex(context, color)
                SettingsStore.setOverlayBgAlpha(context, background); SettingsStore.setOverlayOpacity(context, opacity)
                val restore = prefs.edit()
                if (hadSeen) restore.putBoolean("seen", seen) else restore.remove("seen")
                if (hadImmersiveSeen) restore.putBoolean("immersive_seen", immersiveSeen) else restore.remove("immersive_seen")
                check(restore.commit())
            }
            check(SettingsStore.flushPendingWritesForTests(context))
        }
        test.finish(if (failure == null) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
            putString("stream", if (failure == null) "Native overlay interaction passed: one panel, actual size gestures, live quick settings, waiting typography, alignment, bounded history, both provider badge cases and immersion restoration; no capture/providers.\n"
                else "Native overlay interaction failed: ${failure.javaClass.simpleName}: ${failure.message}\n")
        })
    }
}
