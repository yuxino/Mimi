package app.yuxino.mimi.android.shortcuts

import android.app.KeyguardManager
import android.content.Intent
import android.os.Bundle
import android.os.PowerManager
import android.view.View
import android.view.ViewGroup
import android.widget.Toast
import androidx.activity.OnBackPressedCallback
import androidx.appcompat.app.AppCompatActivity
import app.yuxino.mimi.android.MainActivity
import app.yuxino.mimi.android.MimiApplication
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.capture.CaptureStartFlow
import app.yuxino.mimi.android.capture.CaptureInput
import app.yuxino.mimi.android.capture.CaptureStartResult
import app.yuxino.mimi.android.capture.MimiService

/** A real, transparent consent owner. It never constructs the main settings UI. */
class CaptionShortcutEntry : AppCompatActivity() {
    private lateinit var captureStart: CaptureStartFlow
    private var route = CaptionShortcutStartRoute()
    private var resumed = false
    private var needsSettings = false
    private var continuingSystemAudio = false
    private val stateListener: () -> Unit = { runOnUiThread {
        captureStart.reconcileServiceStart()
        continueStart()
    } }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (savedInstanceState == null && !CaptionShortcutActions.isStartRequest(this, intent)) {
            finishAndRemoveTask()
            return
        }
        route = CaptionShortcutStartRoute(savedInstanceState?.getString("route")?.let { saved ->
            CaptionShortcutPhase.entries.firstOrNull { it.name == saved }
        } ?: CaptionShortcutPhase.IDLE)
        needsSettings = savedInstanceState?.getBoolean("needs_settings") ?: false
        continuingSystemAudio = savedInstanceState?.getBoolean("continuing_system_audio") ?: false
        captureStart = CaptureStartFlow(this, savedInstanceState?.getBundle("capture_start"),
            onChanged = {}, onFinished = ::captureFinished)
        // The decor is genuinely shown for Android's foreground owner check, but
        // has no opaque pixels, settings hierarchy, or launch-preview content.
        setContentView(View(this), ViewGroup.LayoutParams(-1, -1))
        onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
            override fun handleOnBackPressed() { finishAndRemoveTask() }
        })
        if (savedInstanceState == null) acceptRequest(intent)
    }

    private fun acceptRequest(request: Intent) {
        if (!CaptionShortcutActions.isStartRequest(this, request)) return
        request.action = null
        if (route.active) return
        continuingSystemAudio = request.getBooleanExtra(CaptionShortcutActions.EXTRA_CONTINUE_SYSTEM_AUDIO, false)
        if (continuingSystemAudio && !(application as MimiApplication).captionShortcuts.consumeContinuation()) {
            finishAndRemoveTask()
            return
        }
        route.request(captureStart.starting || MimiService.isStarting || MimiService.isStopping)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        acceptRequest(intent)
        continueStart()
    }

    override fun onStart() {
        super.onStart()
        if (::captureStart.isInitialized) MimiService.addStateListener(stateListener)
    }

    override fun onStop() {
        MimiService.removeStateListener(stateListener)
        super.onStop()
    }

    override fun onResume() {
        super.onResume()
        resumed = true
        if (::captureStart.isInitialized) captureStart.reconcileServiceStart()
        continueStart()
    }

    override fun onPause() {
        resumed = false
        super.onPause()
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus) continueStart()
    }

    private fun captureFinished(result: CaptureStartResult) {
        when (result) {
            CaptureStartResult.STARTED -> route.observeStarted(running = true, busy = false)
            CaptureStartResult.CANCELLED, CaptureStartResult.PERMISSIONS_READY -> route.cancelled()
            CaptureStartResult.SETUP_REQUIRED, CaptureStartResult.FAILED -> {
                if (continuingSystemAudio) route.cancelled() else needsSettings = true
            }
        }
        continueStart()
    }

    private fun continueStart() {
        if (!::captureStart.isInitialized || isFinishing || isDestroyed) return
        val visibleAndUnlocked = resumed && window.decorView.isShown && hasWindowFocus() &&
            getSystemService(PowerManager::class.java).isInteractive &&
            !getSystemService(KeyguardManager::class.java).isKeyguardLocked
        if (!visibleAndUnlocked) return
        if (needsSettings) {
            needsSettings = false
            try {
                startActivity(Intent(this, MainActivity::class.java).putExtra("settings_section", "services")
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_REORDER_TO_FRONT or Intent.FLAG_ACTIVITY_SINGLE_TOP))
            } catch (_: RuntimeException) { Toast.makeText(this, R.string.capture_failed, Toast.LENGTH_LONG).show() }
            finishAndRemoveTask()
            return
        }
        if (route.dispatch(visibleAndUnlocked,
                captureStart.starting || MimiService.isStarting || MimiService.isStopping, MimiService.isRunning)) {
            captureStart.begin(requiredInput = if (continuingSystemAudio) CaptureInput.SYSTEM else null)
        }
        if (!route.active || route.takeReturn(visibleAndUnlocked, captureStart.notificationPermissionPending)) finishAndRemoveTask()
    }

    override fun onSaveInstanceState(outState: Bundle) {
        if (::captureStart.isInitialized) outState.putBundle("capture_start", captureStart.saveState())
        outState.putString("route", route.phase.name)
        outState.putBoolean("needs_settings", needsSettings)
        outState.putBoolean("continuing_system_audio", continuingSystemAudio)
        super.onSaveInstanceState(outState)
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        captureStart.onRequestPermissionsResult(requestCode, grantResults)
    }

    @Deprecated("Deprecated in Java")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        captureStart.onActivityResult(requestCode, resultCode, data)
    }

    override fun onDestroy() {
        if (::captureStart.isInitialized) captureStart.onDestroy()
        super.onDestroy()
    }
}
