package app.yuxino.mimi.android

import android.text.Editable
import android.text.InputType
import android.text.TextWatcher
import android.view.Gravity
import android.view.View
import android.widget.*
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import androidx.core.view.doOnLayout
import app.yuxino.mimi.android.provider.*
import com.google.android.material.button.MaterialButton
import com.google.android.material.textfield.TextInputEditText
import com.google.android.material.textfield.TextInputLayout

/** A draft only. ServiceSettingsActivity commits this alongside the speech configuration. */
internal class TextTranslationSettings(private val activity: AppCompatActivity, private val onModeChange: (Boolean) -> Unit = {}) {
    private class Draft(val saved: TranslationConfiguration) {
        var endpoint = saved.endpoint.ifBlank {
            if (saved.provider == TextTranslationProvider.CHAT_MOCK) "http://127.0.0.1:8000/v1" else ""
        }
        var model = saved.model
        var key = ""
        var localHttp = saved.allowLocalHttp
        var forgetKey = false
    }
    private val providers = listOf(TextTranslationProvider.BUILTIN, TextTranslationProvider.DEEPL, TextTranslationProvider.DEEPLX,
        TextTranslationProvider.CHAT_MOCK, TextTranslationProvider.OPENAI_COMPATIBLE, TextTranslationProvider.NONE)
    private val drafts = providers.associateWith { Draft(SettingsStore.translationConfiguration(activity, it)) }
    private var selected = SettingsStore.textTranslationProvider(activity)
    private var rendering = false
    private val root = LinearLayout(activity).apply { orientation = LinearLayout.VERTICAL }
    private val fields = LinearLayout(activity).apply { orientation = LinearLayout.VERTICAL }
    private val mode = Spinner(activity).apply { tag = "translation-mode"; background = null; setPadding(0, 0, 0, 0) }
    private val help: ImageButton
    private val inputLayouts = mutableMapOf<TextInputEditText, TextInputLayout>()
    private val endpoint = field(R.string.translation_endpoint, "translation-endpoint")
    private val model = field(R.string.translation_model, "translation-model")
    private val key = field(R.string.translation_key, "translation-key", true)
    private val localHttp = CheckBox(activity).apply {
        tag = "translation-local-http"; text = activity.getString(R.string.translation_local_http); textSize = 16f
    }
    private val result = ServiceSettingsUi.label(activity, "", 16f).apply {
        tag = "translation-status"; visibility = View.GONE
        accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE
    }
    private val check = MaterialButton(activity, null, com.google.android.material.R.attr.materialButtonOutlinedStyle).apply {
        tag = "translation-check"; setText(R.string.translation_check); setIconResource(R.drawable.ic_check)
        textSize = 16f; isAllCaps = false; cornerRadius = dp(12)
    }
    private val removeKey = MaterialButton(activity, null, com.google.android.material.R.attr.borderlessButtonStyle).apply {
        tag = "translation-remove-key"; setText(R.string.translation_remove_key); textSize = 16f; isAllCaps = false
    }
    private var request: TranslationCall? = null
    private var generation = 0
    val enabled: Boolean get() = selected != TextTranslationProvider.BUILTIN
    val view: View get() = root

    init {
        val header = HelpUi.heading(activity, activity.getString(R.string.translation_title),
            activity.getString(R.string.translation_help_builtin), 18f, "translation-help")
        help = header.findViewWithTag("translation-help")
        help.setOnClickListener { showHelp() }
        root.addView(header)
        mode.adapter = TranslationProviderAdapter(activity, providers)
        mode.contentDescription = activity.getString(R.string.translation_title)
        mode.setSelection(providers.indexOf(selected))
        root.addView(mode, LinearLayout.LayoutParams(-1, dp(56)).apply { bottomMargin = dp(16) })
        fields.addView(localHttp, LinearLayout.LayoutParams(-1, -2))
        val actions = LinearLayout(activity).apply { gravity = Gravity.END or Gravity.CENTER_VERTICAL }
        actions.addView(removeKey)
        actions.addView(check)
        fields.addView(actions, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(8) })
        fields.addView(result, LinearLayout.LayoutParams(-2, -2).apply { gravity = Gravity.END; topMargin = dp(8); bottomMargin = dp(12) })
        root.addView(fields)
        val watcher = object : TextWatcher {
            override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) = Unit
            override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) {
                if (rendering) return
                rememberDraft(); invalidateCheck(); updateKeyLabel()
            }
            override fun afterTextChanged(s: Editable?) = Unit
        }
        listOf(endpoint, model, key).forEach { it.addTextChangedListener(watcher) }
        localHttp.setOnCheckedChangeListener { _, _ -> if (!rendering) { rememberDraft(); invalidateCheck() } }
        removeKey.setOnClickListener {
            drafts.getValue(selected).forgetKey = true
            key.setText(""); rememberDraft(); updateKeyLabel(); invalidateCheck()
        }
        mode.onItemSelectedListener = object : AdapterView.OnItemSelectedListener {
            override fun onItemSelected(parent: AdapterView<*>?, view: View?, position: Int, id: Long) {
                if (providers[position] != selected) {
                    rememberDraft(); selected = providers[position]; renderDraft()
                }
            }
            override fun onNothingSelected(parent: AdapterView<*>?) = Unit
        }
        check.setOnClickListener { checkConnection() }
        renderDraft()
    }

    private fun rememberDraft() {
        if (rendering || !hasNetworkProvider()) return
        drafts.getValue(selected).apply {
            endpoint = this@TextTranslationSettings.endpoint.text.toString()
            model = this@TextTranslationSettings.model.text.toString()
            key = this@TextTranslationSettings.key.text.toString()
            localHttp = this@TextTranslationSettings.localHttp.isChecked
        }
    }

    private fun renderDraft() {
        invalidateCheck(); rendering = true
        help.tooltipText = activity.getString(helpResource()).substringBefore("\n\n")
        val state = drafts.getValue(selected)
        endpoint.setText(state.endpoint); model.setText(state.model); key.setText(state.key)
        localHttp.isChecked = state.localHttp
        fields.visibility = if (hasNetworkProvider()) View.VISIBLE else View.GONE
        val customEndpoint = selected.usesOpenAIProtocol || selected == TextTranslationProvider.DEEPLX
        ServiceSettingsUi.fieldVisible(inputLayouts.getValue(endpoint), customEndpoint)
        ServiceSettingsUi.fieldLabel(inputLayouts.getValue(endpoint), activity.getString(if (selected == TextTranslationProvider.DEEPLX) R.string.translation_deeplx_endpoint else R.string.translation_endpoint))
        inputLayouts.getValue(endpoint).placeholderText = if (selected == TextTranslationProvider.DEEPLX) "https://example.com/translate" else "https://example.com/v1"
        ServiceSettingsUi.fieldVisible(inputLayouts.getValue(model), selected.usesOpenAIProtocol)
        localHttp.visibility = if (customEndpoint) View.VISIBLE else View.GONE
        rendering = false
        updateKeyLabel(); onModeChange(enabled)
    }

    fun draft(): TranslationConfiguration? {
        rememberDraft()
        if (!hasNetworkProvider()) return TranslationConfiguration(provider = selected)
        val state = drafts.getValue(selected)
        val config = TranslationConfiguration(
            endpoint = if (selected == TextTranslationProvider.DEEPL) "" else state.endpoint.trim(),
            model = if (selected.usesOpenAIProtocol) state.model.trim() else "",
            apiKey = state.key.trim().ifBlank { if (reusesSavedKey(state)) state.saved.apiKey else "" },
            allowLocalHttp = selected != TextTranslationProvider.DEEPL && state.localHttp,
            provider = selected,
        )
        if (selected == TextTranslationProvider.DEEPL && config.apiKey.isBlank()) {
            showError(R.string.translation_key_required); return null
        }
        if (selected != TextTranslationProvider.DEEPL && config.endpoint.isBlank()) {
            showError(R.string.translation_endpoint_required); return null
        }
        if (selected.usesOpenAIProtocol && config.model.isBlank()) {
            showError(R.string.translation_required); return null
        }
        try { validateTranslationConfiguration(config) }
        catch (error: IllegalArgumentException) {
            showError(if (error.message == "translation_key") R.string.translation_key_invalid else R.string.translation_invalid)
            return null
        }
        return config
    }

    private fun updateKeyLabel() {
        val state = drafts.getValue(selected)
        val hasSavedKey = reusesSavedKey(state)
        val label = activity.getString(when (selected) {
            TextTranslationProvider.DEEPL -> R.string.translation_deepl_key
            TextTranslationProvider.DEEPLX -> R.string.translation_token
            else -> R.string.translation_key
        })
        ServiceSettingsUi.fieldLabel(inputLayouts.getValue(key), if (hasSavedKey && state.key.isBlank()) activity.getString(R.string.translation_saved_field, label) else label)
        inputLayouts.getValue(key).placeholderText = when {
            hasSavedKey && state.key.isBlank() -> activity.getString(R.string.service_secret_saved)
            selected == TextTranslationProvider.DEEPL -> null
            else -> activity.getString(R.string.translation_key_optional)
        }
        removeKey.visibility = if (hasSavedKey && selected != TextTranslationProvider.DEEPL) View.VISIBLE else View.GONE
    }

    private fun hasNetworkProvider() = selected !in listOf(TextTranslationProvider.BUILTIN, TextTranslationProvider.NONE)
    private fun reusesSavedKey(state: Draft) = !state.forgetKey && state.saved.apiKey.isNotBlank() &&
        (selected == TextTranslationProvider.DEEPL || state.endpoint.trim() == state.saved.endpoint.trim())

    private fun showHelp() {
        val body = when (selected) {
            TextTranslationProvider.BUILTIN -> activity.getString(R.string.translation_help_builtin)
            TextTranslationProvider.NONE -> activity.getString(R.string.translation_help_none)
            else -> activity.getString(R.string.translation_help_speech) + "\n\n" + activity.getString(when (selected) {
                TextTranslationProvider.DEEPL -> R.string.translation_help_deepl
                TextTranslationProvider.DEEPLX -> R.string.translation_help_deeplx
                TextTranslationProvider.CHAT_MOCK -> R.string.translation_help_chatmock
                else -> R.string.translation_help
            }) + (if (selected == TextTranslationProvider.DEEPL) "" else "\n\n" + activity.getString(R.string.translation_help_network)) +
                "\n\n" + activity.getString(R.string.translation_help_save)
        }
        ServiceSettingsUi.showHelp(activity, activity.getString(translationProviderLabel(selected)), body)
    }

    private fun helpResource() = when (selected) {
        TextTranslationProvider.BUILTIN -> R.string.translation_help_builtin
        TextTranslationProvider.NONE -> R.string.translation_help_none
        TextTranslationProvider.DEEPL -> R.string.translation_help_deepl
        TextTranslationProvider.DEEPLX -> R.string.translation_help_deeplx
        TextTranslationProvider.CHAT_MOCK -> R.string.translation_help_chatmock
        else -> R.string.translation_help
    }

    private fun checkConnection() {
        val config = draft() ?: return
        if (!hasNetworkProvider()) return
        val epoch = ++generation
        request?.cancel()
        check.isEnabled = false; check.setText(R.string.translation_checking)
        result.visibility = View.GONE
        request = createTranslationClient(config).check { outcome -> activity.runOnUiThread {
            if (epoch != generation || activity.isDestroyed || activity.isFinishing) return@runOnUiThread
            request = null; check.isEnabled = true; check.setText(R.string.translation_check)
            result.visibility = View.VISIBLE
            result.text = when (outcome) {
                is TranslationResult.Success -> activity.getString(R.string.translation_check_success, outcome.elapsedMs)
                is TranslationResult.Failure -> activity.getString(when (outcome.code) {
                    "translation_http_401", "translation_http_403", "translation_rejected_401", "translation_rejected_403" -> R.string.translation_check_auth
                    "translation_http_404", "translation_rejected_404" -> R.string.translation_check_not_found
                    "translation_http_429", "translation_http_456", "translation_rejected_429", "translation_rejected_456" -> R.string.translation_check_busy
                    "translation_timeout" -> R.string.translation_check_timeout
                    "translation_response", "translation_empty_response", "translation_incomplete", "translation_too_large" -> R.string.translation_check_response
                    else -> R.string.translation_check_failure
                }, outcome.elapsedMs)
            }
            revealResult()
        } }
    }

    private fun showError(message: Int) {
        result.setText(message); result.visibility = View.VISIBLE; revealResult()
    }
    private fun revealResult() {
        result.doOnLayout {
            result.requestRectangleOnScreen(android.graphics.Rect(0, 0, result.width, result.height), false)
        }
    }
    private fun invalidateCheck() {
        ++generation; request?.cancel(); request = null
        check.isEnabled = true; check.setText(R.string.translation_check); result.visibility = View.GONE
    }
    fun dispose() { invalidateCheck() }
    private fun dp(value: Int) = ServiceSettingsUi.dp(activity, value)
    private fun field(title: Int, tag: String, secret: Boolean = false): TextInputEditText {
        val box = TextInputLayout(activity).apply {
            hint = activity.getString(title); isSaveEnabled = false
            if (secret) endIconMode = TextInputLayout.END_ICON_PASSWORD_TOGGLE
        }
        val edit = TextInputEditText(box.context).apply {
            this.tag = tag; textSize = 16f; isSingleLine = true; isSaveEnabled = false
            importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS
            inputType = InputType.TYPE_CLASS_TEXT or if (secret) InputType.TYPE_TEXT_VARIATION_PASSWORD else InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
            typeface = android.graphics.Typeface.DEFAULT
        }
        inputLayouts[edit] = box
        box.addView(edit, LinearLayout.LayoutParams(-1, -2))
        ServiceSettingsUi.addField(activity, fields, box, edit, activity.getString(title), dp(16))
        return edit
    }
}

internal fun translationProviderLabel(provider: TextTranslationProvider): Int = when (provider) {
    TextTranslationProvider.BUILTIN -> R.string.translation_builtin
    TextTranslationProvider.NONE -> R.string.translation_none
    TextTranslationProvider.CHAT_MOCK -> R.string.translation_chatmock
    TextTranslationProvider.OPENAI_COMPATIBLE -> R.string.translation_openai_compatible
    TextTranslationProvider.DEEPL -> R.string.translation_deepl
    TextTranslationProvider.DEEPLX -> R.string.translation_deeplx
}
