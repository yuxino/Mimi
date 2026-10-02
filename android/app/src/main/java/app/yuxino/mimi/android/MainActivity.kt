package app.yuxino.mimi.android

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.media.projection.MediaProjectionManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.widget.Toast
import android.widget.TextView
import android.view.View
import android.view.Gravity
import android.widget.LinearLayout
import android.widget.FrameLayout
import androidx.appcompat.app.AppCompatActivity
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.capture.CaptureHealth
import com.google.android.material.button.MaterialButton
import com.google.android.material.bottomsheet.BottomSheetDialog
import com.google.android.material.snackbar.Snackbar

class MainActivity : AppCompatActivity() {

    private lateinit var startStop: MaterialButton
    private var starting = false
    private lateinit var guide: FirstRunGuide
    private val stateListener: () -> Unit = { runOnUiThread { refreshUi() } }

    private val projectionManager: MediaProjectionManager by lazy {
        getSystemService(Context.MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        starting = savedInstanceState?.getBoolean("starting") ?: false
        permissionOnly = savedInstanceState?.getBoolean("permission_only") ?: false
        val firstRunPrefs = getSharedPreferences("first_run", 0)
        if (!firstRunPrefs.contains("seen") && java.io.File(applicationInfo.dataDir, "shared_prefs/mimi_secure.xml").exists()) {
            firstRunPrefs.edit().putBoolean("seen", true).apply()
        }
        setContentView(R.layout.activity_main)
        applySystemBarInsets()

        guide = FirstRunGuide(this) { beginStartFlow(permissionOnly = guide.step == 2) }
        startStop = findViewById(R.id.start_stop)
        findViewById<View>(R.id.first_run_guide).setOnClickListener { guide.open() }
        if (savedInstanceState?.getBoolean("guide_open") == true) guide.open(savedInstanceState.getInt("guide_step"), savedInstanceState.getString("guide_provider"))
        else if (guide.shouldOpen()) guide.open()
        findViewById<MaterialButton>(R.id.go_settings).setOnClickListener {
            startActivity(Intent(this, SettingsActivity::class.java))
        }
        findViewById<View>(R.id.service_settings).setOnClickListener { openSettings() }
        findViewById<View>(R.id.source_language_action).setOnClickListener { showLanguages(source = true) }
        findViewById<View>(R.id.target_language_action).setOnClickListener { showLanguages(source = false) }
        findViewById<SubtitlePreviewView>(R.id.subtitle_preview).apply {
            showAppearanceShortcut()
            setOnClickListener {
                startActivity(Intent(this@MainActivity, SettingsActivity::class.java)
                    .putExtra("settings_section", "appearance"))
            }
        }
        findViewById<View>(R.id.copy_capture_diagnostics).setOnClickListener {
            val observation = MimiService.captureObservation
            val report = "mimi Android capture diagnostics v1\n" +
                "androidApi=${Build.VERSION.SDK_INT}\n" +
                "source=android_playback_capture\nusage=media,game,unknown\nmicrophone=false\n" +
                "running=${MimiService.isRunning}\n" +
                "observation=${observation?.state?.name ?: "STOPPED"}\n" +
                "pcmAgeMs=${observation?.pcmAgeMs ?: "unknown"}\n" +
                "soundAgeMs=${observation?.soundAgeMs ?: "unknown"}\n" +
                "captureError=${MimiService.lastCaptureError ?: "none"}"
            val copied = try {
                val clipboard = getSystemService(Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager
                clipboard.setPrimaryClip(android.content.ClipData.newPlainText("mimi capture diagnostics", report))
                true
            } catch (_: RuntimeException) {
                false
            }
            Toast.makeText(this,
                if (copied) R.string.capture_diagnostics_copied else R.string.capture_diagnostics_copy_failed,
                if (copied) Toast.LENGTH_SHORT else Toast.LENGTH_LONG,
            ).show()
        }
        startStop.setOnClickListener {
            if (MimiService.isRunning) {
                stopService()
            } else if (!SettingsStore.isConfigured(this)) {
                openSettings()
            } else {
                beginStartFlow()
            }
        }

    }

    override fun onStart() {
        super.onStart()
        MimiService.addStateListener(stateListener)
    }

    override fun onStop() {
        MimiService.removeStateListener(stateListener)
        super.onStop()
    }

    override fun onSaveInstanceState(outState: Bundle) {
        outState.putBoolean("starting", starting)
        outState.putBoolean("permission_only", permissionOnly)
        outState.putBoolean("guide_open", guide.showing)
        outState.putInt("guide_step", guide.step)
        outState.putString("guide_provider", guide.providerId)
        super.onSaveInstanceState(outState)
    }

    override fun onResume() {
        super.onResume()
        refreshUi()
    }

    private fun openSettings() {
        startActivity(Intent(this, SettingsActivity::class.java))
    }

    private fun refreshUi() {
        try { refreshAvailableUi() } catch (_: Exception) {
            startStop.isEnabled = false
            findViewById<TextView>(R.id.status).setText(R.string.service_save_failed)
            findViewById<TextView>(R.id.status_hint).setText(R.string.guide_storage_unavailable)
        }
    }

    private fun refreshAvailableUi() {
        guide.refresh()
        val running = MimiService.isRunning
        val captureState = MimiService.captureObservation?.state
        val keyOk = SettingsStore.isConfigured(this)
        val overlayOk = Settings.canDrawOverlays(this)
        startStop.isEnabled = !starting
        startStop.setText(when {
            running -> R.string.stop_capture
            !keyOk -> R.string.home_setup_action
            else -> R.string.start_capture
        })
        startStop.setIconResource(when {
            running -> R.drawable.ic_stop
            !keyOk -> R.drawable.ic_arrow
            else -> R.drawable.ic_play
        })
        findViewById<TextView>(R.id.status).setText(when {
            starting -> R.string.home_starting_status
            running && captureState == CaptureHealth.State.AUDIO -> R.string.capture_audio_detected
            running && captureState == CaptureHealth.State.NO_PCM -> R.string.capture_no_pcm
            running && captureState == CaptureHealth.State.SILENT -> R.string.capture_silent
            running -> R.string.capture_waiting
            !keyOk -> R.string.home_setup_status
            else -> R.string.home_ready_status
        })
        findViewById<TextView>(R.id.status_hint).setText(when {
            running && (captureState == CaptureHealth.State.NO_PCM || captureState == CaptureHealth.State.SILENT) -> R.string.capture_no_sound_hint
            running -> R.string.capture_source_hint
            !keyOk -> R.string.home_setup_hint
            !overlayOk -> R.string.home_overlay_hint
            else -> R.string.home_ready_hint
        })
        findViewById<TextView>(R.id.source_summary).text = languageLabel(SettingsStore.sourceLang(this))
        findViewById<TextView>(R.id.target_summary).text = languageLabel(SettingsStore.targetLang(this))
        val provider = providerTitle(this, app.yuxino.mimi.android.provider.ServiceProvider.fromId(SettingsStore.provider(this)))
        findViewById<TextView>(R.id.provider_summary).text = getString(
            if (keyOk) R.string.home_service_ready else R.string.home_service_unset, provider,
        )
        findViewById<SubtitlePreviewView>(R.id.subtitle_preview).configure(
            SettingsStore.fontSize(this), SettingsStore.translationColor(this),
            SettingsStore.overlayOpacity(this), SettingsStore.overlayBgAlpha(this), SettingsStore.targetLang(this),
            SettingsStore.immersiveSubtitles(this),
        )
    }

    private fun languageLabel(code: String): String = getString(when (code) {
        "zh" -> R.string.lang_zh
        "en" -> R.string.lang_en
        "ja" -> R.string.lang_ja
        "ko" -> R.string.lang_ko
        else -> R.string.lang_auto
    })

    private fun showLanguages(source: Boolean) {
        val previousSource = SettingsStore.sourceLang(this)
        val previousTarget = SettingsStore.targetLang(this)
        val selected = if (source) SettingsStore.sourceLang(this) else SettingsStore.targetLang(this)
        val provider = app.yuxino.mimi.android.provider.ServiceProvider.fromId(SettingsStore.provider(this))
        val choices = if (source) provider.sources else provider.targets.filter { it != SettingsStore.sourceLang(this) }
        val dialog = BottomSheetDialog(this)
        val content = layoutInflater.inflate(R.layout.language_sheet, FrameLayout(this), false)
        content.findViewById<TextView>(R.id.language_heading).setText(
            if (source) R.string.language_source_title else R.string.language_target_title,
        )
        val options = content.findViewById<LinearLayout>(R.id.language_options)
        for (code in choices) {
            val row = MaterialButton(this, null, com.google.android.material.R.attr.borderlessButtonStyle).apply {
                tag = "language-$code"
                text = languageLabel(code)
                isAllCaps = false
                textSize = 16f
                gravity = Gravity.CENTER_VERTICAL or Gravity.START
                setTextColor(ContextCompat.getColor(context, R.color.mimi_text))
                minHeight = dp(54)
                setPadding(dp(16), 0, dp(16), 0)
                cornerRadius = dp(12)
                if (code == selected) {
                    setBackgroundTintList(ContextCompat.getColorStateList(context, R.color.mimi_surface))
                    setIconResource(R.drawable.ic_check)
                    iconGravity = MaterialButton.ICON_GRAVITY_END
                    iconTint = ContextCompat.getColorStateList(context, R.color.mimi_text)
                }
                setOnClickListener {
                    dialog.dismiss()
                    if (code != selected) {
                        saveLanguage(source, code)
                        refreshUi()
                        Snackbar.make(this@MainActivity.findViewById(android.R.id.content), getString(
                            if (MimiService.isRunning) R.string.language_saved_next_session else R.string.language_saved,
                            languageLabel(code),
                        ), Snackbar.LENGTH_LONG).setAction(R.string.undo) {
                            SettingsStore.setSourceLang(this@MainActivity, previousSource)
                            SettingsStore.setTargetLang(this@MainActivity, previousTarget)
                            refreshUi()
                        }.show()
                    }
                }
            }
            options.addView(row, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT,
                LinearLayout.LayoutParams.WRAP_CONTENT))
        }
        dialog.setContentView(content)
        dialog.show()
        dialog.findViewById<View>(com.google.android.material.R.id.design_bottom_sheet)
            ?.backgroundTintList = ContextCompat.getColorStateList(this, R.color.mimi_bg)
    }

    private fun saveLanguage(source: Boolean, code: String) {
        if (source) SettingsStore.setSourceLang(this, code) else SettingsStore.setTargetLang(this, code)
        val provider = app.yuxino.mimi.android.provider.ServiceProvider.fromId(SettingsStore.provider(this))
        val (from, to) = provider.normalize(SettingsStore.sourceLang(this), SettingsStore.targetLang(this))
        SettingsStore.setSourceLang(this, from)
        SettingsStore.setTargetLang(this, to)
    }

    private fun dp(value: Int): Int = (value * resources.displayMetrics.density).toInt()

    private fun requestNotificationPermissionIfNeeded() {
        if (Build.VERSION.SDK_INT >= 33 &&
            ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS)
            != PackageManager.PERMISSION_GRANTED
        ) {
            ActivityCompat.requestPermissions(
                this, arrayOf(Manifest.permission.POST_NOTIFICATIONS), REQ_NOTIFICATIONS,
            )
        }
    }

    private var permissionOnly = false

    private fun beginStartFlow(permissionOnly: Boolean = false) {
        if (starting || MimiService.isRunning) return
        this.permissionOnly = permissionOnly
        if (!Settings.canDrawOverlays(this)) {
            Toast.makeText(this, R.string.need_overlay_permission, Toast.LENGTH_LONG).show()
            starting = true
            refreshUi()
            ActivityCompat.startActivityForResult(this,
                Intent(
                    Settings.ACTION_MANAGE_OVERLAY_PERMISSION,
                    Uri.parse("package:$packageName"),
                ), REQ_OVERLAY, null,
            )
            return
        }
        if (!SettingsStore.isConfigured(this)) {
            Toast.makeText(this, R.string.need_api_key, Toast.LENGTH_LONG).show()
            startActivity(Intent(this, SettingsActivity::class.java))
            return
        }
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.RECORD_AUDIO)
            != PackageManager.PERMISSION_GRANTED
        ) {
            starting = true
            refreshUi()
            Toast.makeText(this, R.string.need_audio_permission, Toast.LENGTH_LONG).show()
            ActivityCompat.requestPermissions(this, arrayOf(Manifest.permission.RECORD_AUDIO), REQ_AUDIO)
            return
        }
        if (permissionOnly) { refreshUi(); return }
        starting = true
        refreshUi()
        projectionManager.createScreenCaptureIntent().let {
            ActivityCompat.startActivityForResult(this, it, REQ_PROJECTION, null)
        }
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode == REQ_AUDIO) starting = false
        if (requestCode == REQ_AUDIO && grantResults.firstOrNull() == PackageManager.PERMISSION_GRANTED) {
            beginStartFlow(permissionOnly)
        } else if (requestCode == REQ_AUDIO) {
            getSharedPreferences("first_run", 0).edit().putBoolean("audio-denied", true).apply()
            Toast.makeText(this, getString(R.string.guide_audio_denied), Toast.LENGTH_LONG).show()
            refreshUi()
        }
    }

    @Deprecated("Deprecated in Java")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == REQ_OVERLAY) {
            starting = false
            if (Settings.canDrawOverlays(this)) beginStartFlow(permissionOnly)
            else getSharedPreferences("first_run", 0).edit().putBoolean("overlay-denied", true).apply()
            refreshUi()
        }
        if (requestCode == REQ_PROJECTION) {
            starting = false
            if (resultCode == Activity.RESULT_OK && data != null) {
                getSharedPreferences("first_run", 0).edit().remove("projection-denied").apply()
                ContextCompat.startForegroundService(
                    this, MimiService.startIntent(this, resultCode, data),
                )
                requestNotificationPermissionIfNeeded()
            } else {
                getSharedPreferences("first_run", 0).edit().putBoolean("projection-denied", true).apply()
                Toast.makeText(this, getString(R.string.guide_projection_denied), Toast.LENGTH_LONG).show()
            }
            refreshUi()
        }
    }

    private fun stopService() {
        stopService(Intent(this, MimiService::class.java))
        refreshUi()
    }

    companion object {
        private const val REQ_NOTIFICATIONS = 11
        private const val REQ_PROJECTION = 12
        private const val REQ_AUDIO = 13
        private const val REQ_OVERLAY = 14
    }
}
