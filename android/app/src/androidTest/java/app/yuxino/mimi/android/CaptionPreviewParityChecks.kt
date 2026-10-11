package app.yuxino.mimi.android

import android.app.Activity
import android.app.Instrumentation
import android.app.LocaleManager
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.os.Bundle
import android.os.LocaleList
import android.os.SystemClock
import android.view.View
import android.view.inspector.WindowInspector
import android.widget.TextView
import app.yuxino.mimi.android.capture.CaptureHealth
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.provider.SubtitleBus
import java.io.File

/** Actual system frame versus pinned preview; no audio or provider traffic. */
internal class CaptionPreviewParityChecks(private val test: Instrumentation) {
    private val context get() = test.targetContext
    private var cases = 0
    private var location = "setup"
    private fun ui(action: () -> Unit) {
        var failure: Throwable? = null
        test.runOnMainSync { try { action() } catch (error: Throwable) { failure = error } }
        test.waitForIdleSync(); failure?.let { throw it }
    }
    private fun await(predicate: () -> Boolean) {
        repeat(120) {
            var result = false; ui { result = predicate() }
            if (result) return
            SystemClock.sleep(40)
        }
        error("Frame did not settle at $location")
    }
    private fun root(): View? = WindowInspector.getGlobalWindowViews().firstOrNull { it.tag == "mimi-overlay" }
    private fun running(value: Boolean) {
        MimiService::class.java.getDeclaredField("isRunning").apply { isAccessible = true; set(null, value) }
    }
    private fun captureState(value: CaptureHealth.Snapshot?) {
        MimiService::class.java.getDeclaredField("captureObservation").apply { isAccessible = true; set(null, value) }
    }
    private fun refresh(home: MainActivity) = ui {
        MainActivity::class.java.getDeclaredMethod("refreshUi").apply { isAccessible = true }.invoke(home)
    }
    private fun bitmap(view: View): Bitmap = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888).also {
        val canvas = Canvas(it); canvas.drawColor(Color.WHITE)
        val layer = canvas.saveLayerAlpha(0f, 0f, view.width.toFloat(), view.height.toFloat(), (view.alpha * 255).toInt())
        view.draw(canvas); canvas.restoreToCount(layer)
    }
    private fun rows(view: View): List<TextView> = when (view) {
        is CaptionPanelView -> listOf(view.status, view.source, view.translation, view.history)
        is CompactCaptionView -> listOf(view.status, view.source, view.translation)
        else -> error("Unexpected frame type")
    }
    private fun compare(home: MainActivity, name: String, screenshot: Boolean = false) {
        location = name
        context.startService(Intent(context, MimiService::class.java).setAction(MimiService.ACTION_APPLY_APPEARANCE))
        refresh(home)
        val preview = home.findViewById<SubtitlePreviewView>(R.id.subtitle_preview)
        fun actual() = root()?.findViewWithTag<View>(if (SettingsStore.immersiveSubtitles(context)) "compact-subtitle" else "expanded-subtitles")
        await {
            val a = actual()
            a != null && a.isShown && preview.frame.width == a.width && preview.frame.height == a.height &&
                rows(preview.frame).zip(rows(a)).all { (x, y) -> x.text.toString() == y.text.toString() && x.visibility == y.visibility }
        }
        SystemClock.sleep(120); test.waitForIdleSync()
        ui {
            val actual = actual()!!
            check(actual.javaClass == preview.frame.javaClass && actual.alpha == preview.frame.alpha)
            for ((a, b) in rows(preview.frame).zip(rows(actual))) {
                check(a.textSize == b.textSize && a.currentTextColor == b.currentTextColor && a.gravity == b.gravity && a.typeface == b.typeface)
                if (a.visibility == View.VISIBLE) {
                    check(a.width == b.width && a.height == b.height && a.lineCount == b.lineCount && a.scrollY == b.scrollY) { "Caption wrapping differs at $name" }
                    for (line in 0 until a.lineCount) check(a.layout.getLineStart(line) == b.layout.getLineStart(line) && a.layout.getLineEnd(line) == b.layout.getLineEnd(line))
                }
            }
            if (actual is CaptionPanelView) {
                val sample = preview.frame as CaptionPanelView
                check(actual.chromeHeight() == sample.chromeHeight() && actual.transcriptScroll.scrollY == sample.transcriptScroll.scrollY) { "Reading viewport differs at $name" }
                for (tag in listOf("overlay-drag-mark", "overlay-route", "overlay-model", "overlay-providers", "enter-immersive")) {
                    val a = actual.findViewWithTag<View>(tag); val b = sample.findViewWithTag<View>(tag)
                    check(a.width == b.width && a.height == b.height && a.left == b.left && a.top == b.top) { "Frame chrome differs: $tag" }
                }
                for (tag in listOf("overlay-drag", "overlay-font", "overlay-providers", "enter-immersive", "overlay-resize")) {
                    check(!sample.findViewWithTag<View>(tag).performClick()) { "Preview navigated away" }
                }
            }
            // Scrollbar fade animations are platform timing, not caption geometry.
            val actualScroll = (actual as? CaptionPanelView)?.transcriptScroll
            val sampleScroll = (preview.frame as? CaptionPanelView)?.transcriptScroll
            val before = actualScroll?.isVerticalScrollBarEnabled
            actualScroll?.isVerticalScrollBarEnabled = false; sampleScroll?.isVerticalScrollBarEnabled = false
            val a = bitmap(actual); val b = bitmap(preview.frame)
            try {
                val pixelsA = IntArray(a.width * a.height); val pixelsB = IntArray(pixelsA.size)
                a.getPixels(pixelsA, 0, a.width, 0, 0, a.width, a.height)
                b.getPixels(pixelsB, 0, b.width, 0, 0, b.width, b.height)
                val differences = pixelsA.indices.count { pixelsA[it] != pixelsB[it] }
                check(differences == 0) { "Frame pixels differ at $name: $differences/${pixelsA.size}" }
                if (screenshot) File(context.getExternalFilesDir("ui-preview"), "parity-$name-frame.png").outputStream().use { check(b.compress(Bitmap.CompressFormat.PNG, 100, it)) }
            } finally {
                a.recycle(); b.recycle()
                if (before != null) { actualScroll!!.isVerticalScrollBarEnabled = before; sampleScroll!!.isVerticalScrollBarEnabled = before }
            }
            val visible = android.graphics.Rect()
            check(preview.stage.getGlobalVisibleRect(visible) && visible.height() == preview.stage.height && visible.width() == preview.stage.width) { "Pinned preview stage is clipped" }
            check(preview.stage.frameScale > 0f && preview.frame.width * preview.stage.frameScale <= preview.stage.width + 1 &&
                preview.frame.height * preview.stage.frameScale <= preview.stage.height + 1) { "Scaled frame clips" }
        }
        if (screenshot) {
            val bitmap = checkNotNull(test.uiAutomation.takeScreenshot())
            File(context.getExternalFilesDir("ui-preview"), "parity-$name-screen.png").outputStream().use { check(bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)) }; bitmap.recycle()
        }
        cases++
    }
    fun run(arguments: Bundle?) {
        if (arguments?.getString("apply_desktop_surface") == "true") {
            ui { SettingsStore.setOverlayOpacity(context, 100); SettingsStore.setOverlayBgAlpha(context, 80) }
            check(SettingsStore.flushPendingWritesForTests(context))
            test.finish(Activity.RESULT_OK, Bundle().apply { putString("stream", "Desktop black surface applied; other preferences retained.\n") }); return
        }
        check(!MimiService.isRunning)
        val manager = context.getSystemService(LocaleManager::class.java)
        val locales = manager.applicationLocales
        val capture = MimiService.captureObservation
        val restore = listOf(
            SettingsStore.fontSize(context).let { { SettingsStore.setFontSize(context, it) } },
            SettingsStore.overlayWidthPercent(context).let { { SettingsStore.setOverlayWidthPercent(context, it) } },
            SettingsStore.overlayHeightPercent(context).let { { SettingsStore.setOverlayHeightPercent(context, it) } },
            SettingsStore.overlayBgAlpha(context).let { { SettingsStore.setOverlayBgAlpha(context, it) } },
            SettingsStore.overlayOpacity(context).let { { SettingsStore.setOverlayOpacity(context, it) } },
            SettingsStore.immersiveSubtitles(context).let { { SettingsStore.setImmersiveSubtitles(context, it) } },
            SettingsStore.displayMode(context).let { { SettingsStore.setDisplayMode(context, it) } },
            SettingsStore.captionAlignment(context).let { { SettingsStore.setCaptionAlignment(context, it) } },
            SettingsStore.sourceLang(context).let { { SettingsStore.setSourceLang(context, it) } },
            SettingsStore.targetLang(context).let { { SettingsStore.setTargetLang(context, it) } },
            SettingsStore.historyLines(context).let { { SettingsStore.setHistoryLines(context, it) } },
        )
        val seenPrefs = context.getSharedPreferences("first_run", 0)
        val hadSeen = seenPrefs.contains("seen"); val seen = seenPrefs.getBoolean("seen", false)
        var home: MainActivity? = null
        var failure: Throwable? = null
        try {
            if (arguments?.getString("fresh_defaults") == "true") check(SettingsStore.overlayBgAlpha(context) == 80 && SettingsStore.overlayOpacity(context) == 100)
            ui { check(seenPrefs.edit().putBoolean("seen", true).commit()); SettingsStore.setSourceLang(context, "ja"); SettingsStore.setTargetLang(context, "zh") }
            val tags = arguments?.getString("locale")?.let { listOf(if (it == "system") "" else it) } ?: InterfaceLanguage.tags
            for (tag in tags) {
                ui { manager.applicationLocales = LocaleList.forLanguageTags(tag) }
                home = test.startActivitySync(Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as MainActivity
                context.startService(Intent(context, MimiService::class.java).setAction(MimiService.ACTION_UI_PREVIEW).putExtra("preview_over_own_ui", true))
                await { root() != null }; ui { running(true) }
                val prefix = tag.ifEmpty { "system" }
                ui {
                    SettingsStore.setImmersiveSubtitles(context, false); SettingsStore.setDisplayMode(context, CaptionDisplayMode.BILINGUAL)
                    SettingsStore.setFontSize(context, 17); SettingsStore.setOverlayWidthPercent(context, 92)
                    SettingsStore.setOverlayHeightPercent(context, 30); SettingsStore.setOverlayBgAlpha(context, 80); SettingsStore.setOverlayOpacity(context, 100)
                }
                compare(home, "$prefix-normal", true)
                val preview = home.findViewById<SubtitlePreviewView>(R.id.subtitle_preview)
                val stageHeight = preview.stage.height
                val slider = home.findViewById<View>(R.id.font_size)
                val sliderPosition = IntArray(2).also(slider::getLocationOnScreen)
                ui { SubtitleBus.clear() }; compare(home, "$prefix-waiting", true)
                ui { captureState(CaptureHealth.Snapshot(CaptureHealth.State.NO_PCM, null, null)) }
                compare(home, "$prefix-no-pcm")
                ui {
                    captureState(null)
                    val created = app.yuxino.mimi.android.provider.SharedSubtitleCore.exchange(org.json.JSONObject()
                        .put("operation", org.json.JSONObject().put("type", "create").put("history_limit", 0)))
                    SubtitleBus.onRuntimeSnapshot(org.json.JSONObject().put("atomicPreview", true).put("connectionStatus", "connecting")
                        .put("snapshot", created.getJSONObject("snapshot")), false)
                }
                compare(home, "$prefix-connecting")
                ui {
                    SubtitleBus.clear(); SubtitleBus.onSourceDraft("Synthetic caption with enough words to wrap onto multiple lines. ".repeat(5), "ja")
                    SubtitleBus.onTranslationDraft("合成字幕用于检查字号和窗口宽度，换行必须与外面完全相同。".repeat(6))
                    SettingsStore.setFontSize(context, 24); SettingsStore.setOverlayWidthPercent(context, 55); SettingsStore.setOverlayHeightPercent(context, 20)
                }
                compare(home, "$prefix-narrow-long", true)
                ui { SettingsStore.setFontSize(context, 12); SettingsStore.setOverlayWidthPercent(context, 100); SettingsStore.setOverlayHeightPercent(context, 70); SettingsStore.setOverlayBgAlpha(context, 100) }
                compare(home, "$prefix-solid-large")
                ui {
                    val color = ((root()!!.findViewWithTag<View>("expanded-subtitles").background as android.graphics.drawable.GradientDrawable).color!!).defaultColor
                    check(color == Color.BLACK) { "100% background is not pure black" }
                    check(preview.stage.height == stageHeight && IntArray(2).also(slider::getLocationOnScreen).contentEquals(sliderPosition)) { "Adjusting style moved the controls" }
                    SettingsStore.setOverlayBgAlpha(context, 0); SettingsStore.setOverlayOpacity(context, 60)
                }
                compare(home, "$prefix-clear")
                for ((index, mode) in CaptionDisplayMode.entries.withIndex()) {
                    ui { SettingsStore.setDisplayMode(context, mode); SettingsStore.setCaptionAlignment(context, CaptionAlignment.entries[index]) }
                    compare(home, "$prefix-${mode.name.lowercase()}")
                }
                ui { SettingsStore.setHistoryLines(context, 3); SettingsStore.setDisplayMode(context, CaptionDisplayMode.BILINGUAL) }
                context.startService(Intent(context, MimiService::class.java).setAction(MimiService.ACTION_UI_PREVIEW_HISTORY))
                await { SubtitleBus.historySnapshot().isNotEmpty() }
                compare(home, "$prefix-history", true)
                ui {
                    SettingsStore.setImmersiveSubtitles(context, true); SettingsStore.setDisplayMode(context, CaptionDisplayMode.BILINGUAL)
                    SubtitleBus.onSourceDraft("Synthetic immersion caption with enough words to overflow the compact viewport. ".repeat(5), "ja")
                    SubtitleBus.onTranslationDraft("合成沉浸字幕用于检查最后三行，与真实透明字幕窗口保持相同换行。".repeat(6))
                }
                compare(home, "$prefix-immersive-long", true)
                ui {
                    val compact = preview.frame as CompactCaptionView
                    check(compact.source.lineCount > compact.source.maxLines && compact.translation.lineCount > compact.translation.maxLines) {
                        "Long immersion fixture did not overflow both viewports"
                    }
                }
                ui { SubtitleBus.clear() }; compare(home, "$prefix-immersive-waiting")
                context.stopService(Intent(context, MimiService::class.java)); await { root() == null }; ui { running(false); home.finish() }
            }
        } catch (error: Throwable) { failure = error }
        finally {
            context.stopService(Intent(context, MimiService::class.java))
            ui { running(false); captureState(capture); home?.finish(); manager.applicationLocales = locales; restore.forEach { it() }
                check(seenPrefs.edit().apply { if (hadSeen) putBoolean("seen", seen) else remove("seen") }.commit()) }
            check(SettingsStore.flushPendingWritesForTests(context))
        }
        test.finish(if (failure == null) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
            putString("stream", if (failure == null) "Caption preview parity passed: $cases native cases; identical frame pixels/geometry; no capture or provider session.\n"
                else "Caption preview parity failed at $location: ${failure!!.javaClass.simpleName}: ${failure!!.message}\n")
        })
    }
}
