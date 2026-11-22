package app.yuxino.mimi.android.shortcuts

import android.Manifest
import android.app.StatusBarManager
import android.content.ComponentName
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.drawable.Icon
import android.net.Uri
import android.os.Build
import android.provider.Settings
import android.view.Gravity
import android.widget.LinearLayout
import android.widget.Toast
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import app.yuxino.mimi.android.MimiApplication
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.capture.CaptureNotificationPermission
import app.yuxino.mimi.android.services.editor.ServiceSettingsUi
import app.yuxino.mimi.android.ui.HelpUi
import com.google.android.material.button.MaterialButton
import com.google.android.material.materialswitch.MaterialSwitch
import com.google.android.material.snackbar.Snackbar

internal class CaptionShortcutSettings(private val activity: AppCompatActivity) : LinearLayout(activity) {
    private val shortcuts = (activity.application as MimiApplication).captionShortcuts
    private var syncing = false

    private val overlayPermission = activity.registerForActivityResult(ActivityResultContracts.StartActivityForResult()) {
        if (Settings.canDrawOverlays(activity)) save { shortcuts.showEdge = true }
        else notice(R.string.need_overlay_permission)
        sync()
    }
    private val notificationSettings = activity.registerForActivityResult(ActivityResultContracts.StartActivityForResult()) {
        if (CaptionShortcutNotification.canPost(activity)) save { shortcuts.showNotification = true }
        else notice(R.string.shortcut_notifications_blocked)
        sync()
    }
    private val notificationPermission = activity.registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted && CaptionShortcutNotification.canPost(activity)) save { shortcuts.showNotification = true }
        else Snackbar.make(this, R.string.shortcut_notifications_blocked, Snackbar.LENGTH_LONG)
            .setAction(R.string.shortcut_open_settings) { openNotificationSettings() }.show()
        sync()
    }

    private val edge = toggle(R.string.shortcut_edge_label, "caption-edge-switch") { enabled ->
        if (enabled && !Settings.canDrawOverlays(activity)) {
            overlayPermission.launch(Intent(Settings.ACTION_MANAGE_OVERLAY_PERMISSION, Uri.parse("package:${activity.packageName}")))
        } else save { shortcuts.showEdge = enabled }
    }
    private val notification = toggle(R.string.shortcut_notification_label, "caption-notification-switch") { enabled ->
        if (enabled) CaptureNotificationPermission.markHandled(activity)
        when {
            !enabled || CaptionShortcutNotification.canPost(activity) -> save { shortcuts.showNotification = enabled }
            Build.VERSION.SDK_INT >= 33 && activity.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) !=
                PackageManager.PERMISSION_GRANTED -> notificationPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
            else -> openNotificationSettings()
        }
    }

    init {
        orientation = VERTICAL
        addView(HelpUi.heading(activity, activity.getString(R.string.shortcut_heading),
            activity.getString(R.string.shortcut_help), 17f, "caption-shortcuts-help"))
        addView(MaterialButton(activity, null, com.google.android.material.R.attr.materialButtonOutlinedStyle).apply {
            tag = "caption-add-tile"
            setText(R.string.shortcut_add_tile)
            isAllCaps = false
            minHeight = ServiceSettingsUi.dp(activity, 48)
            setOnClickListener { addTile() }
        }, LayoutParams(-1, -2).apply { topMargin = ServiceSettingsUi.dp(activity, 8); bottomMargin = ServiceSettingsUi.dp(activity, 8) })
        addView(edge)
        addView(notification)
        sync()
    }

    private fun toggle(label: Int, marker: String, onChange: (Boolean) -> Unit) = MaterialSwitch(activity).apply {
        tag = marker
        setText(label)
        textSize = 16f
        setTextColor(ContextCompat.getColor(activity, R.color.mimi_text))
        minHeight = ServiceSettingsUi.dp(activity, 52)
        switchPadding = ServiceSettingsUi.dp(activity, 12)
        gravity = Gravity.CENTER_VERTICAL
        val verticalInset = ServiceSettingsUi.dp(activity, 8)
        setPaddingRelative(0, verticalInset, 0, verticalInset)
        trackTintList = ContextCompat.getColorStateList(activity, R.color.mimi_switch_track)
        thumbTintList = ContextCompat.getColorStateList(activity, R.color.mimi_switch_thumb)
        setOnCheckedChangeListener { _, enabled ->
            if (!syncing) {
                try { onChange(enabled) }
                catch (_: android.content.ActivityNotFoundException) { notice(R.string.shortcut_settings_unavailable) }
                sync()
            }
        }
    }

    private fun openNotificationSettings() {
        try {
            notificationSettings.launch(CaptionShortcutNotification.settingsIntent(activity))
        } catch (_: android.content.ActivityNotFoundException) {
            notice(R.string.shortcut_settings_unavailable)
        }
    }

    override fun onWindowFocusChanged(hasWindowFocus: Boolean) {
        super.onWindowFocusChanged(hasWindowFocus)
        if (hasWindowFocus) { shortcuts.refresh(); sync() }
    }

    private fun sync() {
        syncing = true
        edge.isChecked = shortcuts.showEdge && Settings.canDrawOverlays(activity)
        notification.isChecked = shortcuts.showNotification && CaptionShortcutNotification.canPost(activity)
        syncing = false
    }

    private fun save(change: () -> Unit) {
        try { change() } catch (_: IllegalStateException) { notice(R.string.service_save_failed) }
    }

    private fun notice(message: Int) = Toast.makeText(activity, message, Toast.LENGTH_LONG).show()

    private fun addTile() {
        if (Build.VERSION.SDK_INT < 33) {
            HelpUi.showDetails(activity, activity.getString(R.string.shortcut_add_tile),
                activity.getString(R.string.shortcut_tile_manual))
            return
        }
        activity.getSystemService(StatusBarManager::class.java).requestAddTileService(
            ComponentName(activity, CaptionTileService::class.java), activity.getString(R.string.shortcut_tile_label),
            Icon.createWithResource(activity, R.drawable.ic_mimi), activity.mainExecutor,
        ) { result ->
            when (result) {
                StatusBarManager.TILE_ADD_REQUEST_RESULT_TILE_ALREADY_ADDED -> notice(R.string.shortcut_tile_already_added)
                StatusBarManager.TILE_ADD_REQUEST_RESULT_TILE_ADDED, StatusBarManager.TILE_ADD_REQUEST_RESULT_TILE_NOT_ADDED -> Unit
                else -> notice(R.string.shortcut_tile_manual)
            }
        }
    }
}
