package app.yuxino.mimi.android.shortcuts

import android.graphics.drawable.Icon
import android.os.Handler
import android.os.Looper
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import android.widget.Toast
import androidx.core.service.quicksettings.TileServiceCompat
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.localization.InterfaceLanguage

/** System-owned tile lifecycle; capture remains owned by MimiService. */
class CaptionTileService : TileService() {
    private val handler = Handler(Looper.getMainLooper())
    private var listening = false
    private val render = Runnable { if (listening) updateTile() }
    private val stateListener: () -> Unit = { handler.removeCallbacks(render); handler.post(render) }

    override fun onStartListening() {
        super.onStartListening()
        listening = true
        MimiService.addStateListener(stateListener)
        InterfaceLanguage.addListener(stateListener)
        updateTile()
    }

    override fun onStopListening() {
        stopListening()
        super.onStopListening()
    }

    override fun onDestroy() {
        stopListening()
        super.onDestroy()
    }

    private fun stopListening() {
        listening = false
        MimiService.removeStateListener(stateListener)
        InterfaceLanguage.removeListener(stateListener)
        handler.removeCallbacks(render)
    }

    private fun updateTile() {
        val tile = qsTile ?: return
        val text = InterfaceLanguage.context(this)
        val state = CaptionShortcutActions.state()
        val detail = text.getString(when (state) {
            CaptionShortcutState.STARTING -> R.string.home_starting_status
            CaptionShortcutState.STOPPING -> R.string.stopping_capture
            CaptionShortcutState.ACTIVE -> R.string.stop_capture
            CaptionShortcutState.INACTIVE -> R.string.start_capture
        })
        tile.label = text.getString(R.string.shortcut_tile_label)
        tile.subtitle = detail
        tile.contentDescription = "${tile.label} · $detail"
        tile.icon = Icon.createWithResource(this, R.drawable.ic_mimi)
        tile.state = when (state) {
            CaptionShortcutState.ACTIVE -> Tile.STATE_ACTIVE
            CaptionShortcutState.STARTING, CaptionShortcutState.STOPPING -> Tile.STATE_UNAVAILABLE
            CaptionShortcutState.INACTIVE -> Tile.STATE_INACTIVE
        }
        tile.updateTile()
    }

    override fun onClick() {
        super.onClick()
        when (CaptionShortcutActions.state()) {
            CaptionShortcutState.STARTING, CaptionShortcutState.STOPPING -> return
            CaptionShortcutState.ACTIVE -> {
                try { startService(MimiService.stopIntent(this)) }
                catch (_: RuntimeException) { showFailure(R.string.service_stop_failed) }
            }
            CaptionShortcutState.INACTIVE -> if (isLocked) unlockAndRun { openStart() } else openStart()
        }
    }

    private fun openStart() {
        // Unlocking may outlive a concurrent start/stop from another entry.
        if (CaptionShortcutActions.state() in setOf(CaptionShortcutState.STARTING, CaptionShortcutState.STOPPING)) return
        try {
            TileServiceCompat.startActivityAndCollapse(this, CaptionShortcutActions.startActivityWrapper(this))
        } catch (_: RuntimeException) { showFailure(R.string.capture_failed) }
    }

    private fun showFailure(message: Int) {
        Toast.makeText(InterfaceLanguage.context(this), message, Toast.LENGTH_LONG).show()
        if (listening) updateTile()
    }
}
