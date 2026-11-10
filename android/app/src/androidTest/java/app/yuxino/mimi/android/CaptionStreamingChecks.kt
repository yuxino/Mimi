package app.yuxino.mimi.android

import android.app.Activity
import android.app.Instrumentation
import android.content.Intent
import android.os.Bundle
import android.os.SystemClock
import android.view.View
import android.view.ViewTreeObserver
import android.view.inspector.WindowInspector
import android.widget.TextView
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.provider.SharedSubtitleCore
import app.yuxino.mimi.android.provider.SubtitleBus
import org.json.JSONObject

/** Continuous native drawing, including TextView's own scroll reconciliation. No capture/providers. */
internal class CaptionStreamingChecks(private val test: Instrumentation) {
    private val context get() = test.targetContext
    private var stage = "setup"
    private fun ui(action: () -> Unit) {
        var failure: Throwable? = null
        test.runOnMainSync { try { action() } catch (error: Throwable) { failure = error } }
        failure?.let { throw it }
    }
    private fun root(): View? = WindowInspector.getGlobalWindowViews().firstOrNull { it.tag == "mimi-overlay" }
    private fun waitFor(predicate: () -> Boolean) {
        repeat(100) {
            var ready = false; ui { ready = predicate() }
            if (ready) return
            SystemClock.sleep(30)
        }
        error("Streaming layout timeout: $stage")
    }
    private fun rows(): List<TextView> = if (SettingsStore.immersiveSubtitles(context)) {
        val panel = root()!!.findViewWithTag<CompactCaptionView>("compact-subtitle")
        listOf(panel.source, panel.translation)
    } else {
        val panel = root()!!.findViewWithTag<CaptionPanelView>("expanded-subtitles")
        listOf(panel.source, panel.translation)
    }
    private fun assertTail(row: TextView) {
        val layout = row.layout ?: return
        val expected = layout.getLineTop((layout.lineCount - row.maxLines).coerceAtLeast(0))
        check(row.scrollY == expected) { "Native text scroll jumped at $stage: ${row.scrollY} instead of $expected" }
    }
    private fun assertVisibleLines() {
        val panel = root()!!.findViewWithTag<CaptionPanelView>("expanded-subtitles")
        for (row in rows().filter { it.isShown }) {
            assertTail(row)
            val layout = row.layout
            val first = (layout.lineCount - row.maxLines).coerceAtLeast(0)
            val origin = row.top - panel.transcriptScroll.scrollY + row.compoundPaddingTop - row.scrollY
            check(origin + layout.getLineTop(first) >= 0 &&
                origin + layout.getLineBottom(layout.lineCount - 1) <= panel.transcriptScroll.height) {
                "Native visible glyphs clipped at $stage"
            }
        }
    }
    fun run(arguments: Bundle?) {
        check(!MimiService.isRunning)
        check(!SettingsStore.isConfigured(context)) { "Streaming fixtures require blank private configuration" }
        var home: MainActivity? = null
        var failure: Throwable? = null
        var draws = 0
        var updates = 0
        var cases = 0
        var drawFailure: Throwable? = null
        var observer: ViewTreeObserver? = null
        val drawListener = ViewTreeObserver.OnDrawListener {
            draws++
            try {
                rows().filter { it.isShown }.forEach(::assertTail)
                if (!SettingsStore.immersiveSubtitles(context)) assertVisibleLines()
            } catch (error: Throwable) { drawFailure = error }
        }
        try {
            ui {
                check(context.getSharedPreferences("first_run", 0).edit().putBoolean("seen", true).commit())
                SettingsStore.setFontSize(context, 16)
                SettingsStore.setHistoryLines(context, 0)
                SettingsStore.setOverlayHeightPercent(context, 32)
                SettingsStore.setImmersiveSubtitles(context, false)
            }
            home = test.startActivitySync(Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as MainActivity
            context.startService(Intent(context, MimiService::class.java).setAction(MimiService.ACTION_UI_PREVIEW).putExtra("preview_over_own_ui", true))
            waitFor { root() != null }
            val samples = listOf(
                "English captions keep the complete newest lines visible while a long sentence continues. ",
                "简体中文字幕持续增长时应该稳定清晰，并且完整地利用字幕区域。",
                "繁體中文字幕持續增長時應該穩定清晰，並且完整地利用字幕區域。",
                "日本語の字幕が長くなっても、最新の行が安定して読みやすく表示されます。",
                "한국어 자막이 길어져도 최신 문장을 안정적으로 읽을 수 있어야 합니다. ",
                "Les sous-titres restent lisibles pendant la mise à jour d’une longue phrase. ",
                "Längere Untertitel müssen während jeder Aktualisierung stabil lesbar bleiben. ",
                "คำบรรยายที่ยาวขึ้นควรแสดงอย่างมั่นคงและอ่านได้ชัดเจนตลอดเวลา ",
                "Длинные русские субтитры должны оставаться читаемыми при каждом обновлении. ",
            )
            // Match the desktop bottom reading edge. An unchanged translated
            // line must not move down when its reference sentence grows.
            for (mode in CaptionDisplayMode.entries) {
                stage = "reading-edge/$mode"
                ui {
                    SettingsStore.setDisplayMode(context, mode)
                    SubtitleBus.onFinalPair("Short source.", "固定的译文。", "en")
                }
                context.startService(Intent(context, MimiService::class.java).setAction(MimiService.ACTION_APPLY_APPEARANCE))
                SystemClock.sleep(180)
                var readingBottom = 0
                fun lastRowBottom(): Int {
                    val panel = root()!!.findViewWithTag<CaptionPanelView>("expanded-subtitles")
                    val row = if (mode.translationVisible) panel.translation else panel.source
                    val layout = row.layout!!
                    return row.top - panel.transcriptScroll.scrollY + row.compoundPaddingTop - row.scrollY +
                        layout.getLineBottom(layout.lineCount - 1)
                }
                ui {
                    val panel = root()!!.findViewWithTag<CaptionPanelView>("expanded-subtitles")
                    readingBottom = lastRowBottom()
                    check(readingBottom == panel.transcriptScroll.height) { "Short captions left an unused bottom reading edge at $stage" }
                }
                for (length in listOf(2, 4, 8, 1)) {
                    ui { SubtitleBus.onFinalPair(samples[0].repeat(length), "固定的译文。", "en") }
                    SystemClock.sleep(90)
                    ui {
                        check(lastRowBottom() == readingBottom) { "Reference growth moved the reading baseline at $stage" }
                        assertVisibleLines()
                    }
                }
                cases++
            }
            for (immersive in listOf(false, true)) for (mode in CaptionDisplayMode.entries) {
                stage = "$immersive/$mode"
                ui { SettingsStore.setImmersiveSubtitles(context, immersive); SettingsStore.setDisplayMode(context, mode) }
                context.startService(Intent(context, MimiService::class.java).setAction(MimiService.ACTION_APPLY_APPEARANCE))
                SystemClock.sleep(180)
                for (atomic in listOf(false, true)) {
                    stage = "handoff/$immersive/$mode/atomic-$atomic"
                    var response = if (atomic) SharedSubtitleCore.exchange(JSONObject().put("operation",
                        JSONObject().put("type", "create").put("history_limit", 0))) else null
                    fun publish(event: JSONObject) {
                        response = SharedSubtitleCore.exchange(JSONObject().put("state", response!!.getString("state"))
                            .put("operation", JSONObject().put("type", "apply").put("event", event)))
                        SubtitleBus.onRuntimeSnapshot(JSONObject().put("atomicPreview", true).put("detectedLanguage", "en")
                            .put("snapshot", response!!.getJSONObject("snapshot")), false)
                    }
                    ui {
                        SubtitleBus.clear()
                        if (atomic) {
                            publish(JSONObject().put("type", "final_pair").put("source", "Synthetic complete A.").put("translation", "完整的合成 A。"))
                            publish(JSONObject().put("type", "source_draft").put("text", "Synthetic pending B."))
                        } else {
                            SubtitleBus.onFinalPair("Synthetic complete A.", "完整的合成 A。", "en")
                            SubtitleBus.onSourceDraft("Synthetic pending B.", "en")
                        }
                    }
                    SystemClock.sleep(180)
                    ui {
                        val current = rows()
                        check(current[0].text.toString() == if (mode == CaptionDisplayMode.ORIGINAL)
                            "Synthetic pending B." else "Synthetic complete A.") { "Reading handoff changed sentence owner at $stage" }
                        if (mode.translationVisible) {
                            check(current[1].isShown && current[1].text.toString() == "完整的合成 A。") { "Pending translation flashed a placeholder at $stage" }
                        }
                        if (atomic) publish(JSONObject().put("type", "translation_draft").put("text", "新的合成 B。"))
                        else SubtitleBus.onTranslationDraft("新的合成 B。")
                    }
                    SystemClock.sleep(180)
                    if (atomic) {
                        ui {
                            check(rows()[0].text.toString() == if (mode == CaptionDisplayMode.ORIGINAL)
                                "Synthetic pending B." else "Synthetic complete A.") { "Atomic translation lost its complete reading pair at $stage" }
                            if (mode.translationVisible) check(rows()[1].text.toString() == "完整的合成 A。")
                            publish(JSONObject().put("type", "preview_pair").put("source_utterance_id", JSONObject.NULL)
                                .put("source", "Synthetic pending B.").put("translation", "新的合成 B。"))
                        }
                        SystemClock.sleep(180)
                    }
                    ui {
                        check(rows()[0].text.toString() == "Synthetic pending B.") { "Completed next translation failed to advance at $stage" }
                        check(rows()[1].text.toString() == "新的合成 B。")
                        SubtitleBus.clear()
                    }
                    cases++
                }
                for ((index, sample) in samples.withIndex()) {
                    stage = "$immersive/$mode/language-$index"
                    val translated = samples[(index + 1) % samples.size]
                    ui { SubtitleBus.onFinalPair(sample.repeat(5), translated.repeat(5), "en") }
                    SystemClock.sleep(120)
                    // TextView registers this reconciliation after text/size changes.
                    // It must never undo tail-following or require cancelling a window draw.
                    ui {
                        for (row in rows().filter { it.isShown }) { row.onPreDraw(); assertTail(row) }
                        check(!root()!!.viewTreeObserver.dispatchOnPreDraw()) { "Native frame drawing cancelled at $stage" }
                    }
                    val height = root()!!.height
                    val beforeDraws = draws
                    ui { observer = root()!!.viewTreeObserver; observer!!.addOnDrawListener(drawListener) }
                    repeat(24) { update ->
                        ui {
                            SubtitleBus.onFinalPair(sample.repeat(4 + update / 4) + " $update", translated.repeat(4 + update / 3) + " $update", "en")
                        }
                        updates++
                        SystemClock.sleep(45)
                        drawFailure?.let { throw it }
                    }
                    SystemClock.sleep(120)
                    ui { observer!!.removeOnDrawListener(drawListener); observer = null }
                    check(draws > beforeDraws + 8) { "Continuous caption updates starved native drawing at $stage" }
                    if (!immersive) check(root()!!.height == height) { "Long content changed the saved panel height at $stage" }
                    cases++
                }
                test.sendStatus(0, Bundle().apply { putString("stream", "STREAMING_PROGRESS immersive=$immersive mode=$mode cases=$cases updates=$updates draws=$draws\n") })
            }
            if (arguments?.getString("probe_only") != "true") {
                for (mode in CaptionDisplayMode.entries) {
                    stage = "height-return/$mode"
                    ui { SettingsStore.setImmersiveSubtitles(context, false); SettingsStore.setDisplayMode(context, mode) }
                    context.startService(Intent(context, MimiService::class.java).setAction(MimiService.ACTION_APPLY_APPEARANCE))
                    SystemClock.sleep(200)
                    for (height in listOf(20, 50, 70, 32)) {
                        ui { SettingsStore.setOverlayHeightPercent(context, height) }
                        context.startService(Intent(context, MimiService::class.java).setAction(MimiService.ACTION_APPLY_APPEARANCE))
                        SystemClock.sleep(200)
                        ui { rows().filter { it.isShown }.forEach(::assertTail) }
                        check(SettingsStore.overlayHeightPercent(context) == height)
                    }
                    cases++
                }
                if (arguments?.getString("skip_rotation") != "true") {
                    for (mode in CaptionDisplayMode.entries) for (height in listOf(20, 32, 70)) {
                        stage = "rotation/$mode/$height"
                        check(test.uiAutomation.setRotation(android.app.UiAutomation.ROTATION_FREEZE_0))
                        ui {
                            SettingsStore.setImmersiveSubtitles(context, false)
                            SettingsStore.setDisplayMode(context, mode)
                            SettingsStore.setOverlayHeightPercent(context, height)
                            SubtitleBus.onFinalPair(samples[0].repeat(8), samples[1].repeat(8), "en")
                        }
                        context.startService(Intent(context, MimiService::class.java).setAction(MimiService.ACTION_APPLY_APPEARANCE))
                        SystemClock.sleep(350)
                        val before = root()!!.width to root()!!.height
                        val savedWidth = SettingsStore.overlayWidthPercent(context)
                        val resize = root()!!.findViewWithTag<View>("overlay-resize")
                        val touchTime = SystemClock.uptimeMillis()
                        fun resizeTouch(action: Int, delta: Float) = ui {
                            val event = android.view.MotionEvent.obtain(touchTime, SystemClock.uptimeMillis(), action,
                                resize.width / 2f + delta, resize.height / 2f + delta, 0)
                            try { resize.dispatchTouchEvent(event) } finally { event.recycle() }
                        }
                        resizeTouch(android.view.MotionEvent.ACTION_DOWN, 0f)
                        resizeTouch(android.view.MotionEvent.ACTION_MOVE, 55f)
                        check(test.uiAutomation.setRotation(android.app.UiAutomation.ROTATION_FREEZE_90))
                        waitFor { root()!!.width != before.first }
                        SystemClock.sleep(350)
                        resizeTouch(android.view.MotionEvent.ACTION_MOVE, 90f)
                        resizeTouch(android.view.MotionEvent.ACTION_UP, 90f)
                        ui { assertVisibleLines() }
                        check(SettingsStore.overlayHeightPercent(context) == height && SettingsStore.overlayWidthPercent(context) == savedWidth) {
                            "Rotation overwrote saved dimensions at $stage"
                        }
                        check(test.uiAutomation.setRotation(android.app.UiAutomation.ROTATION_FREEZE_0))
                        waitFor { root()!!.width == before.first && root()!!.height == before.second }
                        ui { assertVisibleLines() }
                        check(root()!!.findViewWithTag<View>("overlay-font").isClickable) { "Cancelled resize opened the quick editor at $stage" }
                        cases++
                    }
                }
            }
        } catch (error: Throwable) { failure = error }
        finally {
            test.uiAutomation.setRotation(android.app.UiAutomation.ROTATION_UNFREEZE)
            ui { observer?.takeIf { it.isAlive }?.removeOnDrawListener(drawListener) }
            context.startService(Intent(context, MimiService::class.java).setAction(MimiService.ACTION_STOP))
            SystemClock.sleep(150)
            ui { SubtitleBus.clear(); home?.finish() }
        }
        test.finish(if (failure == null) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
            putString("stream", if (failure == null) "Caption streaming passed: $cases cases, $updates updates, $draws native draws; 9 languages, 3 modes, normal/immersive; no capture/providers.\n"
                else "Caption streaming failed at $stage: ${failure!!.stackTraceToString()}\n")
        })
    }
}
