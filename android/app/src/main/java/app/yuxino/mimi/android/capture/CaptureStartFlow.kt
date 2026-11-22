package app.yuxino.mimi.android.capture

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.media.projection.MediaProjectionManager
import android.net.Uri
import android.os.Bundle
import android.provider.Settings
import android.widget.Toast
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.localization.InterfaceLanguage
import app.yuxino.mimi.android.settings.SettingsStore

internal enum class CaptureStartResult { STARTED, PERMISSIONS_READY, CANCELLED, SETUP_REQUIRED, FAILED }

/** One visible Activity owns consent and service delivery, independently of its UI. */
internal class CaptureStartFlow(
    private val activity: Activity,
    savedState: Bundle?,
    private val onChanged: () -> Unit,
    private val onFinished: (CaptureStartResult) -> Unit,
    private val fromMainScreen: Boolean = false,
) {
    var starting = savedState?.getBoolean("starting") ?: false
        private set
    var input = savedState?.getString("input")?.let(CaptureInput::request)
        private set
    private var request = savedState?.getLong("request")?.takeIf { it != 0L }
    private var permissionOnly = savedState?.getBoolean("permission_only") ?: false
    private var requiredInput = savedState?.getString("required_input")?.let(CaptureInput::request)
    var notificationPermissionPending = savedState?.getBoolean("notification_pending") ?: false
        private set
    private var completion = savedState?.getString("completion")?.let { saved ->
        CaptureStartResult.entries.firstOrNull { it.name == saved }
    }

    fun saveState(): Bundle = Bundle().apply {
        putBoolean("starting", starting)
        putString("input", input?.id)
        putLong("request", request ?: 0L)
        putBoolean("permission_only", permissionOnly)
        putString("required_input", requiredInput?.id)
        putBoolean("notification_pending", notificationPermissionPending)
        putString("completion", completion?.name)
    }

    fun begin(permissionOnly: Boolean = false, requiredInput: CaptureInput? = this.requiredInput) {
        if (starting || MimiService.isStarting || MimiService.isRunning || MimiService.isStopping) return
        this.permissionOnly = permissionOnly
        this.requiredInput = requiredInput
        try { continuePermissions() } catch (_: RuntimeException) { failed() }
    }

    private fun continuePermissions() {
        val selected = input ?: SettingsStore.captureInput(activity).also { input = it }
        if (requiredInput != null && (selected != requiredInput || SettingsStore.captureInput(activity) != requiredInput)) {
            finish(CaptureStartResult.CANCELLED)
            return
        }
        if (!Settings.canDrawOverlays(activity)) {
            notice(R.string.need_overlay_permission)
            starting = true
            onChanged()
            ActivityCompat.startActivityForResult(activity, Intent(Settings.ACTION_MANAGE_OVERLAY_PERMISSION,
                Uri.parse("package:${activity.packageName}")), REQ_OVERLAY, null)
            return
        }
        if (!SettingsStore.isConfigured(activity)) {
            notice(R.string.need_api_key)
            finish(CaptureStartResult.SETUP_REQUIRED)
            return
        }
        if (ContextCompat.checkSelfPermission(activity, Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
            starting = true
            onChanged()
            notice(if (selected.usesMicrophone) R.string.microphone_permission_hint else R.string.need_audio_permission)
            ActivityCompat.requestPermissions(activity, arrayOf(Manifest.permission.RECORD_AUDIO), REQ_AUDIO)
            return
        }
        if (permissionOnly) {
            finish(CaptureStartResult.PERMISSIONS_READY)
            return
        }
        if (!selected.needsProjection) {
            startSelectedCapture(MimiService.microphoneIntent(activity))
            return
        }
        starting = true
        onChanged()
        val manager = activity.getSystemService(Context.MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
        ActivityCompat.startActivityForResult(activity, manager.createScreenCaptureIntent(), REQ_PROJECTION, null)
    }

    fun onRequestPermissionsResult(requestCode: Int, grantResults: IntArray) {
        when (requestCode) {
            REQ_AUDIO -> {
                starting = false
                if (grantResults.firstOrNull() == PackageManager.PERMISSION_GRANTED) begin(permissionOnly)
                else {
                    denied("audio-denied")
                    notice(R.string.guide_audio_denied)
                    finish(CaptureStartResult.CANCELLED)
                }
            }
            REQ_NOTIFICATIONS -> {
                notificationPermissionPending = false
                deliverCompletion()
            }
        }
    }

    fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        try {
            when (requestCode) {
                REQ_OVERLAY -> {
                    starting = false
                    if (Settings.canDrawOverlays(activity)) begin(permissionOnly)
                    else { denied("overlay-denied"); finish(CaptureStartResult.CANCELLED) }
                }
                REQ_PROJECTION -> {
                    starting = false
                    val selected = input
                    if (resultCode == Activity.RESULT_OK && data != null && selected?.needsProjection == true) {
                        activity.getSharedPreferences("first_run", 0).edit().remove("projection-denied").apply()
                        startSelectedCapture(MimiService.startIntent(activity, resultCode, data, selected))
                    } else {
                        denied("projection-denied")
                        notice(R.string.guide_projection_denied)
                        finish(CaptureStartResult.CANCELLED)
                    }
                }
            }
        } catch (_: RuntimeException) { failed() }
    }

    fun reconcileServiceStart() {
        request?.takeIf { !MimiService.startPending(it) }?.let {
            request = null
            finish(if (MimiService.isRunning) CaptureStartResult.STARTED else CaptureStartResult.FAILED)
        }
        deliverCompletion()
    }

    private fun startSelectedCapture(intent: Intent) {
        if (requiredInput != null && SettingsStore.captureInput(activity) != requiredInput) {
            finish(CaptureStartResult.CANCELLED)
            return
        }
        input = null
        starting = true
        val token = MimiService.prepareStart(activity)
        request = token
        MimiService.startFromActivity(activity, intent.putExtra(MimiService.EXTRA_START_REQUEST, token))
        if (CaptureNotificationPermission.takeFirstOffer(activity, fromMainScreen)) {
            notificationPermissionPending = true
            try {
                ActivityCompat.requestPermissions(activity, arrayOf(Manifest.permission.POST_NOTIFICATIONS), REQ_NOTIFICATIONS)
            } catch (_: RuntimeException) {
                // Capture is already starting. This optional dialog must not cancel the service request.
                notificationPermissionPending = false
            }
        }
        onChanged()
    }

    /** Finishing cancels undelivered work; recreation retains its original token. */
    fun onDestroy() {
        if (activity.isFinishing) request?.let(MimiService::cancelStartRequest)
    }

    private fun failed() {
        val token = request
        request = null
        token?.let(MimiService::cancelStartRequest)
        notificationPermissionPending = false
        notice(R.string.capture_failed)
        finish(CaptureStartResult.FAILED)
    }

    private fun finish(result: CaptureStartResult) {
        starting = false
        input = null
        permissionOnly = false
        completion = result
        onChanged()
        deliverCompletion()
    }

    private fun deliverCompletion() {
        if (notificationPermissionPending) return
        val result = completion ?: return
        completion = null
        onFinished(result)
    }

    private fun denied(key: String) {
        activity.getSharedPreferences("first_run", 0).edit().putBoolean(key, true).apply()
    }

    private fun notice(message: Int) = Toast.makeText(InterfaceLanguage.context(activity), message, Toast.LENGTH_LONG).show()

    private companion object {
        const val REQ_NOTIFICATIONS = 11
        const val REQ_PROJECTION = 12
        const val REQ_AUDIO = 13
        const val REQ_OVERLAY = 14
    }
}
