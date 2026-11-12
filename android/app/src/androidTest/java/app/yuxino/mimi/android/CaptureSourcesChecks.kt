package app.yuxino.mimi.android

import android.Manifest
import android.app.Activity
import android.app.Instrumentation
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.Bundle
import android.os.SystemClock
import android.provider.Settings
import androidx.core.content.ContextCompat
import app.yuxino.mimi.android.capture.CaptureInput
import app.yuxino.mimi.android.capture.CaptureSource
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.provider.ProviderEngine
import app.yuxino.mimi.android.provider.SharedSubtitleCore
import app.yuxino.mimi.android.provider.SubtitleBus
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong

/** Explicit emulator-only capture check. Discards all PCM; never creates a provider or reads credentials. */
internal class CaptureSourcesChecks(private val instrumentation: Instrumentation) {
    private val context get() = instrumentation.targetContext
    private var stage = "initial"

    fun run(arguments: Bundle?) {
        var activity: CaptureProbeActivity? = null
        var failure: String? = null
        var ownedSession = false
        val results = JSONArray()
        try {
            check(arguments?.getString("host_microphone_disabled") == "true") { "disable_emulator_host_microphone_first" }
            check(Build.HARDWARE in setOf("ranchu", "goldfish")) { "emulator_only" }
            check(!MimiService.isRunning && !MimiService.isStopping) { "active_session" }
            check(Settings.canDrawOverlays(context)) { "overlay_permission_required" }
            check(ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED)
            val host = instrumentation.startActivitySync(Intent(context, CaptureProbeActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as CaptureProbeActivity
            activity = host
            val consent = PlaybackCaptureChecks(instrumentation)
            consent.verifyEmulatorConsentMode()
            stage = "rejected_start_completion"
            onMain {
                MimiService.setCaptureEnginesForTests(context, null)
                val request = MimiService.prepareStart(host)
                check(MimiService.isStarting && MimiService.startPending(request))
                ownedSession = true
                MimiService.startFromActivity(host, Intent(host, MimiService::class.java)
                    .setAction(MimiService.ACTION_CAPTURE_TEST).putExtra(MimiService.EXTRA_START_REQUEST, request))
            }
            await { !MimiService.isStarting }
            check(!MimiService.isRunning && MimiService.lastCaptureError == "capture.start_failed")
            results.put(JSONObject().put("rejectedStartUnlocked", true))
            val inputs = listOf(CaptureInput.MICROPHONE, CaptureInput.BOTH, CaptureInput.BOTH)
            for ((index, input) in inputs.withIndex()) {
                stage = "${input.id}_$index"
                val engines = input.sources.associateWith { DiscardEngine(if (it == CaptureSource.SYSTEM) 16_000 else 24_000) }
                val oldRequests = host.requests
                onMain {
                    MimiService.setCaptureEnginesForTests(context, engines)
                    ownedSession = true
                    if (input.needsProjection) host.requestProjection(input, guarded = true)
                    else MimiService.startFromActivity(host, MimiService.microphoneIntent(host)
                        .setAction(MimiService.ACTION_CAPTURE_TEST)
                        .putExtra(MimiService.EXTRA_START_REQUEST, MimiService.prepareStart(host)))
                }
                if (input.needsProjection) consent.respondToEmulatorConsent(true)
                await { MimiService.isRunning && engines.values.all { it.chunks.get() >= 5 } }
                check(!MimiService.isStarting)
                check(MimiService.captureInput == input)
                check(MimiService.captureObservations.keys == input.sources.toSet())
                check(host.requests - oldRequests == if (input.needsProjection) 1 else 0)
                check((MimiService.captureProjectionForTests != null) == input.needsProjection)
                val types = MimiService.captureForegroundTypesForTests
                check((types and ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION != 0) == input.needsProjection)
                if (Build.VERSION.SDK_INT >= 30) check(types and ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE != 0)
                // Emulator dummy input may contain nonzero samples with host input
                // disabled. Count packets only; this is lifecycle, not acoustic proof.
                check(engines.values.none { it.invalid.get() })
                if (input == CaptureInput.BOTH) onMain { checkSourceCaptions(stopped = false) }
                val revoke = index == 2
                onMain {
                    if (revoke) checkNotNull(MimiService.captureProjectionForTests).stop()
                    else context.startService(MimiService.stopIntent(context))
                }
                await { !MimiService.isRunning && !MimiService.isStopping && engines.values.all { it.stopped.get() } }
                check(MimiService.captureObservations.isEmpty())
                check(MimiService.captureProjectionForTests == null)
                check(MimiService.captureForegroundTypesForTests == 0)
                if (input == CaptureInput.BOTH) onMain { checkSourceCaptions(stopped = true) }
                if (revoke) check(MimiService.lastCaptureError == "capture.projection_stopped")
                else check(engines.values.all { it.finished.get() })
                val stoppedCounts = engines.values.map { it.chunks.get() }
                SystemClock.sleep(350)
                check(stoppedCounts == engines.values.map { it.chunks.get() }) { "pcm_after_stop" }
                results.put(JSONObject().put("input", input.id).put("projectionRequested", input.needsProjection)
                    .put("foregroundTypes", types).put("revoked", revoke).put("independentEngines", engines.size)
                    .put("packets", JSONArray(stoppedCounts))
                    .put("microphoneNonzeroPackets", engines.getValue(CaptureSource.MICROPHONE).nonzero.get()))
            }
        } catch (error: Throwable) {
            failure = "$stage:${error.javaClass.simpleName}:${error.message?.takeIf { it.matches(Regex("[a-z_]+")) }.orEmpty()}"
        } finally {
            if (ownedSession) runCatching {
                onMain { context.startService(MimiService.stopIntent(context)) }
                await { !MimiService.isRunning && !MimiService.isStopping }
                onMain { MimiService.setCaptureEnginesForTests(context, null); SubtitleBus.clear() }
            }.onFailure { failure = failure ?: "cleanup_capture" }
            runCatching { onMain { activity?.finish() } }.onFailure { failure = failure ?: "cleanup_activity" }
        }
        val result = JSONObject().put("passed", failure == null).put("failure", failure ?: JSONObject.NULL)
            .put("androidApi", Build.VERSION.SDK_INT).put("device", Build.MODEL).put("hostMicrophone", false)
            .put("providerStarted", false).put("credentialsRead", false).put("pcmSaved", false).put("checks", results)
        instrumentation.finish(if (failure == null) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
            putString("stream", "CAPTURE_SOURCES_RESULT $result\n")
        })
    }

    private fun checkSourceCaptions(stopped: Boolean) {
        if (!stopped) {
            SubtitleBus.setHistoryLimit(2)
            for (source in CaptureSource.entries) {
                val initial = SharedSubtitleCore.exchange(JSONObject().put("operation", JSONObject()
                    .put("type", "create").put("history_limit", 2)))
                val final = SharedSubtitleCore.exchange(JSONObject().put("state", initial.getString("state"))
                    .put("operation", JSONObject().put("type", "apply").put("event", JSONObject()
                        .put("type", "identified_final_pair").put("utterance_id", "same-id")
                        .put("source", "Synthetic ${source.id} source").put("translation", "Synthetic ${source.id} translation"))))
                SubtitleBus.onRuntimeSnapshot(JSONObject().put("snapshot", final.getJSONObject("snapshot")), false, source.id)
            }
        }
        val frame = CaptionFrameContent.live(context, CaptionDisplayMode.BILINGUAL, 2, null, !stopped, false)
        for (source in CaptureSource.entries) {
            check(frame.original.contains("Synthetic ${source.id} source"))
            check(frame.translated.contains("Synthetic ${source.id} translation"))
            val label = InterfaceLanguage.context(context).getString(if (source == CaptureSource.SYSTEM)
                R.string.capture_input_system else R.string.capture_input_microphone)
            check(frame.original.contains(label) && frame.translated.contains(label))
        }
        check(frame.confirmed && SubtitleBus.hasRetainedContent)
        if (stopped) check(frame.status.isEmpty())
    }

    private fun onMain(action: () -> Unit) {
        var failure: Throwable? = null
        instrumentation.runOnMainSync { try { action() } catch (error: Throwable) { failure = error } }
        failure?.let { throw it }
    }
    private fun await(condition: () -> Boolean) {
        val deadline = SystemClock.elapsedRealtime() + 15_000
        while (!condition()) {
            check(SystemClock.elapsedRealtime() < deadline) { "capture_check_timeout" }
            SystemClock.sleep(50)
        }
    }
    private class DiscardEngine(override val sampleRateHz: Int) : ProviderEngine {
        val chunks = AtomicLong()
        val nonzero = AtomicLong()
        val invalid = AtomicBoolean()
        val stopped = AtomicBoolean()
        val finished = AtomicBoolean()
        override fun start() =
            error("provider_start_forbidden")
        override fun sendAudio(pcm16Mono: ByteArray) {
            if (stopped.get() || pcm16Mono.size % 2 != 0 || pcm16Mono.size > sampleRateHz * 2 / 5) invalid.set(true)
            chunks.incrementAndGet()
            if (pcm16Mono.any { it != 0.toByte() }) nonzero.incrementAndGet()
        }
        override fun finish(onFinished: () -> Unit) { finished.set(true); onFinished() }
        override fun stop() { stopped.set(true) }
    }
}
