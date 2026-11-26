package app.yuxino.mimi.android.services.translation

import android.text.Editable
import android.text.InputType
import android.text.TextWatcher
import android.view.Gravity
import android.view.View
import android.widget.*
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import androidx.core.view.doOnLayout
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.provider.*
import app.yuxino.mimi.android.services.editor.ServiceSettingsActivity
import app.yuxino.mimi.android.services.editor.ServiceSettingsUi
import app.yuxino.mimi.android.services.editor.TextTranslationEditorDraft
import app.yuxino.mimi.android.services.editor.TextTranslationRouteDraft
import app.yuxino.mimi.android.services.setup.ServiceSetups
import app.yuxino.mimi.android.services.setup.TextTranslationPresets
import app.yuxino.mimi.android.services.setup.TextTranslationSetupPage
import app.yuxino.mimi.android.services.setup.openServiceSetup
import app.yuxino.mimi.android.services.setup.openTextTranslationSetup
import app.yuxino.mimi.android.services.setup.textTranslationServiceSetupId
import app.yuxino.mimi.android.settings.SettingsStore
import app.yuxino.mimi.android.ui.HelpUi
import com.google.android.material.button.MaterialButton
import com.google.android.material.textfield.TextInputEditText
import com.google.android.material.textfield.TextInputLayout

/** A draft only. ServiceSettingsActivity commits this alongside the speech configuration. */
internal class TextTranslationSettings(private val activity: AppCompatActivity, private val profileId: String? = null,
    editorDraft: TextTranslationEditorDraft? = null, private val speechProvider: ServiceProvider = ServiceProvider.DASHSCOPE) {
    private val providers = buildList {
        if (speechProvider == ServiceProvider.DASHSCOPE) add(TextTranslationProvider.BUILTIN)
        addAll(listOf(TextTranslationProvider.NONE, TextTranslationProvider.XAI,
            TextTranslationProvider.DEEPL, TextTranslationProvider.DEEPLX,
            TextTranslationProvider.OPENAI_COMPATIBLE, TextTranslationProvider.CHAT_MOCK))
    }
    private val drafts = providers.associateWith { route ->
        TextTranslationDraft(if (profileId == null) TranslationConfiguration(provider = route) else SettingsStore.profileTranslationConfiguration(activity, profileId, route)).apply {
            editorDraft?.routes?.get(route)?.let {
                endpoint = it.endpoint; model = it.model; key = it.key
                localHttp = it.localHttp; forgetKey = it.forgetKey
            }
        }
    }
    private var selected = (editorDraft?.selected ?: if (profileId == null) {
        when {
            speechProvider == ServiceProvider.XAI -> TextTranslationProvider.XAI
            speechProvider.supportsSpeechTranslationMode -> TextTranslationProvider.OPENAI_COMPATIBLE
            else -> TextTranslationProvider.BUILTIN
        }
    } else SettingsStore.profileTextTranslationProvider(activity, profileId)).let {
        if (it in providers) it else if (speechProvider.supportsSpeechTranslationMode) TextTranslationProvider.OPENAI_COMPATIBLE else TextTranslationProvider.NONE
    }
    private var rendering = false
    private val root = LinearLayout(activity).apply { orientation = LinearLayout.VERTICAL }
    private val fields = LinearLayout(activity).apply { orientation = LinearLayout.VERTICAL }
    private val mode = Spinner(activity).apply { tag = "translation-mode"; background = null; setPadding(0, 0, 0, 0); minimumHeight = dp(56) }
    private val preset = Spinner(activity).apply { tag = "translation-preset"; background = null; setPadding(0, 0, 0, 0); minimumHeight = dp(56); isSaveEnabled = false }
    private val presetRow = LinearLayout(activity).apply { orientation = LinearLayout.VERTICAL; tag = "translation-preset-row" }
    private val presetHelp: ImageButton
    private val setupNoKey = ServiceSettingsUi.label(activity, activity.getString(R.string.translation_setup_no_key), 16f).apply {
        tag = "translation-setup-no-key"; visibility = View.GONE
    }
    private val setupKey = setupButton(R.string.translation_setup_get_key, "translation-setup-key") {
        openSetup(TextTranslationSetupPage.API_KEY)
    }
    private val setupGuide = setupButton(R.string.translation_setup_guide, "translation-setup-guide") {
        openSetup(TextTranslationSetupPage.GUIDE)
    }
    private val setupActions = object : LinearLayout(activity) {
        init { gravity = Gravity.END or Gravity.CENTER_VERTICAL; tag = "translation-setup-actions" }
        override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
            val available = View.MeasureSpec.getSize(widthMeasureSpec) - paddingLeft - paddingRight
            val children = listOf(setupKey, setupGuide).filter { it.visibility != View.GONE }
            children.forEach { it.measure(View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED),
                View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)) }
            orientation = if (View.MeasureSpec.getMode(widthMeasureSpec) != View.MeasureSpec.UNSPECIFIED &&
                children.sumOf { it.measuredWidth } > available) VERTICAL else HORIZONTAL
            super.onMeasure(widthMeasureSpec, heightMeasureSpec)
        }
    }
    private var presetSelection = -1
    private var customPreset = editorDraft?.customPreset ?: false
    private val qwenModels = listOf("lite", "flash", "plus")
    private val qwenModel = Spinner(activity).apply { tag = "qwen-mt-model"; background = null; setPadding(0, 0, 0, 0) }
    private val qwenModelRow = LinearLayout(activity).apply { orientation = LinearLayout.VERTICAL; tag = "qwen-mt-model-row" }
    private val help: ImageButton
    private val inputLayouts = mutableMapOf<TextInputEditText, TextInputLayout>()
    private val endpoint = field(R.string.translation_endpoint, "translation-endpoint")
    private val model = field(R.string.translation_model, "translation-model")
    private val key = field(R.string.translation_key, "translation-key", true)
    private val directSetupKey = setupButton(R.string.translation_setup_get_key, "translation-service-setup-key") {
        if (key.isEnabled && providers.getOrNull(mode.selectedItemPosition) == selected) {
            textTranslationServiceSetupId(selected)?.let { openServiceSetup(activity, it, TextTranslationSetupPage.API_KEY) }
        }
    }
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
    private var editingEnabled = true
    val enabled: Boolean get() = selected != TextTranslationProvider.BUILTIN
    val qwenMtModel: String get() = qwenModels[qwenModel.selectedItemPosition.coerceIn(qwenModels.indices)]
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
        root.addView(mode, LinearLayout.LayoutParams(-1, -2).apply { bottomMargin = dp(16) })
        val presetHeading = HelpUi.heading(activity, activity.getString(R.string.translation_preset),
            activity.getString(R.string.translation_preset_help), 16f, "translation-preset-help")
        presetHelp = presetHeading.findViewWithTag("translation-preset-help")
        presetRow.addView(presetHeading)
        preset.adapter = TextTranslationPresetAdapter(activity)
        preset.contentDescription = activity.getString(R.string.translation_preset)
        presetRow.addView(preset, LinearLayout.LayoutParams(-1, -2))
        presetRow.addView(setupNoKey, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(8) })
        setupActions.addView(setupKey, LinearLayout.LayoutParams(-2, -2).apply { gravity = Gravity.END })
        setupActions.addView(setupGuide, LinearLayout.LayoutParams(-2, -2).apply { gravity = Gravity.END })
        presetRow.addView(setupActions, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(4) })
        root.addView(presetRow, LinearLayout.LayoutParams(-1, -2).apply { bottomMargin = dp(16) })
        qwenModel.adapter = ArrayAdapter(activity, R.layout.mimi_spinner_item,
            listOf("Qwen-MT Lite", "Qwen-MT Flash", "Qwen-MT Plus")).apply { setDropDownViewResource(R.layout.mimi_spinner_dropdown) }
        qwenModel.contentDescription = activity.getString(R.string.translation_model)
        qwenModel.setSelection(qwenModels.indexOf(editorDraft?.qwenMtModel ?: if (profileId == null) "lite" else SettingsStore.profileQwenMtModel(activity, profileId)))
        qwenModelRow.addView(ServiceSettingsUi.label(activity, activity.getString(R.string.translation_model), 16f))
        qwenModelRow.addView(qwenModel, LinearLayout.LayoutParams(-1, dp(56)))
        root.addView(qwenModelRow, LinearLayout.LayoutParams(-1, -2).apply { bottomMargin = dp(16) })
        fields.addView(directSetupKey, LinearLayout.LayoutParams(-2, -2).apply { gravity = Gravity.END; bottomMargin = dp(8) })
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
                rememberDraft(); syncPresetSelection(); invalidateCheck(); updateKeyLabel()
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
                applyPendingProvider()
            }
            override fun onNothingSelected(parent: AdapterView<*>?) = Unit
        }
        preset.onItemSelectedListener = object : AdapterView.OnItemSelectedListener {
            override fun onItemSelected(parent: AdapterView<*>?, view: View?, position: Int, id: Long) {
                if (position == preset.selectedItemPosition) applyPendingPreset()
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

    fun editorDraft(): TextTranslationEditorDraft {
        applyPendingProvider()
        applyPendingPreset()
        rememberDraft()
        return TextTranslationEditorDraft(selected, qwenMtModel, drafts.mapValues { (_, state) ->
            TextTranslationRouteDraft(state.endpoint, state.model, state.key, state.localHttp, state.forgetKey)
        }, customPreset)
    }

    private fun applyPendingProvider() {
        if (rendering) return
        val next = providers.getOrNull(mode.selectedItemPosition) ?: return
        if (next == selected) return
        // Spinner exposes a new selection before its callback is dispatched.
        // Save, Check and recreation must commit the old fields to their own
        // route before rendering and consuming the newly selected draft.
        applyPendingPreset()
        rememberDraft()
        selected = next
        renderDraft()
    }

    private fun renderDraft() {
        invalidateCheck(); rendering = true
        HelpUi.setMessage(help, activity.getString(helpResource()).substringBefore("\n\n"))
        val state = drafts.getValue(selected)
        endpoint.setText(state.endpoint); model.setText(state.model); key.setText(state.key)
        localHttp.isChecked = state.localHttp
        fields.visibility = if (hasNetworkProvider()) View.VISIBLE else View.GONE
        presetRow.visibility = if (selected == TextTranslationProvider.OPENAI_COMPATIBLE) View.VISIBLE else View.GONE
        syncPresetSelection()
        qwenModelRow.visibility = if (selected == TextTranslationProvider.BUILTIN) View.VISIBLE else View.GONE
        val customEndpoint = selected.hasCustomEndpoint
        ServiceSettingsUi.fieldVisible(inputLayouts.getValue(endpoint), customEndpoint)
        ServiceSettingsUi.fieldLabel(inputLayouts.getValue(endpoint), activity.getString(if (selected == TextTranslationProvider.DEEPLX) R.string.translation_deeplx_endpoint else R.string.translation_endpoint))
        inputLayouts.getValue(endpoint).placeholderText = if (selected == TextTranslationProvider.DEEPLX) "https://example.com/translate" else "https://example.com/v1"
        ServiceSettingsUi.fieldVisible(inputLayouts.getValue(model), selected.usesOpenAIProtocol)
        localHttp.visibility = if (customEndpoint) View.VISIBLE else View.GONE
        rendering = false
        val setup = textTranslationServiceSetupId(selected)?.let { id -> ServiceSetups.entries.firstOrNull { it.id == id } }
        directSetupKey.visibility = if (setup == null) View.GONE else View.VISIBLE
        directSetupKey.contentDescription = setup?.let { "${it.label}: ${directSetupKey.text}" }
        updateKeyLabel()
    }

    private fun syncPresetSelection() {
        val id = if (selected == TextTranslationProvider.OPENAI_COMPATIBLE && !customPreset) drafts.getValue(selected).presetId() else null
        presetSelection = TextTranslationPresets.entries.indexOfFirst { it.id == id } + 1
        if (preset.selectedItemPosition != presetSelection) preset.setSelection(presetSelection)
        val setup = TextTranslationPresets.entries.firstOrNull { it.id == id }
        HelpUi.setMessage(presetHelp, activity.getString(setup?.setupHelpResource ?: R.string.translation_preset_help))
        setupKey.visibility = if (setup?.let { it.setupKind == "cloud" && it.apiKeyUrl.isNotBlank() } == true) View.VISIBLE else View.GONE
        setupGuide.visibility = if (setup?.guideUrl?.isNotBlank() == true) View.VISIBLE else View.GONE
        setupGuide.setText(if (setup?.setupKind == "local") R.string.translation_setup_local_guide else R.string.translation_setup_guide)
        setupKey.contentDescription = setup?.let { "${it.label}: ${setupKey.text}" }
        setupGuide.contentDescription = setup?.let { "${it.label}: ${setupGuide.text}" }
        setupNoKey.visibility = if (setup?.setupKind == "keyless") View.VISIBLE else View.GONE
        setupActions.visibility = if (setup == null) View.GONE else View.VISIBLE
    }

    private fun openSetup(page: TextTranslationSetupPage) {
        // A browser action can arrive before Spinner's queued selection callback.
        // Resolve that selection before choosing a trusted destination or snapshotting.
        val action = if (page == TextTranslationSetupPage.API_KEY) setupKey else setupGuide
        if (!action.isEnabled ||
            providers.getOrNull(mode.selectedItemPosition) != selected) return
        applyPendingPreset()
        rememberDraft()
        if (selected != TextTranslationProvider.OPENAI_COMPATIBLE || customPreset) return
        val presetId = drafts.getValue(selected).presetId() ?: return
        openTextTranslationSetup(activity, presetId, page)
    }

    private fun applyPendingPreset() {
        if (rendering || selected != TextTranslationProvider.OPENAI_COMPATIBLE) return
        val position = preset.selectedItemPosition
        if (position == presetSelection) return
        presetSelection = position
        // Explicit Custom stays selected while this editor's fields are edited.
        val choice = TextTranslationPresets.entries.getOrNull(position - 1)
        customPreset = choice == null
        if (choice == null) { syncPresetSelection(); updateKeyLabel(); return }
        rememberDraft()
        drafts.getValue(selected).applyPreset(choice)
        renderDraft()
    }

    fun draft(validate: Boolean = true): TranslationConfiguration? {
        applyPendingProvider()
        applyPendingPreset()
        rememberDraft()
        if (!hasNetworkProvider()) return TranslationConfiguration(provider = selected)
        val state = drafts.getValue(selected)
        val config = state.configuration()
        if (!validate) return config
        if (selected in listOf(TextTranslationProvider.DEEPL, TextTranslationProvider.XAI) && config.apiKey.isBlank()) {
            showError(R.string.translation_key_required); return null
        }
        if (selected.hasCustomEndpoint && config.endpoint.isBlank()) {
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
        val hasSavedKey = state.reusesSavedKey()
        val cloudPreset = selected == TextTranslationProvider.OPENAI_COMPATIBLE && !customPreset &&
            TextTranslationPresets.entries.getOrNull(presetSelection - 1)?.setupKind == "cloud"
        val label = activity.getString(when (selected) {
            TextTranslationProvider.DEEPL -> R.string.translation_deepl_key
            TextTranslationProvider.DEEPLX -> R.string.translation_token
            else -> R.string.translation_key
        })
        ServiceSettingsUi.fieldLabel(inputLayouts.getValue(key), if (hasSavedKey && state.key.isBlank()) activity.getString(R.string.translation_saved_field, label) else label)
        inputLayouts.getValue(key).placeholderText = when {
            hasSavedKey && state.key.isBlank() -> activity.getString(R.string.service_secret_saved)
            selected in listOf(TextTranslationProvider.DEEPL, TextTranslationProvider.XAI) || cloudPreset -> null
            else -> activity.getString(R.string.translation_key_optional)
        }
        removeKey.visibility = if (hasSavedKey && selected !in listOf(TextTranslationProvider.DEEPL, TextTranslationProvider.XAI)) View.VISIBLE else View.GONE
    }

    private fun hasNetworkProvider() = selected !in listOf(TextTranslationProvider.BUILTIN, TextTranslationProvider.NONE)

    private fun showHelp() {
        val body = when (selected) {
            TextTranslationProvider.BUILTIN -> activity.getString(R.string.translation_help_builtin)
            TextTranslationProvider.NONE -> activity.getString(if (speechProvider == ServiceProvider.XAI) R.string.translation_help_xai_original else R.string.translation_help_none)
            else -> activity.getString(if (speechProvider == ServiceProvider.XAI) R.string.xai_help else R.string.translation_help_speech) + "\n\n" + activity.getString(when (selected) {
                TextTranslationProvider.DEEPL -> R.string.translation_help_deepl
                TextTranslationProvider.DEEPLX -> R.string.translation_help_deeplx
                TextTranslationProvider.CHAT_MOCK -> R.string.translation_help_chatmock
                TextTranslationProvider.XAI -> R.string.translation_help_xai
                else -> R.string.translation_help
            }) + (if (!selected.hasCustomEndpoint) "" else "\n\n" + activity.getString(R.string.translation_help_network)) +
                "\n\n" + activity.getString(R.string.profiles_translation_help_save)
        }
        HelpUi.showDetails(activity, activity.getString(translationProviderLabel(selected)), body)
    }

    private fun helpResource() = when (selected) {
        TextTranslationProvider.BUILTIN -> R.string.translation_help_builtin
        TextTranslationProvider.NONE -> if (speechProvider == ServiceProvider.XAI) R.string.translation_help_xai_original else R.string.translation_help_none
        TextTranslationProvider.DEEPL -> R.string.translation_help_deepl
        TextTranslationProvider.DEEPLX -> R.string.translation_help_deeplx
        TextTranslationProvider.CHAT_MOCK -> R.string.translation_help_chatmock
        TextTranslationProvider.XAI -> R.string.translation_help_xai
        else -> R.string.translation_help
    }

    private fun checkConnection() {
        if (!editingEnabled || request != null) return
        val config = draft() ?: return
        if (!hasNetworkProvider()) return
        val epoch = ++generation
        request?.cancel()
        check.isEnabled = false; check.setText(R.string.translation_checking)
        result.visibility = View.GONE
        val probe = try {
            // The check uses only text-service fields. A speech key or an unsaved recognition mode is irrelevant.
            val (source, target) = speechProvider.normalize(SettingsStore.sourceLang(activity),
                SettingsStore.targetLang(activity).takeUnless { it == "original" } ?: "zh", config.provider,
                xaiSpeechMode = if (speechProvider == ServiceProvider.XAI) "asrMt" else "voice",
                speechTranslationMode = if (speechProvider.supportsSpeechTranslationMode) "asrMt" else "simultaneous")
            buildTextProbeConfiguration(config, source, target, qwenMtModel)
        } catch (_: Exception) {
            updateCheckEnabled(); check.setText(R.string.translation_check)
            showError(R.string.translation_invalid); return
        }
        request = NativeTextProbeClient().check(probe) { outcome -> activity.runOnUiThread {
            if (epoch != generation || activity.isDestroyed || activity.isFinishing) return@runOnUiThread
            request = null; updateCheckEnabled(); check.setText(R.string.translation_check)
            result.visibility = View.VISIBLE
            result.text = when (outcome) {
                is TranslationResult.Success -> activity.getString(R.string.translation_check_success, outcome.elapsedMs)
                is TranslationResult.Failure -> activity.getString(when (outcome.code) {
                    "translation_http_401" -> R.string.translation_check_auth
                    "translation_http_429" -> R.string.translation_check_busy
                    "translation_timeout" -> R.string.translation_check_timeout
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
        updateCheckEnabled(); check.setText(R.string.translation_check); result.visibility = View.GONE
    }
    private fun updateCheckEnabled() { check.isEnabled = editingEnabled && request == null }
    fun dispose() {
        invalidateCheck()
        key.transformationMethod = android.text.method.PasswordTransformationMethod.getInstance()
    }
    fun setEditingEnabled(enabled: Boolean) {
        editingEnabled = enabled
        if (!enabled) invalidateCheck()
        listOf(mode, preset, setupKey, setupGuide, directSetupKey, qwenModel, endpoint, model, key, localHttp, removeKey).forEach { it.isEnabled = enabled }
        updateCheckEnabled()
    }
    private fun dp(value: Int) = ServiceSettingsUi.dp(activity, value)
    private fun setupButton(label: Int, tag: String, action: () -> Unit) =
        MaterialButton(activity, null, com.google.android.material.R.attr.borderlessButtonStyle).apply {
            this.tag = tag; setText(label); textSize = 16f; isAllCaps = false; isSingleLine = false
            minimumHeight = dp(48)
            setTextColor(ContextCompat.getColor(activity, R.color.mimi_text))
            setOnClickListener { action() }
        }
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
            if (secret) transformationMethod = android.text.method.PasswordTransformationMethod.getInstance()
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
    TextTranslationProvider.XAI -> R.string.translation_xai
    TextTranslationProvider.DEEPL -> R.string.translation_deepl
    TextTranslationProvider.DEEPLX -> R.string.translation_deeplx
}
