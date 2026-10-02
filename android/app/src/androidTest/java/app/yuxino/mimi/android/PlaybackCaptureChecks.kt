package app.yuxino.mimi.android

import android.Manifest
import android.app.Activity
import android.app.Instrumentation
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Rect
import android.content.pm.PackageManager
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioTrack
import android.os.Build
import android.os.Bundle
import android.os.SystemClock
import android.provider.Settings
import android.view.inspector.WindowInspector
import android.view.View
import android.view.ViewGroup
import android.view.Choreographer
import android.view.accessibility.AccessibilityNodeInfo
import android.accessibilityservice.AccessibilityServiceInfo
import android.widget.TextView
import androidx.core.content.ContextCompat
import app.yuxino.mimi.android.capture.CaptureHealth
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.provider.ProviderEngine
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.util.concurrent.FutureTask
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import kotlin.concurrent.thread
import kotlin.math.sin

/** Real playback capture with synthetic media, a discard sink and fresh system consent each time. */
internal class PlaybackCaptureChecks(private val instrumentation: Instrumentation) {
    private val targetContext get() = instrumentation.targetContext
    private val observations = JSONArray()
    private var stage = "initial"
    private var consentTimeoutMs = 60_000L
    private var emulatorConsent = false

    fun run(arguments: Bundle?) {
        status("init")
        consentTimeoutMs = arguments?.getString("consent_timeout_ms")?.toLongOrNull()?.coerceIn(10_000, 90_000) ?: 60_000
        emulatorConsent = arguments?.getString("emulator_consent") == "true"
        var activity: CaptureProbeActivity? = null
        var media: SyntheticMedia? = null
        var failure: String? = null
        var probeStarted = false
        val audio = targetContext.getSystemService(AudioManager::class.java)
        val oldVolume = audio.getStreamVolume(AudioManager.STREAM_MUSIC)
        try {
            check(targetContext.applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE != 0)
            check(!MimiService.isRunning) { "active_session" }
            if (emulatorConsent) verifyEmulatorConsentMode()
            check(Settings.canDrawOverlays(targetContext)) { "overlay_permission_required" }
            check(ContextCompat.checkSelfPermission(targetContext, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) {
                "playback_audio_permission_required"
            }
            audio.setStreamVolume(AudioManager.STREAM_MUSIC, audio.getStreamMaxVolume(AudioManager.STREAM_MUSIC), 0)
            status("init_activity_before")
            activity = startActivitySync(Intent(targetContext, CaptureProbeActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as CaptureProbeActivity
            status("init_activity_ready")
            media = SyntheticMedia(AudioAttributes.ALLOW_CAPTURE_BY_ALL)

            val first = DiscardEngine()
            probeStarted = true
            request(activity, first, "allowed-tone", accept = true)
            await("tone_nonzero_pcm") { first.nonzero.get() > 0 && MimiService.captureObservation?.state == CaptureHealth.State.AUDIO }
            checkpoint("allowed_tone_audio", first)
            synchronized(MimiService.firstRunEvidence) {
                check(!MimiService.firstRunEvidence.complete) { "no_caption_must_not_complete_guide" }
            }
            media.silent = true
            await("zero_pcm_silent") { first.zero.get() > 0 && MimiService.captureObservation?.state == CaptureHealth.State.SILENT }
            checkNoSoundHint(expected = true)
            checkpoint("zero_media_silent", first)
            media.silent = false
            await("tone_recovers") { MimiService.captureObservation?.state == CaptureHealth.State.AUDIO }
            checkNoSoundHint(expected = false)
            checkpoint("tone_recovers_audio", first)
            stopAndVerify(first, "normal_stop")

            media.silent = true
            val second = DiscardEngine()
            request(activity, second, "fresh-consent-silent", accept = true)
            check(MimiService.captureObservation?.state == CaptureHealth.State.WAITING) { "new_session_reused_observation" }
            await("fresh_session_silent") { second.zero.get() > 0 && MimiService.captureObservation?.state == CaptureHealth.State.SILENT }
            check(second.nonzero.get() == 0L) { "new_session_reused_audio" }
            checkpoint("fresh_session_zero_only", second)
            media.close()
            media = SyntheticMedia(AudioAttributes.ALLOW_CAPTURE_BY_NONE)
            val optOutZeros = second.zero.get()
            SystemClock.sleep(7_000)
            await("source_opt_out_silent") { MimiService.captureObservation?.state == CaptureHealth.State.SILENT && second.zero.get() > optOutZeros + 20 }
            check(media.framesPlayed > 48_000 * 5) { "opt_out_media_not_playing" }
            check(second.nonzero.get() == 0L) { "source_opt_out_captured" }
            checkpoint("opt_out_tone_zero_only", second)
            stage = "projection_revoke"
            runOnMainSync { checkNotNull(MimiService.captureProjectionForTests).stop() }
            verifyStopped(second, "projection_revoke")
            check(MimiService.lastCaptureError == "capture.projection_stopped") { "projection_stop_callback_missing" }
            checkpoint("projection_revoked_cleanup", second)

            media.close()
            media = SyntheticMedia(AudioAttributes.ALLOW_CAPTURE_BY_ALL)
            val third = DiscardEngine()
            request(activity, third, "fresh-consent-after-revoke", accept = true)
            await("restart_after_revoke_audio") { third.nonzero.get() > 0 && MimiService.captureObservation?.state == CaptureHealth.State.AUDIO }
            checkpoint("restart_after_revoke_audio", third)
            stopAndVerify(third, "final_stop")
            val cancelled = DiscardEngine()
            request(activity, cancelled, "cancel-consent", accept = false)
            check(cancelled.chunks.get() == 0L && !MimiService.isRunning) { "cancel_started_capture" }
            checkpoint("cancel_no_capture", cancelled)
            check(activity.requests == 4) { "fresh_consent_request_count" }
        } catch (error: Throwable) {
            failure = "$stage:${error.javaClass.simpleName}"
        } finally {
            runCatching { media?.close() }.onFailure { failure = failure ?: "cleanup_media" }
            if (probeStarted) {
                runCatching {
                    targetContext.startService(MimiService.stopIntent(targetContext))
                    await("teardown", 5_000) { !MimiService.isRunning }
                }.onFailure { failure = failure ?: "cleanup_capture" }
                runCatching { runOnMainSync { MimiService.setCaptureEngineForTests(targetContext, null) } }
                    .onFailure { failure = failure ?: "cleanup_probe" }
            }
            runCatching { runOnMainSync { activity?.finish() } }.onFailure { failure = failure ?: "cleanup_activity" }
            runCatching { audio.setStreamVolume(AudioManager.STREAM_MUSIC, oldVolume, 0) }
                .onFailure { failure = failure ?: "cleanup_volume" }
        }
        val report = JSONObject().put("androidApi", Build.VERSION.SDK_INT)
            .put("device", Build.MODEL).put("source", "real_audio_playback_capture")
            .put("media", "synthetic_tone_and_zero_pcm").put("microphone", false)
            .put("consentMode", if (emulatorConsent) "emulator_ui" else "manual")
            .put("providerStarted", false).put("credentialsRead", false)
            .put("projectionRevocation", "MediaProjection.stop_callback")
            .put("passed", failure == null).put("failure", failure ?: JSONObject.NULL)
            .put("observations", observations)
        File(targetContext.getExternalFilesDir(null), "capture-probe").apply { mkdirs() }
            .resolve("results.json").writeText(report.toString(2))
        instrumentation.finish(if (failure == null) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
            putString("stream", "PLAYBACK_CAPTURE_RESULT ${report}\n")
        })
    }

    private fun runOnMainSync(action: () -> Unit) {
        var error: Throwable? = null
        instrumentation.runOnMainSync { try { action() } catch (caught: Throwable) { error = caught } }
        error?.let { throw it }
    }
    private fun startActivitySync(intent: Intent): Activity = instrumentation.startActivitySync(intent)
    private fun sendStatus(code: Int, data: Bundle) = instrumentation.sendStatus(code, data)
    private fun status(label: String) {
        stage = label
        sendStatus(0, Bundle().apply { putString("stream", "PLAYBACK_CAPTURE_STAGE $label\n") })
    }

    private fun checkNoSoundHint(expected: Boolean) {
        val text = targetContext.getString(R.string.capture_no_sound_hint)
        fun contains(view: View): Boolean = (view is TextView && view.isAttachedToWindow && view.isShown &&
            !view.isLayoutRequested && view.width > 0 && view.height > 0 && view.text.toString() == text &&
            view.getGlobalVisibleRect(Rect())) ||
            (view is ViewGroup && (0 until view.childCount).any { contains(view.getChildAt(it)) })
        await("no_sound_hint_$expected") {
            var shown = false
            runOnMainSync { shown = WindowInspector.getGlobalWindowViews().filter { it.tag == "mimi-overlay" }.any { contains(it) } }
            shown == expected
        }
    }

    private fun request(activity: CaptureProbeActivity, sink: DiscardEngine, label: String, accept: Boolean) {
        stage = label
        runOnMainSync { MimiService.setCaptureEngineForTests(targetContext, sink); activity.requestProjection() }
        sendStatus(0, Bundle().apply { putString("stream", "PROJECTION_CONSENT_REQUEST $label expected=${if (accept) "approve-entire-screen" else "cancel"}\n") })
        if (emulatorConsent) respondToEmulatorConsent(accept)
        await("consent_$label", consentTimeoutMs) { activity.consentResult != null }
        check(activity.consentResult == if (accept) Activity.RESULT_OK else Activity.RESULT_CANCELED) { "unexpected_consent_result" }
        if (accept) await("running_$label", 10_000) { MimiService.isRunning }
    }

    /** Opt-in UI consent automation never supports physical devices or bypasses a fresh platform prompt. */
    private fun verifyEmulatorConsentMode() {
        check(Build.HARDWARE in setOf("ranchu", "goldfish")) { "emulator_consent_requires_emulator_hardware" }
        status("init_automation_before")
        val automation = instrumentation.uiAutomation
        status("init_automation_connected")
        status("init_qemu_before")
        val qemu = android.os.ParcelFileDescriptor.AutoCloseInputStream(
            automation.executeShellCommand("getprop ro.kernel.qemu")
        ).use { input ->
            // A property needs one line, not shell EOF. Bound even a broken pipe's wait.
            val read = FutureTask<String?> { input.bufferedReader().readLine()?.trim() }
            val reader = thread(name = "mimi-emulator-property", isDaemon = true) { read.run() }
            try { read.get(5, TimeUnit.SECONDS) } finally {
                input.close()
                read.cancel(true)
                reader.join(1_000)
                check(!reader.isAlive) { "emulator_property_worker_retained" }
            }
        }
        check(qemu == "1") { "emulator_consent_requires_qemu_flag" }
        status("init_qemu_verified")
        val info = automation.serviceInfo
        info.flags = info.flags or AccessibilityServiceInfo.FLAG_REPORT_VIEW_IDS
        automation.serviceInfo = info
        status("init_automation_ready")
    }

    private fun respondToEmulatorConsent(accept: Boolean) {
        val automation = instrumentation.uiAutomation
        fun dialog(): AccessibilityNodeInfo? = automation.rootInActiveWindow?.takeIf { root ->
            root.packageName?.toString() == "com.android.systemui" &&
                findNode(root) { it.text?.toString()?.contains("mimi", ignoreCase = true) == true } != null &&
                findNode(root) { it.viewIdResourceName?.endsWith("/screen_share_mode_spinner") == true } != null
        }
        await("visible_platform_consent", consentTimeoutMs) { dialog() != null }
        if (!accept) {
            awaitClick("platform_cancel") {
                dialog()?.let { root -> findNode(root) { it.text?.toString().equals("Cancel", ignoreCase = true) } }
            }
            return
        }
        // Read and operate the current native controls; never retain coordinates or reuse a projection token.
        awaitClick("platform_mode_spinner") {
            dialog()?.let { root -> findNode(root) { it.viewIdResourceName?.endsWith("/screen_share_mode_spinner") == true } }
        }
        awaitClick("platform_entire_screen") {
            automation.rootInActiveWindow?.takeIf { it.packageName?.toString() in setOf("com.android.systemui", "android") }
                ?.let { root -> findNode(root) { it.text?.toString().equals("Entire screen", ignoreCase = true) } }
        }
        await("platform_entire_screen_selected") {
            dialog()?.let { root -> findNode(root) { it.text?.toString().equals("Entire screen", ignoreCase = true) } } != null
        }
        awaitClick("platform_start") {
            dialog()?.let { root -> findNode(root) {
                it.text?.toString() in setOf("Start", "Start now", "Share screen")
            } }
        }
    }

    private fun findNode(root: AccessibilityNodeInfo, predicate: (AccessibilityNodeInfo) -> Boolean): AccessibilityNodeInfo? {
        if (predicate(root)) return root
        for (index in 0 until root.childCount) {
            val child = root.getChild(index) ?: continue
            findNode(child, predicate)?.let { return it }
        }
        return null
    }

    private fun awaitClick(label: String, control: () -> AccessibilityNodeInfo?) {
        await(label, consentTimeoutMs) {
            var node = control()
            var attempts = 0
            while (node != null && !node.isClickable && attempts++ < 5) node = node.parent
            node?.isEnabled == true && node.performAction(AccessibilityNodeInfo.ACTION_CLICK)
        }
    }

    private fun stopAndVerify(sink: DiscardEngine, label: String) {
        stage = label
        targetContext.startService(MimiService.stopIntent(targetContext))
        verifyStopped(sink, label)
        checkpoint(label, sink)
    }

    private fun verifyStopped(sink: DiscardEngine, label: String) {
        await("stopped_$label", 5_000) { !MimiService.isRunning && sink.stopped.get() }
        check(MimiService.captureObservation == null && MimiService.captureProjectionForTests == null) { "capture_state_retained" }
        synchronized(MimiService.firstRunEvidence) {
            check(!MimiService.firstRunEvidence.complete && !MimiService.firstRunEvidence.audioSubmitted) { "capture_evidence_retained" }
        }
        runOnMainSync { check(WindowInspector.getGlobalWindowViews().none { it.tag == "mimi-overlay" || it.tag == "exit-immersive" }) }
        val before = sink.chunks.get()
        SystemClock.sleep(800)
        check(sink.chunks.get() == before) { "pcm_after_stop" }
        check(Thread.getAllStackTraces().keys.none { it.name == "mimi-capture" && it.isAlive }) { "capture_worker_retained" }
    }

    private fun await(label: String, timeoutMs: Long = 15_000, condition: () -> Boolean) {
        stage = label
        val deadline = SystemClock.elapsedRealtime() + timeoutMs
        while (!condition()) {
            check(SystemClock.elapsedRealtime() < deadline) { label }
            SystemClock.sleep(100)
        }
    }

    private fun checkpoint(label: String, sink: DiscardEngine) {
        val observation = MimiService.captureObservation
        if (observation != null) checkNoSoundHint(expected =
            observation.state == CaptureHealth.State.SILENT || observation.state == CaptureHealth.State.NO_PCM)
        val entry = JSONObject().put("check", label).put("chunks", sink.chunks.get())
            .put("bytes", sink.bytes.get()).put("nonzeroChunks", sink.nonzero.get()).put("zeroChunks", sink.zero.get())
            .put("state", observation?.state?.name ?: "STOPPED")
            .put("pcmAgeMs", observation?.pcmAgeMs ?: JSONObject.NULL)
            .put("soundAgeMs", observation?.soundAgeMs ?: JSONObject.NULL)
        observations.put(entry)
        instrumentation.waitForIdleSync()
        // Main-queue idle can precede WRAP_CONTENT relayout and Surface buffer submission.
        // Let the first frame lay out/draw and the following frame publish it before capture.
        val frames = CountDownLatch(1)
        runOnMainSync {
            Choreographer.getInstance().postFrameCallback {
                Choreographer.getInstance().postFrameCallback { frames.countDown() }
            }
        }
        check(frames.await(2, TimeUnit.SECONDS)) { "screenshot_frames_timeout" }
        instrumentation.waitForIdleSync()
        val bitmap = checkNotNull(instrumentation.uiAutomation.takeScreenshot())
        try {
            File(targetContext.getExternalFilesDir(null), "capture-probe").apply { mkdirs() }
                .resolve("$label.png").outputStream().use { check(bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)) }
        } finally { bitmap.recycle() }
        sendStatus(0, Bundle().apply { putString("stream", "PLAYBACK_CAPTURE_CHECK $entry\n") })
    }

    private class DiscardEngine : ProviderEngine {
        override val sampleRateHz = 16_000
        val chunks = AtomicLong()
        val bytes = AtomicLong()
        val nonzero = AtomicLong()
        val zero = AtomicLong()
        val stopped = AtomicBoolean()
        override fun start(apiKey: String, sourceLang: String, targetLang: String, customBaseUrl: String, customModel: String) {
            error("probe_must_not_start_provider")
        }
        override fun sendAudio(pcm16Mono: ByteArray) {
            check(!stopped.get())
            chunks.incrementAndGet()
            bytes.addAndGet(pcm16Mono.size.toLong())
            if (pcm16Mono.any { it != 0.toByte() }) nonzero.incrementAndGet() else zero.incrementAndGet()
        }
        override fun stop() { stopped.set(true) }
    }

    private class SyntheticMedia(policy: Int) : AutoCloseable {
        @Volatile var silent = false
        val framesPlayed get() = track.playbackHeadPosition
        private val active = AtomicBoolean(true)
        private val track = AudioTrack.Builder()
            .setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA)
                .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC).setAllowedCapturePolicy(policy).build())
            .setAudioFormat(AudioFormat.Builder().setEncoding(AudioFormat.ENCODING_PCM_FLOAT)
                .setSampleRate(48_000).setChannelMask(AudioFormat.CHANNEL_OUT_STEREO).build())
            .setBufferSizeInBytes(AudioTrack.getMinBufferSize(48_000, AudioFormat.CHANNEL_OUT_STEREO, AudioFormat.ENCODING_PCM_FLOAT) * 2)
            .setTransferMode(AudioTrack.MODE_STREAM).build()
        private val worker: Thread
        init {
            check(track.state == AudioTrack.STATE_INITIALIZED)
            track.play()
            worker = thread(name = "mimi-synthetic-media") {
                val tone = FloatArray(9_600) { index -> (0.2 * sin(2 * Math.PI * 1_000 * (index / 2) / 48_000)).toFloat() }
                val zero = FloatArray(tone.size)
                try {
                    while (active.get()) check(track.write(if (silent) zero else tone, 0, tone.size, AudioTrack.WRITE_BLOCKING) > 0)
                } catch (_: Exception) { active.set(false) }
            }
        }
        override fun close() {
            active.set(false)
            runCatching { track.stop() }
            worker.join(1_000)
            track.release()
            check(!worker.isAlive) { "media_worker_retained" }
        }
    }
}
