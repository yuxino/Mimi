package app.yuxino.mimi.android.shortcuts

import android.app.KeyguardManager
import android.content.BroadcastReceiver
import android.content.ComponentCallbacks
import android.content.Context
import android.content.Intent
import android.content.SharedPreferences
import android.content.res.Configuration
import android.hardware.display.DisplayManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.PowerManager
import android.provider.Settings
import android.util.Log
import android.view.Display
import android.view.WindowManager
import android.widget.Toast
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.localization.InterfaceLanguage

/** The opt-in shortcut belongs to the app process, independent of any capture service. */
internal class CaptionEdgeControl(context: Context, private val preferences: SharedPreferences) {
    private val app = context.applicationContext
    private val main = Handler(Looper.getMainLooper())
    private var enabled = false
    private var ownUiVisible = false
    private var window: CaptionEdgeWindow? = null
    private var placement = CaptionEdgePlacement(preferences.getFloat("edge_x", 1f), preferences.getFloat("edge_y", .5f))
    private val reconcile = Runnable { reconcileWindow() }
    private val refreshAction = Runnable { window?.refreshAction() }
    private val sessionChanged: () -> Unit = { main.post(refreshAction) }
    private val configuration = object : ComponentCallbacks {
        override fun onConfigurationChanged(newConfig: Configuration) {
            main.removeCallbacks(reconcile)
            main.post(reconcile)
        }
        override fun onLowMemory() = Unit
    }
    private val screen = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            if (intent.action == Intent.ACTION_SCREEN_OFF) hide() else reconcileWindow()
        }
    }

    fun setEnabled(value: Boolean) {
        if (enabled == value) return
        enabled = value
        if (value) {
            app.registerCaptionScreenEvents(screen)
            app.registerComponentCallbacks(configuration)
            MimiService.addStateListener(sessionChanged)
            main.post(reconcile)
        } else {
            main.removeCallbacks(reconcile)
            MimiService.removeStateListener(sessionChanged)
            main.removeCallbacks(refreshAction)
            app.unregisterComponentCallbacks(configuration)
            app.unregisterReceiver(screen)
            hide()
        }
    }

    fun setOwnUiVisible(value: Boolean) {
        if (ownUiVisible == value) return
        ownUiVisible = value
        main.removeCallbacks(reconcile)
        if (value) hide() else if (enabled) main.post(reconcile)
    }

    private fun unlocked() = app.getSystemService(PowerManager::class.java).isInteractive &&
        !app.getSystemService(KeyguardManager::class.java).isKeyguardLocked

    private fun reconcileWindow() {
        if (!enabled || ownUiVisible || !unlocked() || !Settings.canDrawOverlays(app)) {
            hide()
            return
        }
        window?.let {
            it.refreshConfiguration()
            return
        }
        val display = app.getSystemService(DisplayManager::class.java).getDisplay(Display.DEFAULT_DISPLAY) ?: return
        try {
            val context = if (Build.VERSION.SDK_INT >= 30) app.createDisplayContext(display)
                .createWindowContext(WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY, null) else app
            val candidate = CaptionEdgeWindow(context, placement, ::toggle) { chosen ->
                // Persist a deliberate placement once, without writing during finger movement.
                preferences.edit().putFloat("edge_x", chosen.x).putFloat("edge_y", chosen.y).apply()
            }
            candidate.show()
            window = candidate
        } catch (_: WindowManager.InvalidDisplayException) {
            Log.w("CaptionEdgeControl", "shortcut_display_unavailable")
        } catch (_: WindowManager.BadTokenException) {
            Log.w("CaptionEdgeControl", "shortcut_window_unavailable")
        } catch (_: SecurityException) {
            Log.w("CaptionEdgeControl", "shortcut_overlay_permission_unavailable")
        }
    }

    private fun hide() {
        window?.let {
            placement = it.placement
            it.close()
            window = null
        }
    }

    private fun toggle() {
        if (!enabled || ownUiVisible || !unlocked()) return
        val stop = when (CaptionShortcutActions.state()) {
            CaptionShortcutState.INACTIVE -> false
            CaptionShortcutState.ACTIVE -> true
            else -> return
        }
        try {
            if (stop) checkNotNull(app.startService(MimiService.stopIntent(app)))
            else app.startActivity(CaptionShortcutActions.startIntent(app))
        } catch (_: RuntimeException) {
            Toast.makeText(InterfaceLanguage.context(app),
                if (stop) R.string.service_stop_failed else R.string.capture_failed, Toast.LENGTH_LONG).show()
        }
    }
}
