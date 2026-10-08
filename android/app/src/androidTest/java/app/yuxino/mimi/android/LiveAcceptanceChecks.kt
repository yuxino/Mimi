package app.yuxino.mimi.android

import android.app.Activity
import android.app.Instrumentation
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.os.Bundle
import android.os.SystemClock
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.provider.SubtitleBus
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import android.view.View
import android.widget.Spinner
import android.widget.TextView
import android.view.inspector.WindowInspector
import android.graphics.Bitmap
import java.util.concurrent.atomic.AtomicInteger

/** Explicit saved-profile acceptance. The operator starts each fresh native consent flow. */
internal class LiveAcceptanceChecks(private val test: Instrumentation) {
    private val context get() = test.targetContext
    private fun status(value: String) = test.sendStatus(0, Bundle().apply { putString("stream", "$value\n") })
    fun run(arguments: Bundle?) {
        val seconds = arguments?.getString("observe_seconds")?.toLongOrNull()?.coerceIn(30, 180) ?: 75
        val rounds = arguments?.getString("rounds")?.toIntOrNull()?.coerceIn(1, 3) ?: 2
        val events = JSONArray()
        val results = JSONArray()
        val started = SystemClock.elapsedRealtime()
        var failure: String? = null
        var stage = "preconditions"
        var home: MainActivity? = null
        var temporary: TemporaryProviderAcceptance? = null
        var finalListener: SubtitleBus.Listener? = null
        try {
            check(context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0)
            check(!MimiService.isRunning && SettingsStore.isConfigured(context))
            check(SettingsStore.historyLines(context) == 0) { "history_must_be_off" }
            if (arguments?.getString("temporary_provider") == "true") {
                stage = "temporary_provider"
                temporary = TemporaryProviderAcceptance(context)
                temporary.apply()
            }
            val configuration = SettingsStore.configuration(context)
            home = test.startActivitySync(Intent(context, MainActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as MainActivity
            for (round in 1..rounds) {
                stage = "await_native_start_$round"
                status("LIVE_READY round=$round provider=${configuration.provider.wireProvider} model=${SettingsStore.qwenMtModel(context)} source=${SettingsStore.sourceLang(context)} target=${SettingsStore.targetLang(context)}; use native start and fresh platform consent; no credential values logged.")
                val deadline = SystemClock.elapsedRealtime() + 90_000
                while (!MimiService.isRunning && SystemClock.elapsedRealtime() < deadline) SystemClock.sleep(100)
                check(MimiService.isRunning) { "native_start_timeout" }
                val activeModels = MimiService.activeModelNames.toList()
                check(activeModels == runtimeModelNames(SettingsStore.runtimeConfiguration(context)))
                if (arguments?.getString("exercise_model_guard") == "true") {
                    stage = "model_guard_$round"
                    val savedModel = SettingsStore.qwenMtModel(context)
                    val editor = test.startActivitySync(Intent(context, ServiceSettingsActivity::class.java)
                        .putExtra("provider", configuration.provider.id).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as ServiceSettingsActivity
                    try {
                        test.runOnMainSync {
                            val spinner = editor.findViewById<View>(android.R.id.content).findViewWithTag<Spinner>("qwen-mt-model")
                            check(!spinner.isEnabled && !editor.findViewById<View>(R.id.save).isEnabled)
                            spinner.setSelection(if (savedModel == "plus") 0 else 2)
                            // Exercise the defensive handler too, including stale/programmatic clicks.
                            editor.findViewById<View>(R.id.save).performClick()
                        }
                        check(SettingsStore.qwenMtModel(context) == savedModel && MimiService.isRunning)
                        check(MimiService.activeModelNames == activeModels)
                    } finally { test.runOnMainSync { editor.finish() } }
                    status("LIVE_MODEL_GUARD_PASSED round=$round model=$savedModel; disabled editor and stale save preserve active model; resume Bilibili playback if paused.")
                }
                stage = "observe_$round"
                val begin = SystemClock.elapsedRealtime()
                var sourceUpdates = 0
                var translationUpdates = 0
                val finals = AtomicInteger()
                val finalLock = Any()
                var lastFinal: Pair<Int, Int>? = null
                finalListener = object : SubtitleBus.Listener {
                    override fun onSubtitleChanged() = synchronized(finalLock) {
                        val pair = if (SubtitleBus.displayPairFinal) Pair(
                            SubtitleBus.displaySource.hashCode(), SubtitleBus.displayTranslation.hashCode(),
                        ) else null
                        if (pair != null && pair != lastFinal) finals.incrementAndGet()
                        lastFinal = pair
                    }
                }
                // A final snapshot and Stop acknowledgement can share one UI
                // tick. Observe native publication, not a lucky 100 ms sample.
                SubtitleBus.addListener(checkNotNull(finalListener))
                var previousSource = 0
                var previousTranslation = 0
                var previousFinal = false
                var previousStatus = false
                var previousCapture = ""
                var lastSourceAt = begin
                var largestSourceGap = 0L
                var modelInspected = false
                fun observe() {
                    val now = SystemClock.elapsedRealtime()
                    // Hashes are transient comparison state only; no text/hash is logged or saved.
                    val source = SubtitleBus.displaySource
                    val translation = SubtitleBus.displayTranslation
                    val sourceHash = source.hashCode()
                    val translationHash = translation.hashCode()
                    val final = SubtitleBus.displayPairFinal
                    val statusShown = SubtitleBus.statusLine.isNotEmpty()
                    val capture = MimiService.captureObservation?.state?.name ?: "WAITING"
                    val feedback = SubtitleBus.runtimeFeedback
                    val sourceChanged = source.isNotEmpty() && sourceHash != previousSource
                    val translationChanged = translation.isNotEmpty() && translationHash != previousTranslation
                    if (sourceChanged) {
                        sourceUpdates++
                        largestSourceGap = maxOf(largestSourceGap, now - lastSourceAt)
                        lastSourceAt = now
                    }
                    if (translationChanged) translationUpdates++
                    if (events.length() < 2000 && (sourceChanged || translationChanged || final != previousFinal ||
                            statusShown != previousStatus || capture != previousCapture)) {
                        events.put(JSONObject().put("round", round).put("elapsedMs", now - started)
                            .put("sourceChanged", sourceChanged).put("translationChanged", translationChanged)
                            .put("sourceChars", source.length).put("translationChars", translation.length)
                            .put("pairedFinal", final).put("sourceDraft", SubtitleBus.sourceDraft.isNotEmpty())
                            .put("translationDraft", SubtitleBus.translationDraft.isNotEmpty())
                            .put("statusShown", statusShown).put("capture", capture).put("running", MimiService.isRunning)
                            .put("connectionStatus", feedback.connectionStatus).put("recoveryReason", feedback.recoveryReason ?: JSONObject.NULL)
                            .put("retryScheduled", feedback.retryScheduled).put("translationTimedOut", feedback.translationTimedOut))
                    }
                    previousSource = sourceHash; previousTranslation = translationHash
                    previousFinal = final; previousStatus = statusShown; previousCapture = capture
                }
                while (SystemClock.elapsedRealtime() - begin < seconds * 1000 && MimiService.isRunning) {
                    test.runOnMainSync { observe() }
                    if (!modelInspected && activeModels.isNotEmpty() && translationUpdates > 0 && SystemClock.elapsedRealtime() - begin > 10_000) {
                        stage = "active_model_display_$round"
                        test.runOnMainSync {
                            val root = WindowInspector.getGlobalWindowViews().first { it.tag == "mimi-overlay" }
                            root.findViewWithTag<View>("compact-subtitle").performClick()
                        }
                        test.waitForIdleSync()
                        test.runOnMainSync {
                            val root = WindowInspector.getGlobalWindowViews().first { it.tag == "mimi-overlay" }
                            val label = root.findViewWithTag<TextView>("overlay-model")
                            check(label.isShown && label.text.toString() == modelNamesLabel(activeModels))
                            check(label.layout.height <= label.height - label.paddingTop - label.paddingBottom)
                        }
                        SystemClock.sleep(120)
                        val bitmap = checkNotNull(test.uiAutomation.takeScreenshot())
                        File(context.getExternalFilesDir(null), "live-model.png").outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
                        bitmap.recycle()
                        modelInspected = true; stage = "observe_$round"
                        status("LIVE_MODEL_DISPLAY_PASSED round=$round; native expanded model label and screenshot verified.")
                    }
                    SystemClock.sleep(100)
                }
                check(MimiService.isRunning) { "session_ended_during_observation" }
                stage = "stop_$round"
                status("LIVE_STOP_REQUEST round=$round")
                val stop = SystemClock.elapsedRealtime()
                context.startService(MimiService.stopIntent(context))
                while (MimiService.isRunning && SystemClock.elapsedRealtime() - stop < 12_000) {
                    test.runOnMainSync { observe() }; SystemClock.sleep(100)
                }
                check(!MimiService.isRunning) { "stop_timeout" }
                results.put(JSONObject().put("round", round).put("observedMs", stop - begin)
                    .put("provider", configuration.provider.wireProvider).put("models", JSONArray(activeModels))
                    .put("sourceUpdates", sourceUpdates).put("translationUpdates", translationUpdates)
                    .put("pairedFinalObservations", finals.get()).put("largestSourceUpdateGapMs", largestSourceGap)
                    .put("stopMs", SystemClock.elapsedRealtime() - stop))
                SubtitleBus.removeListener(checkNotNull(finalListener)); finalListener = null
                check(sourceUpdates > 0 && translationUpdates > 0 && finals.get() > 0) { "missing_subtitle_stages" }
                SystemClock.sleep(1500)
                check(!MimiService.isRunning) { "session_restarted_after_stop" }
            }
        } catch (_: Exception) { failure = stage }
        finally {
            finalListener?.let(SubtitleBus::removeListener)
            context.startService(MimiService.stopIntent(context))
            val deadline = SystemClock.elapsedRealtime() + 12_000
            while (MimiService.isRunning && SystemClock.elapsedRealtime() < deadline) SystemClock.sleep(100)
            test.runOnMainSync { home?.finish() }
            try { check(!MimiService.isRunning); temporary?.close() }
            catch (_: Exception) { failure = "restore_saved_configuration" }
        }
        val report = JSONObject().put("passed", failure == null).put("failureStage", failure ?: JSONObject.NULL)
            .put("capture", "system_playback_only").put("savedCredentialsRestored", failure != "restore_saved_configuration")
            .put("historyEnabled", false).put("rounds", results).put("events", events)
            .put("timingScope", "Android adapter observations; not cross-platform latency or rendered frame timing")
        File(context.getExternalFilesDir(null), "live-acceptance.json").writeText(report.toString(2), Charsets.UTF_8)
        test.finish(if (failure == null) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
            putString("stream", "LIVE_RESULT passed=${failure == null}; failureStage=${failure ?: "none"}; $results; content-free observations saved.\n")
        })
    }
}
