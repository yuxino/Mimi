package app.yuxino.mimi.android.services.editor

import android.view.Gravity
import android.view.View
import android.widget.ArrayAdapter
import android.widget.LinearLayout
import android.widget.Spinner
import androidx.appcompat.app.AppCompatActivity
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.provider.*
import app.yuxino.mimi.android.ui.HelpUi
import com.google.android.material.button.MaterialButton
import org.json.JSONObject

/** Native recognition editor and diagnostic lifecycle. Text translation has its own editor and key. */
internal class SpeechModeSettings(
    private val activity: AppCompatActivity,
    saved: ServiceConfiguration,
    draft: ServiceEditorDraft?,
    private val configuration: () -> JSONObject?,
    private val modeChanged: () -> Unit,
) {
    private val xai = saved.provider == ServiceProvider.XAI
    private val modeLabel = if (xai) R.string.xai_mode else R.string.speech_translation_mode
    private val root = LinearLayout(activity).apply { orientation = LinearLayout.VERTICAL }
    private val mode = Spinner(activity).apply {
        tag = if (xai) "xai-mode" else "speech-translation-mode"; background = null
        adapter = ArrayAdapter(activity, R.layout.mimi_spinner_item,
            if (xai) listOf(activity.getString(R.string.xai_asr_mt), activity.getString(R.string.xai_voice))
            else listOf(activity.getString(R.string.speech_translation_simultaneous), activity.getString(R.string.speech_translation_asr_mt))).apply {
            setDropDownViewResource(R.layout.mimi_spinner_dropdown)
        }
        contentDescription = activity.getString(modeLabel)
        setSelection(if (xai) {
            draft?.xaiMode ?: if (saved.xaiSpeechMode == "asrMt") 0 else 1
        } else {
            draft?.speechTranslationMode ?: if (saved.speechTranslationMode == "asrMt") 1 else 0
        })
    }
    private val checks = LinearLayout(activity).apply { orientation = LinearLayout.VERTICAL }
    private var request: TranslationCall? = null
    private var generation = 0
    private val button = MaterialButton(activity, null, com.google.android.material.R.attr.materialButtonOutlinedStyle).apply {
        tag = if (xai) "xai-speech-check" else "speech-check"; setText(R.string.xai_check_speech)
        setIconResource(R.drawable.ic_check); isAllCaps = false; textSize = 16f
        setOnClickListener { check() }
    }
    private val result = ServiceSettingsUi.label(activity, "", 16f).apply {
        tag = if (xai) "xai-speech-status" else "speech-status"; visibility = View.GONE
        accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE
    }
    val view: View get() = root
    val selectedPosition: Int get() = mode.selectedItemPosition
    val speechMode: String get() = if (xai) {
        if (selectedPosition == 0) "asrMt" else "voice"
    } else if (selectedPosition == 0) "simultaneous" else "asrMt"

    init {
        root.addView(HelpUi.heading(activity, activity.getString(modeLabel),
            activity.getString(if (xai) R.string.xai_help else R.string.speech_translation_help), 18f,
            if (xai) "xai-mode-help" else "speech-translation-mode-help"))
        root.addView(mode, LinearLayout.LayoutParams(-1, dp(56)))
        checks.addView(button, LinearLayout.LayoutParams(-2, -2).apply { gravity = Gravity.END })
        checks.addView(result, LinearLayout.LayoutParams(-2, -2).apply { gravity = Gravity.END; bottomMargin = dp(8) })
        root.addView(checks)
        mode.onItemSelectedListener = object : android.widget.AdapterView.OnItemSelectedListener {
            override fun onItemSelected(parent: android.widget.AdapterView<*>?, view: View?, position: Int, id: Long) {
                invalidate(); render(); modeChanged()
            }
            override fun onNothingSelected(parent: android.widget.AdapterView<*>?) = Unit
        }
        render()
    }

    private fun render() {
        checks.visibility = if (speechMode == "asrMt") View.VISIBLE else View.GONE
    }

    fun invalidate() {
        ++generation; request?.cancel(); request = null; result.visibility = View.GONE
        button.isEnabled = mode.isEnabled; button.setText(R.string.xai_check_speech)
    }

    private fun check() {
        val session = configuration() ?: return
        val epoch = ++generation
        request?.cancel(); button.isEnabled = false; button.setText(R.string.translation_checking)
        result.visibility = View.GONE
        request = NativeTextProbeClient(networkSnapshot = NativeRuntimeNetwork::snapshot, speech = true).check(session) { outcome -> activity.runOnUiThread {
            if (generation != epoch || activity.isFinishing || activity.isDestroyed) return@runOnUiThread
            request = null; button.isEnabled = mode.isEnabled; button.setText(R.string.xai_check_speech)
            result.apply {
                visibility = View.VISIBLE
                text = when (outcome) {
                    is TranslationResult.Success -> activity.getString(R.string.translation_check_success, outcome.elapsedMs)
                    is TranslationResult.Failure -> activity.getString(when (outcome.code) {
                        "translation_http_401" -> R.string.translation_check_auth
                        "translation_http_429" -> R.string.translation_check_busy
                        "translation_timeout" -> R.string.translation_check_timeout
                        else -> R.string.translation_check_failure
                    }, outcome.elapsedMs)
                }
            }
        } }
    }

    fun setEditingEnabled(enabled: Boolean) {
        mode.isEnabled = enabled
        button.isEnabled = enabled && request == null
        if (!enabled) invalidate()
    }
    fun dispose() = invalidate()
    private fun dp(value: Int) = ServiceSettingsUi.dp(activity, value)
}
