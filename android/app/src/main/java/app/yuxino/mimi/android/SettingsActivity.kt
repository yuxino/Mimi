package app.yuxino.mimi.android

import android.content.Intent
import android.os.Bundle
import android.view.View
import android.widget.AdapterView
import android.widget.TextView
import android.widget.ArrayAdapter
import android.widget.RadioGroup
import android.widget.ScrollView
import android.widget.SeekBar
import android.widget.Spinner
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import com.google.android.material.button.MaterialButton
import com.google.android.material.materialswitch.MaterialSwitch
import app.yuxino.mimi.android.capture.MimiService

class SettingsActivity : AppCompatActivity() {

    private lateinit var fontSeek: SeekBar
    private lateinit var opacitySeek: SeekBar
    private lateinit var bgAlphaSeek: SeekBar
    private lateinit var historySeek: SeekBar
    private lateinit var colorSpinner: Spinner
    private val immersiveHelp = ImmersiveModeHelp(this)
    private var stopObservingAppearance: (() -> Unit)? = null
    private var lastColorPosition = 0
    private var syncImmersiveSelection: (() -> Unit)? = null

    // Display the neutral default first without changing persisted preset indices.
    private val colorIndices = listOf(1, 0, 2, 3, 4)
    private val colorNameIds = listOf(
        R.string.settings_color_white, R.string.settings_color_teal,
        R.string.settings_color_yellow, R.string.settings_color_green,
        R.string.settings_color_pink,
    )

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_settings)
        applySystemBarInsets()
        InterfaceLanguage.bind(this, findViewById(R.id.interface_language))
        findViewById<View>(R.id.back).setOnClickListener { finish() }
        val tabs = findViewById<RadioGroup>(R.id.settings_tabs)
        val stackedTabs = resources.configuration.fontScale >= 1.5f
        tabs.orientation = if (stackedTabs) RadioGroup.VERTICAL else RadioGroup.HORIZONTAL
        for (index in 0 until tabs.childCount) {
            tabs.getChildAt(index).layoutParams = RadioGroup.LayoutParams(
                if (stackedTabs) -1 else 0, if (stackedTabs) -2 else -1,
                if (stackedTabs) 0f else 1f,
            )
        }
        fun showTab() {
            val appearance = tabs.checkedRadioButtonId == R.id.tab_appearance
            findViewById<View>(R.id.service_panel).visibility = if (appearance) View.GONE else View.VISIBLE
            findViewById<View>(R.id.appearance_panel).visibility = if (appearance) View.VISIBLE else View.GONE
            findViewById<ScrollView>(R.id.settings_scroll).scrollTo(0, 0)
        }
        tabs.setOnCheckedChangeListener { _, _ -> showTab() }
        val initialTab = if (intent.getStringExtra("settings_section") == "appearance") {
            R.id.tab_appearance
        } else R.id.tab_service
        tabs.check(savedInstanceState?.getInt("settings_tab", initialTab) ?: initialTab)
        showTab()
        val morePanel = findViewById<View>(R.id.appearance_more_panel)
        val moreToggle = findViewById<MaterialButton>(R.id.appearance_more_toggle)
        fun showMore(expanded: Boolean) {
            morePanel.visibility = if (expanded) View.VISIBLE else View.GONE
            moreToggle.setText(if (expanded) R.string.settings_more_collapse else R.string.settings_more)
        }
        showMore(savedInstanceState?.getBoolean("settings_more") ?: false)
        moreToggle.setOnClickListener { showMore(morePanel.visibility != View.VISIBLE) }

        fontSeek = findViewById(R.id.font_size)
        opacitySeek = findViewById(R.id.overlay_opacity)
        bgAlphaSeek = findViewById(R.id.overlay_bg_alpha)
        historySeek = findViewById(R.id.history_lines)
        colorSpinner = findViewById(R.id.translation_color)
        val immersiveSwitch = findViewById<MaterialSwitch>(R.id.immersive_subtitles)
        immersiveSwitch.isChecked = SettingsStore.immersiveSubtitles(this)
        var updatingImmersiveSwitch = false
        fun syncImmersiveSwitch() {
            updatingImmersiveSwitch = true
            try { immersiveSwitch.isChecked = SettingsStore.immersiveSubtitles(this) }
            finally { updatingImmersiveSwitch = false }
        }
        fun applyImmersiveMode(enabled: Boolean) {
            SettingsStore.setImmersiveSubtitles(this, enabled)
            syncImmersiveSwitch()
            if (MimiService.isRunning) {
                startService(Intent(this, MimiService::class.java)
                    .setAction(MimiService.ACTION_APPLY_APPEARANCE))
            }
            refreshPreview()
        }
        syncImmersiveSelection = ::syncImmersiveSwitch
        immersiveSwitch.setOnCheckedChangeListener { _, enabled ->
            if (updatingImmersiveSwitch) return@setOnCheckedChangeListener
            if (enabled) {
                syncImmersiveSwitch()
                immersiveHelp.requestEnable(
                    onConfirmed = { applyImmersiveMode(true) },
                    onCancelled = { syncImmersiveSwitch(); refreshPreview() },
                )
            } else {
                immersiveHelp.dismiss()
                runCatching { applyImmersiveMode(false) }.onFailure { syncImmersiveSwitch() }
            }
        }

        fontSeek.progress = SettingsStore.fontSize(this)
        opacitySeek.progress = SettingsStore.overlayOpacity(this)
        bgAlphaSeek.progress = SettingsStore.overlayBgAlpha(this)
        historySeek.progress = SettingsStore.historyLines(this)
        colorSpinner.adapter = arrayAdapter(colorNameIds.map { getString(it) })
        colorSpinner.setSelection(colorIndices.indexOf(SettingsStore.translationColorIndex(this)).coerceAtLeast(0))

        val sliderListener = object : SeekBar.OnSeekBarChangeListener {
            override fun onProgressChanged(seekBar: SeekBar?, progress: Int, fromUser: Boolean) {
                refreshPreview()
                // Programmatic initialization/restoration must never write preferences.
                if (!fromUser) return
                when (seekBar?.id) {
                    R.id.font_size -> SettingsStore.setFontSize(this@SettingsActivity, progress)
                    R.id.overlay_bg_alpha -> SettingsStore.setOverlayBgAlpha(this@SettingsActivity, progress)
                    R.id.overlay_opacity -> SettingsStore.setOverlayOpacity(this@SettingsActivity, progress)
                    R.id.history_lines -> SettingsStore.setHistoryLines(this@SettingsActivity, progress)
                }
            }
            override fun onStartTrackingTouch(seekBar: SeekBar?) = Unit
            override fun onStopTrackingTouch(seekBar: SeekBar?) = Unit
        }
        listOf(fontSeek, opacitySeek, bgAlphaSeek, historySeek).forEach {
            it.setOnSeekBarChangeListener(sliderListener)
        }
        // Spinner reports its initial selection too; only a changed user selection saves.
        lastColorPosition = colorIndices.indexOf(SettingsStore.translationColorIndex(this)).coerceAtLeast(0)
        colorSpinner.onItemSelectedListener = object : AdapterView.OnItemSelectedListener {
            override fun onItemSelected(parent: AdapterView<*>?, view: View?, position: Int, id: Long) {
                refreshPreview()
                if (position == lastColorPosition) return
                lastColorPosition = position
                SettingsStore.setTranslationColorIndex(this@SettingsActivity, colorIndices[position])
            }
            override fun onNothingSelected(parent: AdapterView<*>?) = Unit
        }
        refreshPreview()
        stopObservingAppearance = SettingsStore.observeAppearance(this) { runOnUiThread { syncAppearance() } }

        findViewById<MaterialButton>(R.id.reset_position).setOnClickListener {
            SettingsStore.clearOverlayPosition(this)
            Toast.makeText(this, R.string.settings_reset_done, Toast.LENGTH_SHORT).show()
        }

    }

    override fun onResume() {
        super.onResume()
        ServiceSettingsUi.renderList(this, findViewById(R.id.service_panel))
        syncAppearance()
    }

    override fun onDestroy() {
        immersiveHelp.dismiss()
        stopObservingAppearance?.invoke()
        stopObservingAppearance = null
        super.onDestroy()
    }

    override fun onSaveInstanceState(outState: Bundle) {
        outState.putInt("settings_tab", findViewById<RadioGroup>(R.id.settings_tabs).checkedRadioButtonId)
        outState.putBoolean("settings_more", findViewById<View>(R.id.appearance_more_panel).visibility == View.VISIBLE)
        super.onSaveInstanceState(outState)
    }

    private fun refreshPreview() {
        val immersive = SettingsStore.immersiveSubtitles(this)
        bgAlphaSeek.isEnabled = !immersive
        opacitySeek.isEnabled = !immersive
        findViewById<TextView>(R.id.font_size_value).text = getString(R.string.settings_font_value, fontSeek.progress)
        findViewById<TextView>(R.id.overlay_opacity_value).text = getString(R.string.settings_percent_value, opacitySeek.progress)
        findViewById<TextView>(R.id.overlay_bg_alpha_value).text = getString(R.string.settings_percent_value, bgAlphaSeek.progress)
        findViewById<TextView>(R.id.history_lines_value).text =
            if (historySeek.progress == 0) getString(R.string.settings_history_off)
            else getString(R.string.settings_history_value, historySeek.progress)
        val colorIndex = colorIndices[colorSpinner.selectedItemPosition.coerceAtLeast(0)]
        findViewById<SubtitlePreviewView>(R.id.subtitle_preview).configure(
            fontSeek.progress, SettingsStore.COLOR_PRESETS[colorIndex].toInt(),
            opacitySeek.progress, bgAlphaSeek.progress,
            SettingsStore.targetLang(this), immersive, SettingsStore.originalTextOnly(this),
        )
    }

    private fun syncAppearance() {
        syncImmersiveSelection?.invoke()
        fontSeek.progress = SettingsStore.fontSize(this)
        opacitySeek.progress = SettingsStore.overlayOpacity(this)
        bgAlphaSeek.progress = SettingsStore.overlayBgAlpha(this)
        historySeek.progress = SettingsStore.historyLines(this)
        lastColorPosition = colorIndices.indexOf(SettingsStore.translationColorIndex(this)).coerceAtLeast(0)
        colorSpinner.setSelection(lastColorPosition)
        refreshPreview()
    }

    private fun arrayAdapter(items: List<String>): ArrayAdapter<String> {
        val adapter = ArrayAdapter(
            this, R.layout.mimi_spinner_item, items,
        )
        adapter.setDropDownViewResource(R.layout.mimi_spinner_dropdown)
        return adapter
    }
}
