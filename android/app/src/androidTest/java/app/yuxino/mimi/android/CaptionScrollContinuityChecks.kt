package app.yuxino.mimi.android

import android.app.Activity
import android.app.Instrumentation
import android.content.Context
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.graphics.Color
import android.graphics.PixelFormat
import android.os.Bundle
import android.os.SystemClock
import android.provider.Settings
import android.view.ContextThemeWrapper
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.ViewTreeObserver
import android.view.WindowManager
import android.widget.TextView
import app.yuxino.mimi.android.capture.MimiService
import org.json.JSONArray
import org.json.JSONObject

/** Synthetic Audio3/Qwen differential layout probe; no service, credentials, provider, or capture. */
internal class CaptionScrollContinuityChecks(private val test: Instrumentation) {
    private val context get() = test.targetContext
    private fun ui(action: () -> Unit) {
        var error: Throwable? = null
        test.runOnMainSync { try { action() } catch (failure: Throwable) { error = failure } }
        error?.let { throw it }
    }
    private data class Frame(val source: String, val translation: String, val confirmed: Boolean = false)
    private data class Geometry(val sourceLines: Int, val sourceLimit: Int, val sourceFirst: Int, val sourceTop: Int,
        val sourceBottom: Int, val translationLines: Int, val translationLimit: Int, val translationFirst: Int,
        val translationTop: Int, val translationBottom: Int, val viewport: Int) {
        fun json() = JSONObject().put("sourceLines", sourceLines).put("sourceLimit", sourceLimit).put("sourceFirst", sourceFirst)
            .put("sourceTop", sourceTop).put("sourceBottom", sourceBottom).put("translationLines", translationLines)
            .put("translationLimit", translationLimit).put("translationFirst", translationFirst).put("translationTop", translationTop)
            .put("translationBottom", translationBottom).put("viewport", viewport)
    }
    fun run() {
        var home: MainActivity? = null
        val guide = context.getSharedPreferences("first_run", 0)
        val hadGuideSeen = guide.contains("seen")
        val guideSeen = guide.getBoolean("seen", false)
        var ownsFixture = false
        var panel: CaptionPanelView? = null
        var manager: WindowManager? = null
        var listener: ViewTreeObserver.OnDrawListener? = null
        var failure: Throwable? = null
        var stage = -1
        var draws = 0
        var drawnStage = -1
        var drawFailure: String? = null
        val report = JSONArray()
        val accessibilityReport = JSONArray()
        var originalBudgetChanges = 0
        var unchangedSourceMovements = 0
        var unchangedTranslationMovements = 0
        try {
            check(context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0)
            check(!MimiService.isRunning && !MimiService.isStopping)
            check(Settings.canDrawOverlays(context)) { "Probe requires existing overlay permission" }
            check(SettingsStore.profiles(context).isEmpty()) { "Probe requires a blank debug test user" }
            ownsFixture = true
            check(guide.edit().putBoolean("seen", true).commit())
            home = test.startActivitySync(Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as MainActivity
            val keyguard = context.getSystemService(android.app.KeyguardManager::class.java)
            ui {
                home!!.setShowWhenLocked(true); home!!.setTurnScreenOn(true)
                home!!.window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                keyguard.requestDismissKeyguard(home, null)
            }
            var foregroundReady = false
            for (attempt in 0 until 150) {
                ui { foregroundReady = !keyguard.isKeyguardLocked && home!!.hasWindowFocus() && home!!.window.decorView.isShown && home!!.window.decorView.width > 0 }
                if (foregroundReady) break
                SystemClock.sleep(20)
            }
            check(foregroundReady) { "native_probe_foreground_not_ready" }
            val scale = context.resources.displayMetrics.density
            fun dp(value: Int) = (value * scale).toInt()
            // Fix the reading budget; caption chrome changes must not turn this
            // cropping regression into a sample whose entire text already fits.
            val readingViewportDp = 128
            fun geometry(): Geometry {
                val current = checkNotNull(panel)
                fun row(view: TextView): List<Int> {
                    val layout = checkNotNull(view.layout)
                    val first = (layout.lineCount - view.maxLines).coerceAtLeast(0)
                    val origin = view.top - current.transcriptScroll.scrollY + view.compoundPaddingTop - view.scrollY
                    return listOf(layout.lineCount, view.maxLines, first, origin + layout.getLineTop(first),
                        origin + layout.getLineBottom(layout.lineCount - 1))
                }
                val source = row(current.source); val translation = row(current.translation)
                return Geometry(source[0], source[1], source[2], source[3], source[4],
                    translation[0], translation[1], translation[2], translation[3], translation[4], current.transcriptScroll.height)
            }
            ui {
                manager = context.getSystemService(Context.WINDOW_SERVICE) as WindowManager
                panel = CaptionPanelView(ContextThemeWrapper(context, R.style.Theme_Mimi))
                panel!!.applyStyle(CaptionFrameStyle(16, Color.WHITE, 100, 80, CaptionDisplayMode.BILINGUAL, CaptionAlignment.LEFT))
                manager!!.addView(panel, WindowManager.LayoutParams(dp(350), dp(readingViewportDp) + panel!!.chromeHeight(), WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
                    WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE,
                    PixelFormat.TRANSLUCENT).apply { gravity = Gravity.TOP or Gravity.CENTER_HORIZONTAL; y = dp(100) })
                listener = ViewTreeObserver.OnDrawListener {
                    if (stage >= 0) {
                        draws++; drawnStage = stage
                        val visible = geometry()
                        if (visible.sourceTop < 0 || visible.translationTop < 0 || visible.sourceBottom > visible.viewport ||
                            visible.translationBottom > visible.viewport) drawFailure = "visible_line_clipped_stage_$stage"
                    }
                }
                panel!!.viewTreeObserver.addOnDrawListener(listener)
            }
            val sourceShort = "We are looking at how subtitles behave while speech continues."
            val sourceLong = "$sourceShort The original sentence grows before the translation catches up, then the same sentence is revised once more."
            val sourceLongest = "$sourceLong We should keep the current reading position steady during each revision."
            val translationShort = "我们正在观察字幕如何变化。"
            val translationMedium = "我们正在观察说话继续时字幕如何变化，原文先增长，译文随后跟上。"
            val translationLong = "$translationMedium 同一句话还会修订，阅读位置应保持稳定，不应因为行数变化而反复跳动。"
            val frames = listOf(
                Frame(sourceShort, translationShort),
                Frame(sourceLong, translationShort),
                Frame(sourceLongest, translationShort),
                Frame(sourceLongest, translationMedium),
                Frame(sourceLongest, translationLong),
                Frame(sourceLongest, translationMedium),
                Frame(sourceLongest, translationShort),
                Frame(sourceLongest, translationLong),
                Frame(sourceLongest, translationLong, true),
                Frame("The next sentence starts.", "下一句话开始了。", true),
            )
            var previous: Geometry? = null
            var previousFrame: Frame? = null
            for ((index, frame) in frames.withIndex()) {
                ui {
                    stage = index
                    panel!!.showContent(CaptionFrameContent(frame.source, frame.translation, confirmed = frame.confirmed), CaptionDisplayMode.BILINGUAL)
                    panel!!.invalidate()
                }
                for (attempt in 0 until 60) {
                    var ready = false; ui { ready = drawnStage == index }
                    if (ready) break
                    SystemClock.sleep(20)
                }
                check(drawnStage == index) { "native_probe_draw_timeout_$index" }
                SystemClock.sleep(80)
                ui {
                    val value = geometry()
                    drawFailure?.let { error(it) }
                    previous?.let { prior ->
                        if (previousFrame!!.source == frame.source) {
                            check(value.sourceFirst >= prior.sourceFirst) { "unchanged_source_revealed_older_line_$index" }
                            if (value.sourceLimit != prior.sourceLimit) originalBudgetChanges++
                            if (value.sourceFirst != prior.sourceFirst || value.sourceTop != prior.sourceTop || value.sourceBottom != prior.sourceBottom) unchangedSourceMovements++
                        }
                        if (previousFrame!!.translation == frame.translation && value.translationBottom != prior.translationBottom) unchangedTranslationMovements++
                    }
                    report.put(value.json().put("stage", index))
                    previous = value; previousFrame = frame
                    // Re-publishing an unchanged pair must not reset either native viewport.
                    panel!!.showContent(CaptionFrameContent(frame.source, frame.translation, confirmed = frame.confirmed), CaptionDisplayMode.BILINGUAL)
                }
                SystemClock.sleep(50)
                ui { check(geometry() == previous) { "unchanged_publication_moved_viewport_$index" } }
            }
            check(unchangedTranslationMovements == 0) { "unchanged_translation_baseline_moved" }
            val freshLimit = report.getJSONObject(2).getInt("sourceLimit")
            val constrainedLimit = report.getJSONObject(3).getInt("sourceLimit")
            check(freshLimit > constrainedLimit) { "fixture_did_not_exercise_source_budget" }
            var resetStage = frames.size
            fun settle(action: () -> Unit): Geometry {
                val index = resetStage++
                ui { stage = index; action(); panel!!.requestLayout(); panel!!.invalidate() }
                for (attempt in 0 until 60) {
                    var ready = false; ui { ready = drawnStage == index }
                    if (ready) break
                    SystemClock.sleep(20)
                }
                check(drawnStage == index) { "native_probe_reset_draw_timeout_$index" }
                SystemClock.sleep(80)
                var result: Geometry? = null
                ui { drawFailure?.let { error(it) }; result = geometry() }
                return checkNotNull(result)
            }
            fun publish(translation: String) = panel!!.showContent(
                CaptionFrameContent(sourceLongest, translation), CaptionDisplayMode.BILINGUAL)
            fun constrain() {
                check(settle { publish(translationLong) }.sourceLimit == constrainedLimit)
                check(settle { publish(translationShort) }.sourceLimit == constrainedLimit)
            }
            // A different source starts its own budget instead of inheriting a previous sentence's crop.
            check(settle { publish(translationShort) }.sourceLimit == freshLimit) { "source_change_did_not_reset" }
            constrain()
            val resized = settle {
                val params = panel!!.layoutParams as WindowManager.LayoutParams
                params.height = dp(readingViewportDp + 100) + panel!!.chromeHeight(); manager!!.updateViewLayout(panel, params)
            }
            check(resized.sourceFirst == 0) { "height_resize_did_not_reset" }
            check(settle {
                val params = panel!!.layoutParams as WindowManager.LayoutParams
                params.height = dp(readingViewportDp) + panel!!.chromeHeight(); manager!!.updateViewLayout(panel, params)
            }.sourceLimit == freshLimit) { "height_restore_did_not_reset" }
            constrain()
            check(settle {
                panel!!.applyStyle(CaptionFrameStyle(12, Color.WHITE, 100, 80, CaptionDisplayMode.BILINGUAL, CaptionAlignment.LEFT))
            }.sourceFirst == 0) { "font_change_did_not_reset" }
            check(settle {
                panel!!.applyStyle(CaptionFrameStyle(16, Color.WHITE, 100, 80, CaptionDisplayMode.BILINGUAL, CaptionAlignment.LEFT))
            }.sourceLimit == freshLimit) { "font_restore_did_not_reset" }
            constrain()
            // Mode-only rendering changes also release the bilingual reference budget.
            ui {
                stage = -1
                panel!!.showContent(CaptionFrameContent(sourceLongest, translationShort), CaptionDisplayMode.ORIGINAL)
            }
            SystemClock.sleep(100)
            check(settle { publish(translationShort) }.sourceLimit == freshLimit) { "display_mode_did_not_reset" }
            constrain()
            fun touch(action: Int, y: Float) {
                val now = SystemClock.uptimeMillis()
                val event = MotionEvent.obtain(now, now, action, dp(100).toFloat(), y, 0)
                try { panel!!.transcriptScroll.dispatchTouchEvent(event) } finally { event.recycle() }
            }
            // Manual history reading must expose the full source and clear the live crop.
            ui {
                stage = -1
                touch(MotionEvent.ACTION_DOWN, dp(80).toFloat())
                touch(MotionEvent.ACTION_MOVE, dp(100).toFloat())
                touch(MotionEvent.ACTION_CANCEL, dp(100).toFloat())
            }
            SystemClock.sleep(100)
            ui {
                check(!panel!!.followingLive && panel!!.source.maxLines == Int.MAX_VALUE) { "manual_history_did_not_release_crop" }
                panel!!.transcriptScroll.scrollTo(0, panel!!.transcriptScroll.getChildAt(0).height)
            }
            check(settle {
                touch(MotionEvent.ACTION_DOWN, dp(100).toFloat())
                touch(MotionEvent.ACTION_MOVE, dp(80).toFloat())
                touch(MotionEvent.ACTION_UP, dp(80).toFloat())
            }.sourceLimit == freshLimit) { "return_to_live_did_not_reset" }
            ui { check(panel!!.followingLive) }
            ui { stage = -1 }
            CaptionReadingInputChecks(test, checkNotNull(panel),
                home!!.findViewById(R.id.subtitle_preview), accessibilityReport).run(
                sourceShort, sourceLongest, translationShort, translationLong,
            )
        } catch (error: Throwable) { failure = error }
        finally {
            ui {
                panel?.let { current ->
                    listener?.let { current.viewTreeObserver.removeOnDrawListener(it) }
                    if (current.isAttachedToWindow) manager?.removeViewImmediate(current)
                }
                home?.let { current ->
                    current.window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                    current.setShowWhenLocked(false); current.setTurnScreenOn(false); current.finish()
                }
            }
            if (ownsFixture) check(guide.edit().apply { if (hadGuideSeen) putBoolean("seen", guideSeen) else remove("seen") }.commit())
        }
        test.finish(if (failure == null) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
            putString("stream", "Caption differential probe asr-mt-differential-v2: passed=${failure == null}; draws=$draws; " +
                "originalBudgetChanges=$originalBudgetChanges; unchangedSourceMovements=$unchangedSourceMovements; " +
                "unchangedTranslationMovements=$unchangedTranslationMovements; captureStarted=false; " +
                "failure=${failure?.message ?: "none"}; geometry=$report; accessibility=$accessibilityReport\n")
        })
    }
}
