package app.yuxino.mimi.android.sessions

import android.app.Activity
import android.app.Instrumentation
import android.content.Intent
import android.os.Bundle
import android.os.SystemClock
import android.view.inspector.WindowInspector
import app.yuxino.mimi.android.CaptureProbeActivity
import app.yuxino.mimi.android.capture.MimiService

/** Real window/lifecycle coverage with synthetic captions; no audio or provider session. */
internal class SavedSessionOverlayChecks(private val test: Instrumentation) {
    private fun onUi(action: () -> Unit) {
        var failure: Throwable? = null
        test.runOnMainSync { try { action() } catch (error: Throwable) { failure = error } }
        failure?.let { throw it }
    }
    private fun await(condition: () -> Boolean) {
        val deadline = SystemClock.elapsedRealtime() + 8_000
        while (true) {
            var done = false
            onUi { done = condition() }
            if (done) return
            check(SystemClock.elapsedRealtime() < deadline) { "overlay_lifecycle_timeout" }
            SystemClock.sleep(30)
        }
    }
    private fun overlayVisible() = WindowInspector.getGlobalWindowViews().any { it.tag == "mimi-overlay" && it.isShown }
    fun run() {
        val context = test.targetContext
        var probe: Activity? = null
        var saved: Activity? = null
        var stage = "preconditions"
        var failure: String? = null
        var ownsPreview = false
        try {
            check(!MimiService.isRunning && !MimiService.isStopping && !MimiService.isStarting)
            onUi { check(WindowInspector.getGlobalWindowViews().none { it.tag == "mimi-overlay" }) }
            probe = test.startActivitySync(Intent(context, CaptureProbeActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            onUi {
                context.startService(Intent(context, MimiService::class.java).setAction(MimiService.ACTION_UI_PREVIEW))
                ownsPreview = true
            }
            stage = "preview_visible"; await(::overlayVisible)
            saved = test.startActivitySync(Intent(context, SavedSessionsActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            stage = "archive_hides_overlay"; await { !overlayVisible() && saved!!.hasWindowFocus() }
            val monitor = test.addMonitor(SavedSessionsActivity::class.java.name, null, false)
            try {
                onUi { saved!!.recreate() }
                saved = checkNotNull(test.waitForMonitorWithTimeout(monitor, 8_000)) { "archive_recreation_timeout" }
            } finally { test.removeMonitor(monitor) }
            stage = "recreated_archive_hides_overlay"; await { !overlayVisible() && saved!!.hasWindowFocus() }
            onUi { saved!!.finish() }
            saved = null
            stage = "leaving_mimi_restores_overlay"; await(::overlayVisible)
        } catch (error: Throwable) { failure = "$stage:${error.javaClass.simpleName}" }
        finally {
            runCatching { onUi { saved?.finish() } }
            if (ownsPreview) runCatching {
                onUi { context.startService(MimiService.stopIntent(context)) }
                await { WindowInspector.getGlobalWindowViews().none { it.tag == "mimi-overlay" } }
            }.onFailure { failure = failure ?: "preview_cleanup" }
            runCatching { onUi { probe?.finish() } }
        }
        test.finish(if (failure == null) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
            putString("stream", "SAVED_OVERLAY_RESULT passed=${failure == null}; failure=${failure ?: "none"}; enter/recreate hide, leave restores; synthetic captions, no audio/provider.\n")
        })
    }
}
