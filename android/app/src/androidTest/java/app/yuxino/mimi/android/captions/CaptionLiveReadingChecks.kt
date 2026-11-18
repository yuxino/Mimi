package app.yuxino.mimi.android.captions

import android.app.Activity
import android.app.Instrumentation
import android.app.UiAutomation
import android.content.Intent
import android.content.res.Configuration
import android.graphics.Rect
import android.os.Bundle
import android.os.SystemClock
import android.view.View
import android.view.ViewTreeObserver
import android.view.inspector.WindowInspector
import app.yuxino.mimi.android.MainActivity
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.provider.SubtitleBus
import app.yuxino.mimi.android.services.acceptance.TemporaryProviderAcceptance
import app.yuxino.mimi.android.services.presentation.runtimeModelNames
import app.yuxino.mimi.android.settings.SettingsStore
import java.io.File
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicReference
import org.json.JSONArray
import org.json.JSONObject

/** Explicit operator-started saved-profile playback; diagnostics never contain caption text. */
internal class CaptionLiveReadingChecks(private val test: Instrumentation) {
    private val context get() = test.targetContext
    private fun ui(action: () -> Unit) {
        var failure: Throwable? = null
        test.runOnMainSync { try { action() } catch (error: Throwable) { failure = error } }
        failure?.let { throw it }
    }
    private fun status(value: String) = test.sendStatus(0, Bundle().apply { putString("stream", "$value\n") })
    private fun root(): View? = WindowInspector.getGlobalWindowViews().firstOrNull { it.tag == "mimi-overlay" }
    fun run(arguments: Bundle?) {
        check(!MimiService.isRunning)
        val mode = SettingsStore.displayMode(context)
        val immersed = SettingsStore.immersiveSubtitles(context)
        val originalSource = SettingsStore.sourceLang(context)
        val originalTarget = SettingsStore.targetLang(context)
        val originalWidth = SettingsStore.overlayWidthPercent(context)
        val originalHeight = SettingsStore.overlayHeightPercent(context)
        val screenChanges = arguments?.getString("screen_changes") == "true"
        val browserFullscreen = arguments?.getString("browser_fullscreen") == "true"
        check(!browserFullscreen || screenChanges)
        val seconds = arguments?.getString("phase_seconds")?.toIntOrNull()
        check(seconds == null || seconds in 30..300)
        check(!screenChanges || seconds != null && seconds >= 90)
        val phases = if (seconds == null) listOf(CaptionDisplayMode.BILINGUAL to 90, CaptionDisplayMode.TRANSLATION to 60,
            CaptionDisplayMode.ORIGINAL to 60, CaptionDisplayMode.BILINGUAL to 60)
            else listOf(CaptionDisplayMode.BILINGUAL to seconds, CaptionDisplayMode.TRANSLATION to seconds,
                CaptionDisplayMode.ORIGINAL to seconds)
        val label = arguments?.getString("result_label") ?: "default"
        check(label.matches(Regex("[a-z0-9-]{1,40}")))
        var provider = SettingsStore.provider(context)
        var models = emptyList<String>()
        var source = originalSource
        var target = originalTarget
        var temporary: TemporaryProviderAcceptance? = null
        val results = JSONArray()
        var failure: Throwable? = null
        val drawFailure = AtomicReference<String?>()
        val draws = AtomicInteger()
        val captionDraws = AtomicInteger()
        val handoffDraws = AtomicInteger()
        val observedMode = AtomicReference<CaptionDisplayMode?>()
        val settling = AtomicBoolean(false)
        var lastSize: Pair<Int, Int>? = null
        val changes = AtomicInteger()
        var observer: ViewTreeObserver? = null
        val listener = ViewTreeObserver.OnDrawListener {
            draws.incrementAndGet()
            val current = root()
            if (current != null) {
                val panel = current.findViewWithTag<CaptionPanelView>("expanded-subtitles")
                val compact = current.findViewWithTag<CompactCaptionView>("compact-subtitle")
                val visibleRows = if (panel.isShown) listOf(panel.source, panel.translation)
                    else listOf(compact.source, compact.translation)
                if (!settling.get() && (compact.isShown || panel.isShown && panel.followingLive)) {
                    if (visibleRows.any { it.isShown && it.text.isNotBlank() }) {
                        captionDraws.incrementAndGet()
                    }
                    val snapshot = SubtitleBus.displaySnapshot()
                    if (observedMode.get()?.translationVisible == true && !snapshot.hidden &&
                        snapshot.retainedConfirmed?.pair?.translation?.isNotBlank() == true) {
                        if (snapshot.translation.isBlank()) handoffDraws.incrementAndGet()
                        if (!visibleRows[1].isShown) drawFailure.set("confirmed_translation_replaced_by_placeholder")
                    }
                    if (observedMode.get() == CaptionDisplayMode.ORIGINAL && !snapshot.hidden &&
                        (snapshot.source.isNotBlank() || snapshot.retainedConfirmed?.pair?.source?.isNotBlank() == true)) {
                        if (snapshot.currentSource.text.isBlank()) handoffDraws.incrementAndGet()
                        if (!visibleRows[0].isShown) drawFailure.set("retained_original_replaced_by_placeholder")
                    }
                    for (row in visibleRows.filter { it.isShown }) {
                        val layout = row.layout ?: continue
                        val first = (layout.lineCount - row.maxLines).coerceAtLeast(0)
                        val tail = layout.getLineTop(first)
                        if (row.scrollY != tail) drawFailure.set("native_tail_reset")
                        val visible = Rect()
                        // Visible rectangles use root-view coordinates, not
                        // screen coordinates; floating windows have an offset.
                        val location = IntArray(2).also(row::getLocationInWindow)
                        val rootLocation = IntArray(2).also(row.rootView::getLocationInWindow)
                        val origin = location[1] - rootLocation[1] + row.compoundPaddingTop - row.scrollY
                        if (!row.getGlobalVisibleRect(visible) || origin + layout.getLineTop(first) < visible.top ||
                            origin + layout.getLineBottom(layout.lineCount - 1) > visible.bottom) {
                            drawFailure.compareAndSet(null, "native_glyph_clipping row=${row.tag} " +
                                "mode=${observedMode.get()?.id} visible=${visible.top},${visible.bottom} " +
                                "glyphs=${origin + layout.getLineTop(first)},${origin + layout.getLineBottom(layout.lineCount - 1)} " +
                                "lines=${layout.lineCount} limit=${row.maxLines} height=${row.height} scroll=${row.scrollY}")
                        }
                    }
                    val size = current.width to current.height
                    // Immersion intentionally grows with its bounded live lines.
                    if (panel.isShown) {
                        if (lastSize?.first == size.first && lastSize?.second != size.second) changes.incrementAndGet()
                        lastSize = size
                    }
                }
            }
        }
        try {
            if (arguments?.getString("temporary_provider") == "true") {
                temporary = TemporaryProviderAcceptance(context)
                temporary.apply()
            }
            check(SettingsStore.isConfigured(context))
            arguments?.getString("test_target")?.let { requested ->
                check(requested in listOf("ja", "ko", "th"))
                val selected = SettingsStore.configuration(context).provider
                check(selected.supportsPair("en", requested, SettingsStore.textTranslationProvider(context)))
                SettingsStore.setSourceLang(context, "en"); SettingsStore.setTargetLang(context, requested)
            }
            provider = SettingsStore.provider(context)
            source = SettingsStore.sourceLang(context); target = SettingsStore.targetLang(context)
            models = runtimeModelNames(SettingsStore.runtimeConfiguration(context))
            test.startActivitySync(Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            status("CAPTION_LIVE_READY provider=$provider models=${models.joinToString(",")} source=$source target=$target; start native playback capture with fresh consent, then resume the same video; recording/history preferences retained.")
            val deadline = SystemClock.elapsedRealtime() + 180000
            while ((!MimiService.isRunning || root() == null) && SystemClock.elapsedRealtime() < deadline) SystemClock.sleep(100)
            check(MimiService.isRunning && root() != null) { "native_start_timeout" }
            check(MimiService.activeModelNames == models) { "active_model_mismatch" }
            // Starting capture and sending a media key do not prove that the
            // browser resumed playback. Do not score a paused-input interval.
            val audioDeadline = SystemClock.elapsedRealtime() + 30000
            while (MimiService.isRunning && MimiService.captureObservation?.state?.name != "AUDIO" &&
                SystemClock.elapsedRealtime() < audioDeadline) SystemClock.sleep(100)
            check(MimiService.isRunning && MimiService.captureObservation?.state?.name == "AUDIO") { "playback_audio_start_timeout" }
            // Leave the operator's native consent/browser inspector free until
            // playback starts; only one UiAutomation connection can own it.
            if (screenChanges && !browserFullscreen) check(test.uiAutomation.setRotation(UiAutomation.ROTATION_FREEZE_0))
            ui { observer = root()!!.viewTreeObserver; observer!!.addOnDrawListener(listener) }
            for ((selected, seconds) in phases) {
                observedMode.set(null)
                ui { SettingsStore.setImmersiveSubtitles(context, false); SettingsStore.setDisplayMode(context, selected) }
                SystemClock.sleep(600)
                ui { lastSize = null; changes.set(0) }
                observedMode.set(selected)
                val beforeDraws = draws.get()
                val beforeCaptionDraws = captionDraws.get()
                val beforeHandoffs = handoffDraws.get()
                var sourceUpdates = 0; var translationUpdates = 0
                var sourceHash = 0; var translationHash = 0
                var maximumSource = 0; var maximumTranslation = 0
                var audioSamples = 0
                var maximumSourceGapMs = 0L
                var firstSourceMs: Long? = null; var firstTranslationMs: Long? = null
                var confirmedUpdates = 0
                var lastConfirmedAt: Long? = SubtitleBus.displaySnapshot().createdAt
                var firstConfirmedMs: Long? = null
                var maximumConfirmedGapMs = 0L
                val start = SystemClock.elapsedRealtime()
                var lastConfirmedUpdateAt = start
                var lastSourceAt = start
                var transitions = 0
                val initialFrame = root()!!.width to root()!!.height
                val initialSurface = uiSurface()
                fun transition(name: String, change: () -> Unit) {
                    settling.set(true)
                    change()
                    // Rotation publishes configuration before the native window
                    // catches up. Require stable actual frame geometry, then
                    // sample live drawing again in the new layout.
                    var stable = 0
                    var previous: Pair<Int, Int>? = null
                    val deadline = SystemClock.elapsedRealtime() + 5000
                    while (SystemClock.elapsedRealtime() < deadline) {
                        val frame = root()?.let { it.width to it.height }
                        if (frame != null && frame.first > 0 && frame.second > 0 && frame == previous) stable++ else stable = 0
                        previous = frame
                        if (stable >= 6) break
                        SystemClock.sleep(50)
                    }
                    check(stable >= 6) { "screen_geometry_not_settled" }
                    ui {
                        check(uiSurface() == initialSurface) { "screen_change_altered_caption_surface" }
                        lastSize = null
                    }
                    settling.set(false)
                    transitions++
                    status("CAPTION_SCREEN_CHANGED mode=${selected.id} action=$name")
                }
                val feedbackEvents = JSONArray()
                var lastFeedback: SubtitleBus.RuntimeFeedback? = null
                status("CAPTION_LIVE_PHASE mode=${selected.id} seconds=$seconds")
                var progress = start + 30000
                while (SystemClock.elapsedRealtime() - start < seconds * 1000) {
                    check(MimiService.isRunning) { "capture_stopped" }
                    drawFailure.get()?.let { error(it) }
                    val raw = SubtitleBus.displaySnapshot()
                    val state = raw.forReading(selected)
                    val a = state.source.hashCode(); val b = state.translation.hashCode()
                    val now = SystemClock.elapsedRealtime()
                    if (screenChanges) {
                        val elapsed = now - start
                        when {
                            transitions == 0 && elapsed >= 8000 -> transition("landscape") {
                                if (browserFullscreen) status("CAPTION_FULLSCREEN_REQUEST mode=${selected.id} action=enter")
                                else check(test.uiAutomation.setRotation(UiAutomation.ROTATION_FREEZE_90))
                                awaitOrientation(Configuration.ORIENTATION_LANDSCAPE)
                            }
                            transitions == 1 && elapsed >= 22000 -> transition("immersive") {
                                ui { SettingsStore.setImmersiveSubtitles(context, true) }
                            }
                            transitions == 2 && elapsed >= 38000 -> transition("normal-small") {
                                ui { SettingsStore.setImmersiveSubtitles(context, false); SettingsStore.setOverlayHeightPercent(context, 20) }
                            }
                            transitions == 3 && elapsed >= 52000 -> transition("normal-large") {
                                ui { SettingsStore.setOverlayHeightPercent(context, 70); SettingsStore.setOverlayWidthPercent(context, 70) }
                            }
                            transitions == 4 && elapsed >= 66000 -> transition("portrait-restored") {
                                ui { SettingsStore.setOverlayHeightPercent(context, originalHeight); SettingsStore.setOverlayWidthPercent(context, originalWidth) }
                                if (browserFullscreen) status("CAPTION_FULLSCREEN_REQUEST mode=${selected.id} action=exit")
                                else check(test.uiAutomation.setRotation(UiAutomation.ROTATION_FREEZE_0))
                                awaitOrientation(Configuration.ORIENTATION_PORTRAIT)
                            }
                        }
                    }
                    if (raw.confirmed && raw.createdAt != null && raw.createdAt != lastConfirmedAt) {
                        confirmedUpdates++
                        if (firstConfirmedMs == null) firstConfirmedMs = now - start
                        maximumConfirmedGapMs = maxOf(maximumConfirmedGapMs, now - lastConfirmedUpdateAt)
                        lastConfirmedUpdateAt = now
                        lastConfirmedAt = raw.createdAt
                    }
                    if (state.source.isNotEmpty() && a != sourceHash) {
                        sourceUpdates++
                        if (firstSourceMs == null) firstSourceMs = now - start
                        maximumSourceGapMs = maxOf(maximumSourceGapMs, now - lastSourceAt)
                        lastSourceAt = now
                    }
                    if (state.translation.isNotEmpty() && b != translationHash) {
                        translationUpdates++
                        if (firstTranslationMs == null) firstTranslationMs = now - start
                    }
                    if (MimiService.captureObservation?.state?.name == "AUDIO") audioSamples++
                    if (state.feedback != lastFeedback && feedbackEvents.length() < 100) {
                        feedbackEvents.put(JSONObject().put("elapsedMs", now - start)
                            .put("connectionStatus", state.feedback.connectionStatus)
                            .put("recoveryReason", state.feedback.recoveryReason ?: JSONObject.NULL)
                            .put("retryScheduled", state.feedback.retryScheduled)
                            .put("translationTimedOut", state.feedback.translationTimedOut))
                        lastFeedback = state.feedback
                    }
                    sourceHash = a; translationHash = b
                    maximumSource = maxOf(maximumSource, state.source.length)
                    maximumTranslation = maxOf(maximumTranslation, state.translation.length)
                    if (SystemClock.elapsedRealtime() >= progress) {
                        status("CAPTION_LIVE_PROGRESS mode=${selected.id} sourceUpdates=$sourceUpdates translationUpdates=$translationUpdates confirmedUpdates=$confirmedUpdates nativeDraws=${draws.get() - beforeDraws} heightChanges=${changes.get()} capture=${MimiService.captureObservation?.state?.name}")
                        progress += 30000
                    }
                    SystemClock.sleep(50)
                }
                results.put(JSONObject().put("mode", selected.id).put("seconds", seconds).put("sourceUpdates", sourceUpdates)
                    .put("translationUpdates", translationUpdates).put("nativeDraws", draws.get() - beforeDraws)
                    .put("maximumSourceChars", maximumSource).put("maximumTranslationChars", maximumTranslation).put("heightChanges", changes.get())
                    .put("readingHandoffDraws", handoffDraws.get() - beforeHandoffs)
                    .put("captionDraws", captionDraws.get() - beforeCaptionDraws)
                    .put("audioSamples", audioSamples).put("firstSourceMs", firstSourceMs ?: JSONObject.NULL)
                    .put("firstTranslationMs", firstTranslationMs ?: JSONObject.NULL).put("maximumSourceGapMs", maximumSourceGapMs)
                    .put("confirmedUpdates", confirmedUpdates).put("firstConfirmedMs", firstConfirmedMs ?: JSONObject.NULL)
                    .put("maximumConfirmedGapMs", maximumConfirmedGapMs)
                    .put("screenTransitions", transitions)
                    .put("connectionEvents", feedbackEvents))
                check(sourceUpdates >= 3 && translationUpdates >= 3) { "insufficient_live_updates" }
                check(audioSamples > 0) { "no_playback_audio" }
                check(captionDraws.get() - beforeCaptionDraws >= 3) { "no_visible_native_captions" }
                check(changes.get() == 0) { "content_changed_window_height" }
                if (screenChanges) {
                    check(transitions == 5 && root()!!.width == initialFrame.first && root()!!.height == initialFrame.second) {
                        "portrait_frame_not_restored"
                    }
                    check(SettingsStore.overlayHeightPercent(context) == originalHeight &&
                        SettingsStore.overlayWidthPercent(context) == originalWidth) { "screen_changes_overwrote_saved_dimensions" }
                }
                status("CAPTION_LIVE_PHASE_PASSED mode=${selected.id}")
            }
        } catch (error: Throwable) { failure = error }
        finally {
            ui {
                settling.set(true)
                observer?.takeIf { it.isAlive }?.removeOnDrawListener(listener)
                SettingsStore.setDisplayMode(context, mode); SettingsStore.setImmersiveSubtitles(context, immersed)
                SettingsStore.setOverlayWidthPercent(context, originalWidth); SettingsStore.setOverlayHeightPercent(context, originalHeight)
            }
            if (screenChanges && !browserFullscreen) test.uiAutomation.setRotation(UiAutomation.ROTATION_UNFREEZE)
            context.startService(Intent(context, MimiService::class.java).setAction(MimiService.ACTION_STOP))
            repeat(100) { if (MimiService.isRunning || MimiService.isStopping) SystemClock.sleep(100) }
            temporary?.close()
            SettingsStore.setSourceLang(context, originalSource); SettingsStore.setTargetLang(context, originalTarget)
            check(SettingsStore.flushPendingWritesForTests(context))
            File(context.getExternalFilesDir(null), "caption-live-reading-$label.json").writeText(JSONObject()
                .put("provider", provider).put("models", JSONArray(models)).put("source", source).put("target", target).put("phases", results)
                .put("passed", failure == null).put("failure", failure?.message ?: JSONObject.NULL).toString(2))
        }
        test.finish(if (failure == null) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
            putString("stream", if (failure == null) "Caption live reading passed: ${phases.sumOf { it.second }} seconds across all three display modes; no native tail reset, glyph clipping or content-driven height changes.\n"
                else "Caption live reading failed: ${failure!!.stackTraceToString()}\n")
        })
    }
    private fun uiSurface(): Pair<Int, Float> {
        val panel = root()!!.findViewWithTag<CaptionPanelView>("expanded-subtitles")
        return (panel.background as android.graphics.drawable.GradientDrawable).color!!.defaultColor to panel.alpha
    }
    private fun awaitOrientation(expected: Int) {
        repeat(100) {
            if (context.resources.configuration.orientation == expected) return
            SystemClock.sleep(50)
        }
        error("screen_orientation_not_applied")
    }
}
