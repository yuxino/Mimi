package app.yuxino.mimi.android

import android.app.Activity
import android.app.Instrumentation
import android.content.Intent
import android.graphics.Bitmap
import android.os.Bundle
import android.os.SystemClock
import android.view.MotionEvent
import android.view.View
import android.view.inspector.WindowInspector
import android.widget.SeekBar
import android.widget.TextView
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.provider.SubtitleBus
import java.io.File
import kotlin.math.abs

/** Comparable native Before/After fixtures; no capture, credentials or provider calls. */
internal class OverlayInteractionChecks(private val test: Instrumentation) {
    private val context get() = test.targetContext
    private fun root() = WindowInspector.getGlobalWindowViews().first { it.tag == "mimi-overlay" }
    private fun caption() = root().findViewWithTag<TextView>("expanded-translation")
    private fun onUi(action: () -> Unit) {
        var failure: Throwable? = null
        test.runOnMainSync { try { action() } catch (error: Throwable) { failure = error } }
        failure?.let { throw it }
        test.waitForIdleSync()
    }
    private fun waitFor(predicate: () -> Boolean) {
        repeat(60) {
            var ready = false
            onUi { ready = predicate() }
            if (ready) return
            SystemClock.sleep(100)
        }
        error("Overlay interaction did not settle")
    }
    private fun capture(name: String) {
        test.waitForIdleSync()
        SystemClock.sleep(350) // Wait for WindowManager/compositor bounds after overlay resizing.
        val bitmap = checkNotNull(test.uiAutomation.takeScreenshot())
        File(checkNotNull(context.getExternalFilesDir("ui-preview")), "interaction-$name.png").outputStream().use {
            bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)
        }
        bitmap.recycle()
    }
    private fun drag(seek: SeekBar, value: Int) {
        onUi {
            val x = seek.paddingLeft + (seek.width - seek.paddingLeft - seek.paddingRight) *
                (value - seek.min).toFloat() / (seek.max - seek.min)
            val time = SystemClock.uptimeMillis()
            for (action in listOf(MotionEvent.ACTION_DOWN, MotionEvent.ACTION_MOVE, MotionEvent.ACTION_UP)) {
                val event = MotionEvent.obtain(time, SystemClock.uptimeMillis(), action, x, seek.height / 2f, 0)
                seek.dispatchTouchEvent(event)
                event.recycle()
            }
        }
    }

    /** Run only on a blank idle device; uses the existing synthetic overlay. */
    private fun immersivePlacementChecks(home: MainActivity) {
        val before = root()
        onUi {
            SettingsStore.setFontSize(context, 12)
            SettingsStore.setOverlayYOffset(context, 48)
            check(context.getSharedPreferences("first_run", 0).edit().putBoolean("immersive_seen", true).commit())
            root().findViewWithTag<View>("collapse-overlay").performClick()
        }
        waitFor { root().findViewWithTag<View>("compact-subtitle").isShown }
        fun bottom(): Int {
            var value = 0
            onUi { val origin = IntArray(2); root().getLocationOnScreen(origin); value = origin[1] + root().height }
            return value
        }
        val anchor = bottom()
        onUi { home.findViewById<View>(R.id.home_immersive).performClick() }
        waitFor { WindowInspector.getGlobalWindowViews().any { it.tag == "exit-immersive" } }
        onUi {
            check(home.findViewById<TextView>(R.id.home_immersive).text.toString() == home.getString(R.string.overlay_exit_immersive))
            home.findViewById<View>(R.id.home_immersive).performClick()
        }
        waitFor { WindowInspector.getGlobalWindowViews().none { it.tag == "exit-immersive" } }
        check(!root().findViewWithTag<View>("expanded-subtitles").isShown)
        repeat(3) {
            onUi { root().findViewWithTag<View>("compact-subtitle").performClick() }
            waitFor { root().findViewWithTag<View>("expanded-subtitles").isShown }
            onUi { root().findViewWithTag<View>("enter-immersive").performClick() }
            waitFor { WindowInspector.getGlobalWindowViews().any { it.tag == "exit-immersive" } }
            check(root() === before) { "Mode toggle replaced the caption window" }
            check(abs(bottom() - anchor) <= 2) { "Immersive changed the compact bottom anchor" }
            onUi { WindowInspector.getGlobalWindowViews().first { it.tag == "exit-immersive" }.performClick() }
            waitFor { root().findViewWithTag<View>("expanded-subtitles").isShown }
            onUi { root().findViewWithTag<View>("collapse-overlay").performClick() }
            waitFor { root().findViewWithTag<View>("compact-subtitle").isShown }
            check(abs(bottom() - anchor) <= 2)
            check(SettingsStore.overlayYOffset(context) == 48)
        }
        onUi {
            SettingsStore.setImmersiveSubtitles(context, true)
            SettingsStore.setFontSize(context, 24)
            SubtitleBus.onFinalPair("Synthetic source with several words.", "合成字幕用于内容高度变化检查。", "en")
        }
        waitFor { WindowInspector.getGlobalWindowViews().any { it.tag == "exit-immersive" } }
        check(abs(bottom() - anchor) <= 2) { "Font/content reflow changed the bottom anchor" }
        onUi { SettingsStore.setImmersiveSubtitles(context, false) }
        waitFor { WindowInspector.getGlobalWindowViews().none { it.tag == "exit-immersive" } }
        check(!root().findViewWithTag<View>("expanded-subtitles").isShown)
        check(SettingsStore.overlayYOffset(context) == 48)
    }

    fun run(arguments: Bundle?) {
        val reviewSeconds = arguments?.getString("review_hold_seconds")?.toLongOrNull()?.takeIf { it in 1..180 }
        val baseline = arguments?.getString("baseline") == "true"
        val prefix = if (baseline) "before" else "after"
        val source = SettingsStore.sourceLang(context)
        val target = SettingsStore.targetLang(context)
        val historyLines = SettingsStore.historyLines(context)
        val position = SettingsStore.overlayYOffset(context)
        val font = SettingsStore.fontSize(context)
        val immersive = SettingsStore.immersiveSubtitles(context)
        val color = SettingsStore.translationColorIndex(context)
        val background = SettingsStore.overlayBgAlpha(context)
        val opacity = SettingsStore.overlayOpacity(context)
        val prefs = context.getSharedPreferences("first_run", 0)
        val hadSeen = prefs.contains("seen")
        val seen = prefs.getBoolean("seen", false)
        val hadImmersiveSeen = prefs.contains("immersive_seen")
        val immersiveSeen = prefs.getBoolean("immersive_seen", false)
        var home: MainActivity? = null
        var settings: SettingsActivity? = null
        var failure: Throwable? = null
        try {
            check(!MimiService.isRunning && !SettingsStore.isConfigured(context)) { "Use a blank idle emulator" }
            SettingsStore.setFontSize(context, 16)
            SettingsStore.setImmersiveSubtitles(context, false)
            SettingsStore.setTranslationColorIndex(context, 1)
            SettingsStore.setOverlayBgAlpha(context, 65)
            SettingsStore.setOverlayOpacity(context, 100)
            check(prefs.edit().putBoolean("seen", true).commit())
            home = test.startActivitySync(Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as MainActivity
            context.startService(Intent(context, MimiService::class.java).setAction(MimiService.ACTION_UI_PREVIEW))
            waitFor { WindowInspector.getGlobalWindowViews().any { it.tag == "mimi-overlay" } }
            onUi { SubtitleBus.clear() }
            waitFor { root().isShown == baseline }
            capture("$prefix-empty")
            onUi {
                SubtitleBus.setHistoryLimit(2)
                SubtitleBus.onFinalPair("もう少し歩いてみましょう。", "再往前走一会儿吧。", "ja")
                SubtitleBus.onSourceDraft("もう少し歩いてみましょう。", "ja")
                SubtitleBus.onTranslationDraft("再往前走一会儿吧。")
            }
            waitFor { root().isShown }
            if (reviewSeconds != null) {
                test.sendStatus(0, Bundle().apply { putString("stream", "READY_FOR_MANUAL_OVERLAY_REVIEW: synthetic subtitles; no capture, credentials or provider.\n") })
                SystemClock.sleep(reviewSeconds * 1000)
            } else {
                capture("$prefix-compact")
                onUi { root().findViewWithTag<View>("compact-subtitle").performClick() }
                waitFor { root().findViewWithTag<View>("expanded-subtitles").isShown }
                val history = SubtitleBus.historySnapshot()
                check(history.isNotEmpty())
                capture("$prefix-expanded")
                onUi { root().findViewWithTag<View>("overlay-font").performClick() }
                if (baseline) {
                    check(SettingsStore.fontSize(context) == 18)
                } else {
                    waitFor { WindowInspector.getGlobalWindowViews().any { it.findViewWithTag<SeekBar>("overlay-font-slider") != null } }
                    capture("$prefix-font-control")
                    onUi { root().findViewWithTag<View>("overlay-font").performClick() }
                    check(WindowInspector.getGlobalWindowViews().count { it.findViewWithTag<SeekBar>("overlay-font-slider") != null } == 1)
                    val slider = WindowInspector.getGlobalWindowViews().firstNotNullOf { it.findViewWithTag<SeekBar>("overlay-font-slider") }
                    drag(slider, 18)
                    waitFor { SettingsStore.fontSize(context) == 18 && abs(caption().textSize - 21 * context.resources.displayMetrics.scaledDensity) < 1 }
                    onUi { check(root().findViewWithTag<TextView>("overlay-font").text.toString().endsWith("18")) }
                    onUi { WindowInspector.getGlobalWindowViews().first { it.findViewWithTag<SeekBar>("overlay-font-slider") != null }.findViewById<View>(android.R.id.button1).performClick() }
                }
                settings = test.startActivitySync(Intent(context, SettingsActivity::class.java)
                    .putExtra("settings_section", "appearance").addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as SettingsActivity
                val seek = settings.findViewById<SeekBar>(R.id.font_size)
                drag(seek, 22)
                waitFor { SettingsStore.fontSize(context) == 22 }
                if (!baseline) waitFor { abs(caption().textSize - 25 * context.resources.displayMetrics.scaledDensity) < 1 }
                onUi {
                    val currentSize = caption().textSize / context.resources.displayMetrics.scaledDensity
                    check(abs(currentSize - (if (baseline) 21 else 25)) < 1) { "Unexpected native caption size" }
                    check(SubtitleBus.historySnapshot() == history)
                }
                capture("$prefix-settings-size")
                onUi { settings.finish() }
                if (!baseline) {
                    immersivePlacementChecks(checkNotNull(home))
                    // Confirmed pairs intentionally remain readable. Test the genuinely empty state.
                    onUi { SubtitleBus.clear(); SubtitleBus.hideLive() }
                    waitFor { !root().isShown }
                }
            }
            check(!MimiService.isRunning)
        } catch (error: Throwable) { failure = error }
        finally {
            context.stopService(Intent(context, MimiService::class.java))
            onUi { settings?.finish(); home?.finish() }
            SettingsStore.setSourceLang(context, source)
            SettingsStore.setTargetLang(context, target)
            SettingsStore.setHistoryLines(context, historyLines)
            SettingsStore.setOverlayYOffset(context, position)
            SettingsStore.setFontSize(context, font)
            SettingsStore.setImmersiveSubtitles(context, immersive)
            SettingsStore.setTranslationColorIndex(context, color)
            SettingsStore.setOverlayBgAlpha(context, background)
            SettingsStore.setOverlayOpacity(context, opacity)
            val restore = prefs.edit()
            if (hadSeen) restore.putBoolean("seen", seen) else restore.remove("seen")
            if (hadImmersiveSeen) restore.putBoolean("immersive_seen", immersiveSeen) else restore.remove("immersive_seen")
            check(restore.commit())
            check(SettingsStore.flushPendingWritesForTests(context))
        }
        test.finish(if (failure == null) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
            putString("stream", if (failure == null && reviewSeconds != null) "Manual overlay review fixture closed; preferences restored; no capture or provider.\n"
                else if (failure == null) "Overlay $prefix checks passed: empty window, font action, live settings, history and silence; synthetic captions, no capture or provider.\n"
                else "Overlay $prefix failed: ${failure.javaClass.simpleName}: ${failure.message}\n")
        })
    }
}
