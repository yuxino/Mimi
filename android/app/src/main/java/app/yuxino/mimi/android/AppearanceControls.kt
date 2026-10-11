package app.yuxino.mimi.android

import android.content.Context
import android.graphics.Typeface
import android.os.Handler
import android.os.Looper
import android.view.Gravity
import android.view.View
import android.widget.AdapterView
import android.widget.ArrayAdapter
import android.widget.LinearLayout
import android.widget.ImageButton
import android.widget.SeekBar
import android.widget.Spinner
import android.widget.TextView
import android.widget.Toast
import androidx.core.content.ContextCompat
import com.google.android.material.button.MaterialButton
import com.google.android.material.materialswitch.MaterialSwitch

/** Same native controls in the caption page and overlay; never starts a provider or capture. */
internal class AppearanceControls(
    context: Context,
    private val floating: Boolean = false,
    private val compactHome: Boolean = false,
    private val requestImmersive: (Boolean) -> Unit,
) : LinearLayout(context) {
    private val main = Handler(Looper.getMainLooper())
    private var observing: (() -> Unit)? = null
    private var syncing = false
    private val refreshers = mutableListOf<() -> Unit>()
    private val styles = if (compactHome) LinearLayout(context).apply {
        id = R.id.appearance_panel; orientation = VERTICAL
    } else this
    private val colorIndices = listOf(1, 0, 2, 3, 4)

    init {
        orientation = VERTICAL
        picker(R.id.display_mode, R.string.settings_display_mode, CaptionDisplayMode.entries.map { context.getString(it.label) },
            { (if (SettingsStore.originalTextOnly(context)) CaptionDisplayMode.ORIGINAL else SettingsStore.displayMode(context)).ordinal },
            { SettingsStore.setDisplayMode(context, CaptionDisplayMode.entries[it]) }).also { spinner ->
            refreshers.add { spinner.isEnabled = !SettingsStore.originalTextOnly(context) }
        }
        slider(R.id.font_size, R.id.font_size_value, R.string.settings_font, 12, 24,
            { SettingsStore.fontSize(context) }, { SettingsStore.setFontSize(context, it) }, R.string.settings_font_value, styles)
        slider(R.id.overlay_width, R.id.overlay_width_value, R.string.settings_width, 55, 100,
            { SettingsStore.overlayWidthPercent(context) }, { SettingsStore.setOverlayWidthPercent(context, it) }, parent = styles)
        val height = slider(R.id.overlay_height, R.id.overlay_height_value, R.string.settings_height, 20, 70,
            { SettingsStore.overlayHeightPercent(context) }, { SettingsStore.setOverlayHeightPercent(context, it) }, parent = styles)
        picker(R.id.caption_alignment, R.string.settings_alignment, CaptionAlignment.entries.map { context.getString(it.label) },
            { SettingsStore.captionAlignment(context).ordinal }, { SettingsStore.setCaptionAlignment(context, CaptionAlignment.entries[it]) }, styles)
        colorPalette(styles)
        val background = slider(R.id.overlay_bg_alpha, R.id.overlay_bg_alpha_value, R.string.settings_background, 0, 100,
            { SettingsStore.overlayBgAlpha(context) }, { SettingsStore.setOverlayBgAlpha(context, it) }, parent = styles)
        val opacity = slider(R.id.overlay_opacity, R.id.overlay_opacity_value, R.string.settings_opacity, 20, 100,
            { SettingsStore.overlayOpacity(context) }, { SettingsStore.setOverlayOpacity(context, it) }, parent = styles)
        val modeRow = LinearLayout(context).apply { gravity = Gravity.CENTER_VERTICAL }
        modeRow.addView(helpHeading(R.string.settings_immersive, R.string.settings_immersive_help, "immersive-help").apply { id = R.id.immersive_heading }, LayoutParams(0, -2, 1f))
        val immersive = MaterialSwitch(context).apply {
            id = R.id.immersive_subtitles
            minimumHeight = dp(48)
            contentDescription = context.getString(R.string.settings_immersive)
            trackTintList = ContextCompat.getColorStateList(context, R.color.mimi_switch_track)
            thumbTintList = ContextCompat.getColorStateList(context, R.color.mimi_switch_thumb)
            setOnCheckedChangeListener { _, enabled ->
                if (!syncing) { sync(); save { requestImmersive(enabled) } }
            }
        }
        modeRow.addView(immersive, LayoutParams(-2, -2))
        addView(modeRow, LayoutParams(-1, -2).apply { topMargin = dp(20) })
        refreshers.add {
            immersive.isChecked = SettingsStore.immersiveSubtitles(context)
            background.isEnabled = !immersive.isChecked
            opacity.isEnabled = !immersive.isChecked
            height.isEnabled = !immersive.isChecked
        }
        if (compactHome) {
            addView(View(context).apply { setBackgroundColor(color(R.color.mimi_border)) },
                LayoutParams(-1, dp(1)).apply { topMargin = dp(24); bottomMargin = dp(16) })
            addView(label(R.string.caption_style_controls).apply {
                typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL)
            }, LayoutParams(-1, -2))
            addView(styles, LayoutParams(-1, -2))
        }
        val more = LinearLayout(context).apply { orientation = VERTICAL }
        styles.addView(View(context).apply { setBackgroundColor(color(R.color.mimi_border)) },
            LayoutParams(-1, dp(1)).apply { topMargin = dp(24) })
        styles.addView(more, LayoutParams(-1, -2))
        slider(R.id.history_lines, R.id.history_lines_value, R.string.settings_history, 0, 6,
            { SettingsStore.historyLines(context) }, { SettingsStore.setHistoryLines(context, it) },
            R.string.settings_history_value, more)
        more.addView(MaterialButton(context, null, com.google.android.material.R.attr.borderlessButtonStyle).apply {
            id = R.id.reset_position; setText(R.string.settings_reset); isAllCaps = false; isSingleLine = false
            minimumHeight = dp(48); setTextColor(ContextCompat.getColor(context, R.color.mimi_text))
            setOnClickListener { save {
                SettingsStore.clearOverlayPosition(context)
                Toast.makeText(context, R.string.settings_reset_done, Toast.LENGTH_SHORT).show()
            } }
        }, LayoutParams(-2, -2).apply { gravity = Gravity.END; topMargin = dp(12) })
        sync()
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        observing = SettingsStore.observeAppearance(context) { main.post { if (isAttachedToWindow) sync() } }
        sync()
    }
    override fun onDetachedFromWindow() {
        observing?.invoke(); observing = null
        main.removeCallbacksAndMessages(null)
        super.onDetachedFromWindow()
    }
    fun sync() {
        syncing = true
        try {
            refreshers.forEach { it() }
        } finally { syncing = false }
    }
    private fun colorPalette(parent: LinearLayout) {
        parent.addView(label(R.string.settings_color), LayoutParams(-1, -2).apply { topMargin = dp(20) })
        val row = CaptionColorPalette(context).apply { id = R.id.translation_color }
        val names = listOf(R.string.settings_color_white, R.string.settings_color_teal, R.string.settings_color_yellow,
            R.string.settings_color_green, R.string.settings_color_pink)
        for ((position, index) in colorIndices.withIndex()) {
            val button = ImageButton(context).apply {
                tag = "caption-color-$index"
                contentDescription = context.getString(R.string.settings_color) + " · " + context.getString(names[position])
                tooltipText = context.getString(names[position]); isFocusable = true
                pointerIcon = android.view.PointerIcon.getSystemIcon(context, android.view.PointerIcon.TYPE_HAND)
                val background = android.util.TypedValue()
                context.theme.resolveAttribute(android.R.attr.selectableItemBackgroundBorderless, background, true)
                setBackgroundResource(background.resourceId)
                setPadding(dp(8), dp(8), dp(8), dp(8))
                setOnClickListener { save { SettingsStore.setTranslationColorIndex(context, index) } }
            }
            row.addView(button)
            refreshers.add {
                button.isSelected = SettingsStore.translationColorIndex(context) == index
                val swatch = android.graphics.drawable.GradientDrawable().apply {
                    shape = android.graphics.drawable.GradientDrawable.OVAL
                    setSize(dp(22), dp(22))
                    setColor(SettingsStore.COLOR_PRESETS[index].toInt()); setStroke(dp(1), color(R.color.mimi_muted))
                }
                val ring = android.graphics.drawable.GradientDrawable().apply {
                    shape = android.graphics.drawable.GradientDrawable.OVAL
                    setSize(dp(32), dp(32))
                    setColor(android.graphics.Color.TRANSPARENT)
                    if (button.isSelected) setStroke(dp(2), color(R.color.mimi_text))
                }
                button.setImageDrawable(android.graphics.drawable.LayerDrawable(arrayOf(ring, swatch)).apply {
                    setLayerInset(1, dp(5), dp(5), dp(5), dp(5))
                })
            }
        }
        parent.addView(row, LayoutParams(-1, -2).apply { topMargin = dp(8) })
    }
    private fun save(action: () -> Unit) {
        try { action() }
        catch (_: RuntimeException) { Toast.makeText(context, R.string.service_save_failed, Toast.LENGTH_LONG).show() }
        sync()
    }
    private fun slider(id: Int, valueId: Int, title: Int, low: Int, high: Int, read: () -> Int, write: (Int) -> Unit,
        format: Int = R.string.settings_percent_value, parent: LinearLayout = this): SeekBar {
        val heading = LinearLayout(context).apply { gravity = Gravity.CENTER_VERTICAL; isBaselineAligned = false }
        val label = if (id == R.id.history_lines) helpHeading(title, R.string.settings_history_help, "history-help").apply { this.id = R.id.history_heading } else label(title)
        heading.addView(label, LayoutParams(0, -2, 1f))
        val value = TextView(context).apply { this.id = valueId; textSize = 16f; setTextColor(color(R.color.mimi_muted)) }
        heading.addView(value, LayoutParams(-2, -2).apply { marginStart = dp(12) })
        parent.addView(heading, LayoutParams(-1, -2).apply { topMargin = dp(20) })
        val seek = SeekBar(context).apply {
            this.id = id; min = low; max = high; tag = if (id == R.id.font_size && floating) "overlay-font-slider" else null
            contentDescription = context.getString(title)
            progressTintList = ContextCompat.getColorStateList(context, R.color.mimi_text)
            thumbTintList = progressTintList
            progressBackgroundTintList = ContextCompat.getColorStateList(context, R.color.mimi_border)
            setOnSeekBarChangeListener(object : SeekBar.OnSeekBarChangeListener {
                override fun onProgressChanged(seekBar: SeekBar?, progress: Int, fromUser: Boolean) {
                    if (fromUser && !syncing) save { write(progress) }
                }
                override fun onStartTrackingTouch(seekBar: SeekBar?) = Unit
                override fun onStopTrackingTouch(seekBar: SeekBar?) = Unit
            })
        }
        parent.addView(seek, LayoutParams(-1, dp(48)))
        refreshers.add {
            seek.progress = read()
            value.text = if (id == R.id.history_lines && seek.progress == 0) context.getString(R.string.settings_history_off)
                else context.getString(format, seek.progress)
        }
        return seek
    }
    private fun picker(id: Int, title: Int, items: List<String>, read: () -> Int, write: (Int) -> Unit, parent: LinearLayout = this): Spinner {
        parent.addView(if (id == R.id.display_mode) helpHeading(title, R.string.settings_display_mode_help, "display-mode-help")
            else label(title), LayoutParams(-1, -2).apply { topMargin = dp(20); bottomMargin = dp(8) })
        val spinner = Spinner(context).apply {
            this.id = id; contentDescription = context.getString(title)
            background = ContextCompat.getDrawable(context, R.drawable.settings_field)
            adapter = ArrayAdapter(context, R.layout.mimi_spinner_item, items).apply {
                setDropDownViewResource(R.layout.mimi_spinner_dropdown)
            }
            setPopupBackgroundResource(R.color.mimi_bg)
            setSelection(read())
            onItemSelectedListener = object : AdapterView.OnItemSelectedListener {
                override fun onItemSelected(parent: AdapterView<*>?, view: View?, position: Int, itemId: Long) {
                    if (!syncing && position != read() && isEnabled) save { write(position) }
                }
                override fun onNothingSelected(parent: AdapterView<*>?) = Unit
            }
        }
        parent.addView(spinner, LayoutParams(-1, -2))
        refreshers.add { if (spinner.selectedItemPosition != read()) spinner.setSelection(read()) }
        return spinner
    }
    private fun helpHeading(title: Int, help: Int, tag: String): View =
        (context as? androidx.appcompat.app.AppCompatActivity)?.let { activity ->
            HelpUi.heading(activity, activity.getString(title), activity.getString(help), 16f, tag)
        } ?: label(title)
    private fun label(title: Int) = TextView(context).apply {
        setText(title); textSize = 16f; setTypeface(typeface, Typeface.NORMAL); setTextColor(color(R.color.mimi_text))
    }
    private fun color(id: Int) = ContextCompat.getColor(context, id)
    private fun dp(value: Int) = (value * resources.displayMetrics.density + 0.5f).toInt()
}
