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
import android.widget.RadioGroup
import android.widget.ScrollView
import android.util.SparseIntArray
import androidx.activity.OnBackPressedCallback
import androidx.core.view.doOnNextLayout
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
    private val immersiveHelp = ImmersiveModeHelp(this)
    private val previewHandler = android.os.Handler(android.os.Looper.getMainLooper())
    private val previewUpdate = Runnable {
        if (!isFinishing && MimiService.isRunning) refreshLivePreview()
    }
    private var stopObservingAppearance: (() -> Unit)? = null
    private lateinit var appearance: AppearanceControls
    private val sectionScroll = SparseIntArray()
    private var currentSection = R.id.tab_caption
    private var lastSessionState: Pair<Boolean, Boolean>? = null
    private val subtitleListener = object : app.yuxino.mimi.android.provider.SubtitleBus.Listener {
        override fun onSubtitleChanged() { previewHandler.removeCallbacks(previewUpdate); previewHandler.post(previewUpdate) }
    }
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
        findViewById<LinearLayout>(R.id.home_heading).addView(HelpUi.heading(this,
            getString(R.string.app_name), getString(R.string.home_description),
            22f, "home-help").apply {
                (this as android.view.ViewGroup).getChildAt(0).let { label ->
                    (label as TextView).typeface = android.graphics.Typeface.create("sans-serif-black", android.graphics.Typeface.NORMAL)
                }
            })
        bindSections(savedInstanceState)

        guide = FirstRunGuide(this) { beginStartFlow(permissionOnly = guide.step == 2) }
        startStop = findViewById(R.id.start_stop)
        findViewById<View>(R.id.first_run_guide).setOnClickListener { guide.open() }
        if (savedInstanceState?.getBoolean("guide_open") == true) guide.open(savedInstanceState.getInt("guide_step"), savedInstanceState.getString("guide_provider"))
        else if (guide.shouldOpen()) guide.open()
        findViewById<View>(R.id.service_settings).setOnClickListener { openSettings() }
        findViewById<View>(R.id.source_language_action).setOnClickListener { showLanguages(source = true) }
        findViewById<View>(R.id.target_language_action).setOnClickListener { showLanguages(source = false) }
        findViewById<View>(R.id.brand_header).visibility = if (resources.configuration.orientation ==
            android.content.res.Configuration.ORIENTATION_LANDSCAPE) View.GONE else View.VISIBLE
        startStop.setOnClickListener {
            if (MimiService.isRunning) {
                stopService()
            } else if (!SettingsStore.isConfigured(this)) {
                openSettings()
            } else {
                beginStartFlow()
            }
        }
        if (savedInstanceState == null && intent.getStringExtra("settings_section") == "appearance") showSubtitleStyles()
    }

    override fun onStart() {
        super.onStart()
        MimiService.addStateListener(stateListener)
        app.yuxino.mimi.android.provider.SubtitleBus.addListener(subtitleListener)
        stopObservingAppearance = SettingsStore.observeAppearance(this) { runOnUiThread { refreshUi() } }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        when (intent.getStringExtra("settings_section")) {
            "services" -> openSettings()
            "appearance" -> showSubtitleStyles()
        }
    }

    override fun onStop() {
        MimiService.removeStateListener(stateListener)
        app.yuxino.mimi.android.provider.SubtitleBus.removeListener(subtitleListener)
        previewHandler.removeCallbacks(previewUpdate)
        stopObservingAppearance?.invoke()
        stopObservingAppearance = null
        immersiveHelp.dismiss()
        super.onStop()
    }

    override fun onSaveInstanceState(outState: Bundle) {
        outState.putBoolean("starting", starting)
        outState.putBoolean("permission_only", permissionOnly)
        outState.putBoolean("guide_open", guide.showing)
        outState.putInt("guide_step", guide.step)
        outState.putString("guide_provider", guide.providerId)
        outState.putString("main_section", when (currentSection) {
            R.id.tab_service -> "services"
            R.id.tab_general -> "general"
            else -> "captions"
        })
        sectionScroll.put(currentSection, findViewById<ScrollView>(R.id.settings_scroll).scrollY)
        outState.putIntArray("section_scroll", sections.map { sectionScroll[it.first] }.toIntArray())
        super.onSaveInstanceState(outState)
    }

    override fun onResume() {
        super.onResume()
        lastSessionState = null
        refreshUi()
        appearance.sync()
    }

    private fun openSettings() {
        findViewById<RadioGroup>(R.id.settings_tabs).check(R.id.tab_service)
    }

    private val sections = listOf(R.id.tab_caption to R.id.home_panel, R.id.tab_service to R.id.service_panel,
        R.id.tab_general to R.id.general_panel)

    private fun bindSections(savedInstanceState: Bundle?) {
        InterfaceLanguage.bind(this, findViewById(R.id.interface_language))
        appearance = AppearanceControls(this, compactHome = true) { enabled ->
            if (enabled) immersiveHelp.requestEnable(onConfirmed = {
                runCatching { SettingsStore.setImmersiveSubtitles(this, true) }
                    .onFailure { Toast.makeText(this, R.string.service_save_failed, Toast.LENGTH_LONG).show() }
                appearance.sync()
            }, onCancelled = { appearance.sync() })
            else { immersiveHelp.dismiss(); SettingsStore.setImmersiveSubtitles(this, false) }
        }
        findViewById<LinearLayout>(R.id.home_panel).apply {
            addView(appearance, indexOfChild(findViewById(R.id.service_settings)), LinearLayout.LayoutParams(-1, -2))
        }
        val tabs = findViewById<RadioGroup>(R.id.settings_tabs)
        val scroll = findViewById<ScrollView>(R.id.settings_scroll)
        savedInstanceState?.getIntArray("section_scroll")?.forEachIndexed { index, value ->
            if (index < sections.size) sectionScroll.put(sections[index].first, value)
        }
        val back = object : OnBackPressedCallback(false) {
            override fun handleOnBackPressed() { tabs.check(R.id.tab_caption) }
        }
        onBackPressedDispatcher.addCallback(this, back)
        fun showSection(selected: Int) {
            sections.forEach { (tab, panel) -> findViewById<View>(panel).visibility = if (tab == selected) View.VISIBLE else View.GONE }
            findViewById<CaptionWorkspace>(R.id.caption_workspace).showsPreview = selected == R.id.tab_caption
            currentSection = selected
            back.isEnabled = selected != R.id.tab_caption
            scroll.post { if (currentSection == selected) scroll.scrollTo(0, sectionScroll[selected]) }
        }
        val initial = when (savedInstanceState?.getString("main_section") ?: intent.getStringExtra("settings_section")) {
            "general" -> R.id.tab_general
            "services" -> R.id.tab_service
            else -> R.id.tab_caption
        }
        tabs.check(initial)
        showSection(tabs.checkedRadioButtonId)
        tabs.setOnCheckedChangeListener { _, selected ->
            sectionScroll.put(currentSection, scroll.scrollY)
            HelpUi.dismiss(this)
            immersiveHelp.dismiss()
            showSection(selected)
        }
        findViewById<View>(R.id.copy_capture_diagnostics).setOnClickListener { copyDiagnostics() }
    }

    private fun showSubtitleStyles() {
        HelpUi.dismiss(this)
        findViewById<RadioGroup>(R.id.settings_tabs).check(R.id.tab_caption)
        val styles = findViewById<View>(R.id.appearance_panel)
        styles.doOnNextLayout { styles.requestRectangleOnScreen(android.graphics.Rect(0, 0, styles.width, dp(176)), true) }
        styles.requestLayout()
    }

    private fun copyDiagnostics() {
        val observation = MimiService.captureObservation
        val report = "mimi Android capture diagnostics v1\n" +
            "androidApi=${Build.VERSION.SDK_INT}\nsource=android_playback_capture\nusage=media,game,unknown\nmicrophone=false\n" +
            "running=${MimiService.isRunning}\nobservation=${observation?.state?.name ?: "STOPPED"}\n" +
            "pcmAgeMs=${observation?.pcmAgeMs ?: "unknown"}\nsoundAgeMs=${observation?.soundAgeMs ?: "unknown"}\n" +
            "captureError=${MimiService.lastCaptureError ?: "none"}"
        val copied = runCatching {
            (getSystemService(Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager)
                .setPrimaryClip(android.content.ClipData.newPlainText("mimi capture diagnostics", report))
        }.isSuccess
        Toast.makeText(this, if (copied) R.string.capture_diagnostics_copied else R.string.capture_diagnostics_copy_failed,
            if (copied) Toast.LENGTH_SHORT else Toast.LENGTH_LONG).show()
    }

    private fun refreshUi() {
        try { refreshAvailableUi() } catch (_: Exception) {
            startStop.isEnabled = MimiService.isRunning && !MimiService.isStopping
            if (MimiService.isRunning) {
                startStop.setText(if (MimiService.isStopping) R.string.stopping_capture else R.string.stop_capture)
                startStop.setIconResource(R.drawable.ic_stop)
            }
            findViewById<View>(R.id.immersive_subtitles).isEnabled = false
            findViewById<TextView>(R.id.status).setText(R.string.service_save_failed)
            findViewById<TextView>(R.id.status_hint).setText(R.string.guide_storage_unavailable)
            findViewById<View>(R.id.status_hint).visibility = View.VISIBLE
            findViewById<View>(R.id.status_help).visibility = View.GONE
        }
    }

    private fun refreshAvailableUi() {
        guide.refresh()
        val running = MimiService.isRunning
        val sessionState = running to MimiService.isStopping
        if (lastSessionState != sessionState) {
            lastSessionState = sessionState
            ServiceSettingsUi.renderList(this, findViewById(R.id.service_panel))
        }
        val captureState = MimiService.captureObservation?.state
        val keyOk = SettingsStore.isConfigured(this)
        val overlayOk = Settings.canDrawOverlays(this)
        findViewById<View>(R.id.immersive_subtitles).isEnabled = true
        startStop.isEnabled = !starting && !MimiService.isStopping
        startStop.setText(when {
            MimiService.isStopping -> R.string.stopping_capture
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
        val hint = when {
            running && (captureState == CaptureHealth.State.NO_PCM || captureState == CaptureHealth.State.SILENT) -> R.string.capture_no_sound_hint
            running -> R.string.capture_source_hint
            !keyOk -> R.string.home_setup_hint
            !overlayOk -> R.string.home_overlay_hint
            else -> R.string.home_ready_hint
        }
        val needsAction = !running && (!keyOk || !overlayOk) ||
            running && (captureState == CaptureHealth.State.NO_PCM || captureState == CaptureHealth.State.SILENT)
        findViewById<TextView>(R.id.status_hint).apply {
            setText(hint); visibility = if (needsAction) View.VISIBLE else View.GONE
        }
        findViewById<android.widget.ImageButton>(R.id.status_help).apply {
            HelpUi.bind(this@MainActivity, this, this@MainActivity.findViewById<TextView>(R.id.status).text.toString(), getString(hint))
            visibility = View.VISIBLE
        }
        findViewById<TextView>(R.id.source_summary).text = languageLabel(SettingsStore.sourceLang(this))
        val originalOnly = SettingsStore.originalTextOnly(this)
        findViewById<TextView>(R.id.target_summary).apply {
            text = if (originalOnly) getString(R.string.translation_none) else languageLabel(SettingsStore.targetLang(this@MainActivity))
            setCompoundDrawablesRelativeWithIntrinsicBounds(0, 0, if (originalOnly) 0 else R.drawable.ic_expand, 0)
        }
        findViewById<View>(R.id.target_language_action).isEnabled = !originalOnly
        val selectedProvider = app.yuxino.mimi.android.provider.ServiceProvider.fromId(SettingsStore.provider(this))
        findViewById<android.widget.ImageView>(R.id.provider_icon).setImageResource(serviceProviderIcon(selectedProvider))
        findViewById<android.widget.ImageView>(R.id.translation_provider_icon).apply {
            val translation = SettingsStore.textTranslationProvider(this@MainActivity)
            val separate = selectedProvider == app.yuxino.mimi.android.provider.ServiceProvider.DASHSCOPE &&
                translation != app.yuxino.mimi.android.provider.TextTranslationProvider.BUILTIN &&
                translation != app.yuxino.mimi.android.provider.TextTranslationProvider.NONE
            visibility = if (separate) View.VISIBLE else View.GONE
            if (separate) setImageResource(translationProviderIcon(translation))
        }
        val provider = providerTitle(this, selectedProvider)
        findViewById<TextView>(R.id.provider_summary).text = getString(
            if (keyOk) R.string.home_service_ready else R.string.home_service_unset, provider,
        )
        findViewById<TextView>(R.id.model_summary).apply {
            val models = if (running) MimiService.activeModelNames else if (keyOk)
                runtimeModelNames(SettingsStore.runtimeConfiguration(this@MainActivity)) else emptyList()
            text = modelNamesLabel(models)
            visibility = if (models.isEmpty()) View.GONE else View.VISIBLE
        }
        findViewById<SubtitlePreviewView>(R.id.subtitle_preview).configure(
            SettingsStore.fontSize(this), SettingsStore.translationColor(this),
            SettingsStore.overlayOpacity(this), SettingsStore.overlayBgAlpha(this), SettingsStore.targetLang(this),
            SettingsStore.immersiveSubtitles(this), SettingsStore.originalTextOnly(this),
            SettingsStore.displayMode(this), SettingsStore.overlayWidthPercent(this), live = running,
            alignment = SettingsStore.captionAlignment(this), heightPercent = SettingsStore.overlayHeightPercent(this),
        )
        if (running) {
            refreshLivePreview()
        }
    }

    private fun refreshLivePreview() {
        val mode = if (SettingsStore.originalTextOnly(this)) CaptionDisplayMode.ORIGINAL else SettingsStore.displayMode(this)
        findViewById<SubtitlePreviewView>(R.id.subtitle_preview).showContent(CaptionFrameContent.live(this, mode,
            SettingsStore.historyLines(this), MimiService.captureObservation?.state, MimiService.isRunning, MimiService.isStopping))
    }

    private fun languageLabel(code: String): String = languageDisplayName(this, code)

    private fun showLanguages(source: Boolean) {
        if (!source && SettingsStore.originalTextOnly(this)) return
        val previousSource = SettingsStore.sourceLang(this)
        val previousTarget = SettingsStore.targetLang(this)
        val selected = if (source) SettingsStore.sourceLang(this) else SettingsStore.targetLang(this)
        val provider = app.yuxino.mimi.android.provider.ServiceProvider.fromId(SettingsStore.provider(this))
        val translation = SettingsStore.textTranslationProvider(this)
        val choices = if (source) provider.sourcesForTranslation(translation)
            else provider.targetsForTranslation(translation).filter { provider.supportsPair(previousSource, it, translation) }
        val dialog = BottomSheetDialog(this)
        val content = layoutInflater.inflate(R.layout.language_sheet, FrameLayout(this), false)
        content.findViewById<LinearLayout>(R.id.language_help_heading).addView(HelpUi.heading(this,
            getString(if (source) R.string.language_source_title else R.string.language_target_title),
            getString(R.string.language_sheet_hint), 22f, "language-help"))
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
        val (from, to) = provider.normalize(SettingsStore.sourceLang(this), SettingsStore.targetLang(this), SettingsStore.textTranslationProvider(this))
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
            openSettings()
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
        try { startService(MimiService.stopIntent(this)) }
        catch (_: RuntimeException) { Toast.makeText(this, R.string.service_stop_failed, Toast.LENGTH_LONG).show() }
        refreshUi()
    }

    companion object {
        private const val REQ_NOTIFICATIONS = 11
        private const val REQ_PROJECTION = 12
        private const val REQ_AUDIO = 13
        private const val REQ_OVERLAY = 14
    }
}
