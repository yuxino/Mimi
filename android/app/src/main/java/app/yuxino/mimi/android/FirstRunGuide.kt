package app.yuxino.mimi.android

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.provider.Settings
import android.view.Gravity
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ScrollView
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.provider.ServiceProvider
import com.google.android.material.bottomsheet.BottomSheetDialog
import com.google.android.material.button.MaterialButton

/** Phone setup keeps provider work separate from permission inspection. */
internal class FirstRunGuide(private val activity: AppCompatActivity, private val start: () -> Unit) {
    private var dialog: BottomSheetDialog? = null
    var step = 0
    private var selected = ServiceProvider.fromId(runCatching { SettingsStore.provider(activity) }.getOrDefault("dashscope"))
    val providerId get() = selected.id
    private fun configured() = runCatching { SettingsStore.isConfigured(activity, selected) }.getOrDefault(false)
    private lateinit var body: LinearLayout
    private var lastState = ""
    private val prefs get() = activity.getSharedPreferences("first_run", 0)
    val showing get() = dialog?.isShowing == true
    fun open(page: Int = 0, providerId: String? = null) {
        if (showing) dialog?.dismiss()
        step = page.coerceIn(0, 4)
        prefs.edit().putBoolean("seen", true).apply()
        selected = ServiceProvider.fromId(providerId ?: runCatching { SettingsStore.provider(activity) }.getOrDefault("dashscope"))
        dialog = BottomSheetDialog(activity).apply {
            body = LinearLayout(activity).apply {
                orientation = LinearLayout.VERTICAL
                setPadding(dp(24), dp(20), dp(24), dp(24))
                background = android.graphics.drawable.GradientDrawable().apply {
                    setColor(ContextCompat.getColor(activity, R.color.mimi_bg))
                    val radius = dp(24).toFloat()
                    cornerRadii = floatArrayOf(radius, radius, radius, radius, 0f, 0f, 0f, 0f)
                }
            }
            setContentView(ScrollView(activity).apply { addView(body) })
            setOnDismissListener { if (dialog === this) dialog = null }
            show()
            behavior.state = com.google.android.material.bottomsheet.BottomSheetBehavior.STATE_EXPANDED
        }
        render()
    }
    fun shouldOpen() = !prefs.getBoolean("seen", false)
    fun dismiss() { dialog?.dismiss() }
    fun refresh() {
        if (Settings.canDrawOverlays(activity)) prefs.edit().putBoolean("overlay-granted", true).apply()
        if (audioPermission()) prefs.edit().putBoolean("audio-granted", true).apply()
        if (!showing) return
        val state = "${configured()}:${Settings.canDrawOverlays(activity)}:" +
            "${audioPermission()}:${MimiService.isRunning}:${MimiService.captureObservation?.state}:" +
            "${prefs.getBoolean("overlay-denied", false)}:${prefs.getBoolean("audio-denied", false)}:${prefs.getBoolean("projection-denied", false)}:${MimiService.lastCaptureError}:" +
            "${synchronized(MimiService.firstRunEvidence) { MimiService.firstRunEvidence.complete }}"
        if (state != lastState) { lastState = state; render() }
    }
    private fun dp(value: Int) = ServiceSettingsUi.dp(activity, value)
    private fun label(text: String, size: Float = 16f, muted: Boolean = false) {
        body.addView(ServiceSettingsUi.label(activity, text, size, muted),
            LinearLayout.LayoutParams(-1, -2).apply { bottomMargin = dp(12) })
    }
    private fun heading(title: String, help: String, tag: String) {
        body.addView(HelpUi.heading(activity, title, help, 24f, tag),
            LinearLayout.LayoutParams(-1, -2).apply { bottomMargin = dp(12) })
    }
    private fun button(text: String, tag: String, secondary: Boolean = false, action: () -> Unit) {
        body.addView(MaterialButton(activity, null, if (secondary)
            com.google.android.material.R.attr.borderlessButtonStyle else com.google.android.material.R.attr.materialButtonStyle).apply {
            this.text = text; this.tag = tag; isAllCaps = false
            setOnClickListener { action() }
        }, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(4) })
    }
    private fun permissionStatus(name: String, allowed: Boolean): String {
        if (allowed) { prefs.edit().putBoolean("$name-granted", true).apply(); return activity.getString(R.string.guide_allowed) }
        return when {
            prefs.getBoolean("$name-granted", false) -> activity.getString(R.string.guide_revoked)
            prefs.getBoolean("$name-denied", false) -> activity.getString(R.string.guide_denied)
            else -> activity.getString(R.string.guide_unknown)
        }
    }
    private fun audioPermission() = ContextCompat.checkSelfPermission(activity, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED
    private fun render() {
        body.removeAllViews()
        // Existing brand artwork is the slot for the user-approved step poses.
        body.addView(ImageView(activity).apply {
            setImageResource(R.drawable.mimi_brand); contentDescription = activity.getString(R.string.brand_description)
            tag = "guide-character-$step"; scaleType = ImageView.ScaleType.FIT_CENTER
        }, LinearLayout.LayoutParams(dp(72), dp(72)).apply { gravity = Gravity.CENTER_HORIZONTAL; bottomMargin = dp(16) })
        label(activity.getString(R.string.guide_progress, step + 1), 12f, true)
        when (step) {
            0 -> {
                label(activity.getString(R.string.guide_intro), 24f)
                label(activity.getString(R.string.guide_intro_hint), muted = true)
                body.addView(android.widget.Spinner(activity).apply {
                    tag = "guide-provider"
                    adapter = android.widget.ArrayAdapter(activity, R.layout.guide_spinner_item,
                        ServiceProvider.entries.map { providerTitle(activity, it) }).apply { setDropDownViewResource(R.layout.mimi_spinner_dropdown) }
                    setSelection(ServiceProvider.entries.indexOf(selected))
                    onItemSelectedListener = object : android.widget.AdapterView.OnItemSelectedListener {
                        override fun onNothingSelected(parent: android.widget.AdapterView<*>?) = Unit
                        override fun onItemSelected(parent: android.widget.AdapterView<*>?, view: android.view.View?, position: Int, id: Long) {
                            val next = ServiceProvider.entries[position]
                            if (next != selected) { selected = next; render() }
                        }
                    }
                }, LinearLayout.LayoutParams(-1, dp(56)).apply { bottomMargin = dp(12) })
                label(providerDescription(activity, selected), muted = true)
                button(activity.getString(R.string.guide_continue_provider, providerTitle(activity, selected)), "guide-next") { step = 1; render() }
            }
            1 -> {
                heading(activity.getString(R.string.guide_connect, providerTitle(activity, selected)),
                    activity.getString(R.string.guide_saved_hint), "guide-storage-help")
                label(if (configured()) activity.getString(R.string.guide_saved) else activity.getString(R.string.guide_missing))
                button(activity.getString(R.string.guide_credentials), "guide-credentials") {
                    activity.startActivity(Intent(activity, ServiceSettingsActivity::class.java).putExtra("provider", selected.id))
                }
                if (MimiService.isRunning) {
                    label(activity.getString(R.string.service_stop_first), muted = true)
                    button(activity.getString(R.string.guide_stop), "guide-stop-before-config", true) {
                        activity.stopService(Intent(activity, MimiService::class.java))
                    }
                }
                if (!MimiService.isRunning && configured()) button(activity.getString(R.string.guide_check_permissions), "guide-next") {
                    if (runCatching { SettingsStore.activateProvider(activity, selected) }.getOrDefault(false)) { step = 2; render() }
                    else android.widget.Toast.makeText(activity, R.string.guide_storage_unavailable, android.widget.Toast.LENGTH_LONG).show()
                }
            }
            2 -> {
                label(activity.getString(R.string.guide_permission_title), 24f)
                label(activity.getString(R.string.guide_permission_states,
                    permissionStatus("overlay", Settings.canDrawOverlays(activity)), permissionStatus("audio", audioPermission()),
                    activity.getString(if (MimiService.isRunning) R.string.guide_projection_current else if (projectionSharingEnded(MimiService.isRunning, MimiService.lastCaptureError)) R.string.guide_projection_revoked else if (prefs.getBoolean("projection-denied", false)) R.string.guide_projection_cancelled else R.string.guide_projection_unknown)), muted = true)
                label(activity.getString(R.string.guide_permission_audio))
                label(activity.getString(R.string.guide_permission_hint), muted = true)
                button(activity.getString(R.string.guide_grant), "guide-permissions") { start() }
                if (Settings.canDrawOverlays(activity) && audioPermission()) button(activity.getString(R.string.guide_trial_next), "guide-next") { step = 3; render() }
            }
            3 -> {
                heading(activity.getString(R.string.guide_play_title), activity.getString(R.string.guide_capture_limits), "guide-capture-help")
                label(activity.getString(R.string.guide_trial_hint, providerTitle(activity, selected)), muted = true)
                label(when {
                    !MimiService.isRunning -> activity.getString(R.string.guide_not_started)
                    MimiService.captureObservation?.state == app.yuxino.mimi.android.capture.CaptureHealth.State.NO_PCM -> activity.getString(R.string.guide_no_pcm)
                    MimiService.captureObservation?.state == app.yuxino.mimi.android.capture.CaptureHealth.State.SILENT -> activity.getString(R.string.guide_silent)
                    MimiService.captureObservation?.state == app.yuxino.mimi.android.capture.CaptureHealth.State.AUDIO -> activity.getString(R.string.guide_audio)
                    else -> activity.getString(R.string.guide_wait_audio)
                })
                button(if (MimiService.isRunning) activity.getString(R.string.guide_see_caption) else activity.getString(R.string.guide_start_trial), "guide-test") {
                    if (MimiService.isRunning) { step = 4; render() } else start()
                }
            }
            4 -> {
                val complete = MimiService.isRunning && Settings.canDrawOverlays(activity) && audioPermission() &&
                    synchronized(MimiService.firstRunEvidence) { MimiService.firstRunEvidence.complete }
                label(if (complete) activity.getString(R.string.guide_complete_title) else activity.getString(R.string.guide_wait_caption_title), 24f)
                if (complete) label(activity.getString(R.string.guide_complete_hint), muted = true)
                val sharingEnded = projectionSharingEnded(MimiService.isRunning, MimiService.lastCaptureError)
                if (sharingEnded) label(activity.getString(R.string.guide_stopped), muted = true)
                button(if (complete) activity.getString(R.string.guide_finish) else if (sharingEnded) activity.getString(R.string.guide_reopen_sharing) else if (MimiService.isRunning) activity.getString(R.string.guide_waiting_back) else activity.getString(R.string.guide_retry), "guide-finish") {
                    if (complete || MimiService.isRunning) dismiss()
                    else if (sharingEnded) start()
                    else { step = 3; render() }
                }
                if (!complete && !MimiService.isRunning) button(activity.getString(R.string.guide_credentials), "guide-fix-credentials", true) {
                    step = 1; render()
                }
                if (!complete && MimiService.isRunning) button(activity.getString(R.string.guide_stop), "guide-stop", true) {
                    activity.stopService(Intent(activity, MimiService::class.java)); render()
                }
            }
        }
        if (step > 0) button(activity.getString(R.string.guide_back), "guide-back", true) { step--; render() }
        button(activity.getString(R.string.guide_skip), "guide-skip", true) { dismiss() }
    }
}
