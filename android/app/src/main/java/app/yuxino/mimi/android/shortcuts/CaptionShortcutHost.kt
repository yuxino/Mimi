package app.yuxino.mimi.android.shortcuts

import android.app.Application
import android.app.KeyguardManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.Handler
import android.os.Looper
import android.os.PowerManager
import androidx.core.content.ContextCompat
import app.yuxino.mimi.android.capture.CaptureInput
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.localization.InterfaceLanguage

/** Owns optional idle controls and a process-local, one-time unlock continuation. */
internal class CaptionShortcutHost(private val application: Application) {
    private val preferences = application.getSharedPreferences("caption_shortcuts", Context.MODE_PRIVATE)
    private val edge = CaptionEdgeControl(application, preferences)
    private val notification = CaptionShortcutNotification(application)
    private val continuation = CaptionUnlockContinuation()
    private val main = Handler(Looper.getMainLooper())
    private val refreshNotification = Runnable { updateNotification() }
    private var observingNotification = false
    private val notificationEvents = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            if (intent.action == CaptionShortcutNotification.ACTION_DISMISSED) continuation.dismiss()
            updateNotification()
        }
    }
    private val onSessionChanged: () -> Unit = {
        main.removeCallbacks(refreshNotification)
        main.post(refreshNotification)
    }

    var showEdge: Boolean
        get() = preferences.getBoolean("edge", false)
        set(value) {
            check(preferences.edit().putBoolean("edge", value).commit())
            edge.setEnabled(value)
        }

    var showNotification: Boolean
        get() = preferences.getBoolean("notification", false)
        set(value) {
            check(preferences.edit().putBoolean("notification", value).commit())
            continuation.reset()
            observeNotification(value || continuation.pending)
            updateNotification()
        }

    init {
        observeNotification(showNotification)
        refresh()
    }

    private fun observeNotification(enabled: Boolean) {
        if (observingNotification == enabled) return
        observingNotification = enabled
        if (enabled) {
            MimiService.addStateListener(onSessionChanged)
            InterfaceLanguage.addListener(onSessionChanged)
            ContextCompat.registerReceiver(application, notificationEvents, IntentFilter().apply {
                addAction(Intent.ACTION_USER_PRESENT)
                addAction(Intent.ACTION_SCREEN_ON)
                addAction(Intent.ACTION_SCREEN_OFF)
                addAction(CaptionShortcutNotification.ACTION_DISMISSED)
            }, ContextCompat.RECEIVER_NOT_EXPORTED)
        } else {
            MimiService.removeStateListener(onSessionChanged)
            InterfaceLanguage.removeListener(onSessionChanged)
            main.removeCallbacks(refreshNotification)
            application.unregisterReceiver(notificationEvents)
        }
    }

    fun onLockInterruption(input: CaptureInput, active: Boolean, preview: Boolean, stopping: Boolean,
                           permissionsValid: Boolean, locked: Boolean) {
        if (CaptionShortcutNotification.canPost(application))
            continuation.interrupted(input, active, preview, stopping, permissionsValid, locked)
        observeNotification(showNotification || continuation.pending)
        // The service publishes its stopped state after releasing every capture lane.
    }

    fun consumeContinuation(): Boolean {
        val accepted = continuation.consume()
        updateNotification()
        return accepted
    }

    fun onCaptureStartingOrUserStop() {
        continuation.reset()
        observeNotification(showNotification)
        main.removeCallbacks(refreshNotification)
        main.post(refreshNotification)
    }

    private fun updateNotification() {
        val canPost = CaptionShortcutNotification.canPost(application)
        if (!canPost) continuation.reset()
        val unlocked = application.getSystemService(PowerManager::class.java).isInteractive &&
            !application.getSystemService(KeyguardManager::class.java).isKeyguardLocked
        val notice = continuation.notice(showNotification, canPost,
            MimiService.isStarting || MimiService.isRunning || MimiService.isStopping, unlocked)
        if (notification.refresh(notice) && notice == CaptionShortcutNotice.CONTINUE) continuation.offered()
        observeNotification(showNotification || continuation.pending)
    }

    fun setOwnUiVisible(visible: Boolean) = edge.setOwnUiVisible(visible)

    fun refresh() {
        edge.setEnabled(showEdge)
        updateNotification()
    }
}
