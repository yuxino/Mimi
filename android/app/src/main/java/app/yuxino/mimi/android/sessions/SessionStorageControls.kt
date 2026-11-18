package app.yuxino.mimi.android.sessions

import android.content.Intent
import android.view.Gravity
import android.view.View
import android.widget.LinearLayout
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.services.editor.ServiceSettingsUi
import app.yuxino.mimi.android.settings.SettingsStore
import app.yuxino.mimi.android.ui.HelpUi
import com.google.android.material.button.MaterialButton
import com.google.android.material.materialswitch.MaterialSwitch

/** Caption-file opt-in is independent of the bounded on-screen history setting. */
internal class SessionStorageControls(private val activity: AppCompatActivity) : LinearLayout(activity) {
    private var syncing = false
    private val toggle = MaterialSwitch(activity)
    private val status = ServiceSettingsUi.label(activity, "", 16f).apply {
        accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE
    }
    private val retry = MaterialButton(activity, null, com.google.android.material.R.attr.borderlessButtonStyle).apply {
        setText(R.string.session_history_retry); isAllCaps = false
        setOnClickListener { sync() }
    }
    init {
        orientation = VERTICAL
        val row = LinearLayout(activity).apply { gravity = Gravity.CENTER_VERTICAL }
        row.addView(HelpUi.heading(activity, activity.getString(R.string.session_history_save),
            activity.getString(R.string.session_history_save_help), 16f, "session-history-help"), LayoutParams(0, -2, 1f))
        toggle.apply {
            tag = "session-history-save"; minimumHeight = ServiceSettingsUi.dp(activity, 48)
            contentDescription = activity.getString(R.string.session_history_save)
            trackTintList = ContextCompat.getColorStateList(activity, R.color.mimi_switch_track)
            thumbTintList = ContextCompat.getColorStateList(activity, R.color.mimi_switch_thumb)
            setOnCheckedChangeListener { _, enabled ->
                if (!syncing) {
                    runCatching { SettingsStore.setRetainSessionHistory(activity, enabled) }
                        .onFailure { Toast.makeText(activity, R.string.session_history_preference_error, Toast.LENGTH_LONG).show() }
                    sync()
                }
            }
        }
        row.addView(toggle, LayoutParams(-2, -2)); addView(row)
        addView(status, LayoutParams(-1, -2)); addView(retry, LayoutParams(-2, -2).apply { gravity = Gravity.END })
        addView(MaterialButton(activity, null, com.google.android.material.R.attr.borderlessButtonStyle).apply {
            tag = "saved-sessions"; setText(R.string.session_history_title); isAllCaps = false
            setOnClickListener { activity.startActivity(Intent(activity, SavedSessionsActivity::class.java)) }
        }, LayoutParams(-2, -2).apply { gravity = Gravity.END })
        sync()
    }
    fun sync(storageFailed: Boolean = MimiService.historySaveError, storageLimited: Boolean = MimiService.historyLimited) {
        syncing = true
        val saved = runCatching { SettingsStore.retainSessionHistory(activity) }
        toggle.isEnabled = saved.isSuccess && (saved.getOrDefault(false) || (!MimiService.isStarting && !MimiService.isRunning && !MimiService.isStopping))
        toggle.isChecked = saved.getOrDefault(false)
        syncing = false
        val message = when {
            saved.isFailure -> R.string.session_history_preference_error
            storageFailed -> R.string.session_history_recording_error
            storageLimited -> R.string.session_history_limit
            else -> null
        }
        status.visibility = if (message == null) View.GONE else View.VISIBLE
        message?.let(status::setText)
        retry.visibility = if (saved.isFailure) View.VISIBLE else View.GONE
    }
}
