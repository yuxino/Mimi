package app.yuxino.mimi.android.services.editor

import android.os.Bundle
import android.text.InputType
import android.view.Gravity
import android.view.View
import android.view.WindowManager
import android.widget.ImageButton
import android.widget.LinearLayout
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import androidx.lifecycle.ViewModelProvider
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.provider.*
import app.yuxino.mimi.android.services.presentation.serviceProviderIcon
import app.yuxino.mimi.android.services.setup.ServiceSetups
import app.yuxino.mimi.android.services.setup.TextTranslationSetupPage
import app.yuxino.mimi.android.services.setup.openServiceSetup
import app.yuxino.mimi.android.services.setup.providerHelp
import app.yuxino.mimi.android.services.setup.speechProviderName
import app.yuxino.mimi.android.services.translation.TextTranslationSettings
import app.yuxino.mimi.android.settings.SettingsStore
import app.yuxino.mimi.android.ui.HelpUi
import app.yuxino.mimi.android.ui.layout.ControlScrollView
import app.yuxino.mimi.android.ui.layout.applySystemBarInsets
import com.google.android.material.button.MaterialButton
import com.google.android.material.textfield.TextInputEditText
import com.google.android.material.textfield.TextInputLayout

class ServiceSettingsActivity : AppCompatActivity() {
    private val inputs = linkedMapOf<String, Pair<TextInputLayout,TextInputEditText>>()
    private lateinit var provider: ServiceProvider
    private lateinit var saved: ServiceConfiguration
    private var storageUnavailable = false
    private var profileId: String? = null
    private lateinit var nameInput: TextInputEditText
    private var alibabaMode: android.widget.Spinner? = null
    private var workspaceInput: Pair<TextInputLayout, TextInputEditText>? = null
    private var moreButton: ImageButton? = null
    private var translationSettings: TextTranslationSettings? = null
    private var speechSettings: SpeechModeSettings? = null
    private lateinit var draftHolder: ServiceEditorDraftViewModel
    private lateinit var status: android.widget.TextView
    private var sessionBlocked = false
    private var setupKey: MaterialButton? = null
    private val sessionListener: () -> Unit = { runOnUiThread { refreshSessionUi() } }
    private fun dp(value:Int)=ServiceSettingsUi.dp(this,value)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        profileId = intent.getStringExtra("profileId")
        draftHolder = ViewModelProvider(this)[ServiceEditorDraftViewModel::class.java]
        val profile = runCatching { profileId?.let { id -> SettingsStore.profiles(this).firstOrNull { it.id == id } ?: error("profile_missing") } }
            .getOrElse { storageUnavailable = true; null }
        provider = profile?.provider ?: (if (storageUnavailable) draftHolder.providerFor(profileId) else null)
            ?: ServiceProvider.fromId(intent.getStringExtra("provider").orEmpty())
        saved = runCatching {
            profileId?.let { SettingsStore.profileConfiguration(this, it) }
                ?: ServiceConfiguration(provider, emptyMap(), alibabaRealtimeMode = "liveTranslate",
                    xaiSpeechMode = if (provider == ServiceProvider.XAI) "asrMt" else "voice")
        }.getOrElse { storageUnavailable = true; ServiceConfiguration(provider, emptyMap()) }
        val editorDraft = draftHolder.restore(profileId, provider)
        val root=LinearLayout(this).apply { orientation=LinearLayout.VERTICAL }
        val header=LinearLayout(this).apply { gravity=Gravity.CENTER_VERTICAL; minimumHeight=dp(64); setPadding(dp(12),dp(8),dp(12),dp(8)) }
        header.addView(ImageButton(this).apply {
            id=R.id.back; setImageResource(R.drawable.ic_back); setBackgroundColor(android.graphics.Color.TRANSPARENT)
            imageTintList=ContextCompat.getColorStateList(context,R.color.mimi_text)
            contentDescription=getString(R.string.settings_back); setOnClickListener { finish() }
        },LinearLayout.LayoutParams(dp(48),dp(48)))
        header.addView(android.widget.ImageView(this).apply {
            setImageResource(serviceProviderIcon(provider))
            scaleType=android.widget.ImageView.ScaleType.FIT_CENTER
            importantForAccessibility=View.IMPORTANT_FOR_ACCESSIBILITY_NO
        }, LinearLayout.LayoutParams(dp(28),dp(28)).apply { marginEnd=dp(10) })
        val helpHeading = HelpUi.heading(this, speechProviderName(this, provider), getString(providerHelp(provider).setup), 21f, "speech-help")
        helpHeading.findViewWithTag<ImageButton>("speech-help").apply {
            setOnClickListener {
                val help = providerHelp(provider)
                val message = android.text.SpannableStringBuilder(getString(if (provider == ServiceProvider.DASHSCOPE) R.string.profiles_alibaba_help else help.setup))
                help.setupLinks.forEach { link ->
                    message.append("\n\n")
                    val start = message.length
                    message.append(getString(link.label))
                    message.setSpan(object : android.text.style.ClickableSpan() {
                        override fun onClick(widget: View) { openHelp(link.url) }
                    }, start, message.length, android.text.Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
                }
                message.append("\n\n").append(getString(R.string.guide_local_save))
                ServiceSettingsUi.showHelp(this@ServiceSettingsActivity, speechProviderName(this@ServiceSettingsActivity, provider), message,
                    listOf(R.string.guide_official to { openHelp(help.documentation) },
                        R.string.guide_billing to { openHelp(help.billing) }))
            }
        }
        header.addView(helpHeading, LinearLayout.LayoutParams(0, -2, 1f))
        if (profile != null) {
            moreButton = ImageButton(this).apply {
                tag = "profile-more"; contentDescription = getString(R.string.navigation_more)
                setImageResource(androidx.appcompat.R.drawable.abc_ic_menu_overflow_material)
                imageTintList = ContextCompat.getColorStateList(context, R.color.mimi_muted)
                val typed = android.util.TypedValue()
                theme.resolveAttribute(android.R.attr.selectableItemBackgroundBorderless, typed, true)
                setBackgroundResource(typed.resourceId); setPadding(dp(12), dp(12), dp(12), dp(12))
                setOnClickListener {
                    if (MimiService.isRunning || MimiService.isStopping) return@setOnClickListener
                    ServiceSettingsUi.showProfileMenu(this@ServiceSettingsActivity, profile) { deleted ->
                        if (deleted) finish()
                        else Toast.makeText(this@ServiceSettingsActivity, R.string.profiles_duplicated, Toast.LENGTH_SHORT).show()
                    }
                }
            }
            header.addView(moreButton, LinearLayout.LayoutParams(dp(48), dp(48)))
        }
        root.addView(header,LinearLayout.LayoutParams(-1,-2))
        val scroll=ControlScrollView(this).apply { isFillViewport=true }
        val content=LinearLayout(this).apply { orientation=LinearLayout.VERTICAL; setPadding(dp(24),dp(12),dp(24),dp(24)) }
        status=ServiceSettingsUi.label(this,"",16f).apply {
            visibility=View.GONE; accessibilityLiveRegion=View.ACCESSIBILITY_LIVE_REGION_POLITE
        }
        if ((MimiService.isRunning || MimiService.isStopping)) { status.setText(R.string.service_stop_first); status.visibility=View.VISIBLE }
        if (storageUnavailable) { status.text=getString(R.string.guide_storage_unavailable); status.visibility=View.VISIBLE }
        content.addView(status,LinearLayout.LayoutParams(-1,-2).apply { bottomMargin=dp(16) })
        nameInput = field(content, "profile-name", getString(R.string.profiles_name), false).second.apply {
            tag = "profile-name"; setText(editorDraft?.name ?: profile?.name ?: speechProviderName(this@ServiceSettingsActivity, provider))
            filters = arrayOf(android.text.InputFilter.LengthFilter(80))
        }
        if (provider == ServiceProvider.DASHSCOPE) {
            content.addView(HelpUi.heading(this, getString(R.string.profiles_alibaba_mode), getString(R.string.profiles_alibaba_help), 18f, "alibaba-mode-help"))
            alibabaMode = android.widget.Spinner(this).apply {
                tag = "alibaba-mode"; background = null
                adapter = android.widget.ArrayAdapter(this@ServiceSettingsActivity, R.layout.mimi_spinner_item,
                    listOf(getString(R.string.profiles_alibaba_asr_mt), getString(R.string.profiles_alibaba_live))).apply { setDropDownViewResource(R.layout.mimi_spinner_dropdown) }
                contentDescription = getString(R.string.profiles_alibaba_mode)
                setSelection(editorDraft?.alibabaMode ?: if (saved.usesAlibabaLiveTranslate) 1 else 0)
            }
            content.addView(alibabaMode, LinearLayout.LayoutParams(-1, dp(56)).apply { bottomMargin = dp(16) })
            workspaceInput = field(content, "alibaba-workspace", getString(R.string.profiles_alibaba_workspace), false).also { it.second.setText(editorDraft?.workspace ?: saved.alibabaWorkspaceId) }
        }
        provider.fields.forEach { field ->
            val title = when (field.id) {
                "endpoint" -> getString(R.string.guide_field_endpoint)
                "deployment" -> getString(R.string.guide_field_deployment)
                "transcriptionDeployment" -> getString(R.string.guide_field_transcription)
                else -> field.label
            }
            val pair=field(content,field.id,title,field.secret)
            if(field.secret) {
                if(saved.value(field.id).isNotBlank()) {
                    ServiceSettingsUi.fieldLabel(pair.first,getString(R.string.translation_saved_field,title))
                    pair.first.placeholderText=getString(R.string.service_secret_saved)
                }
            } else pair.second.setText(saved.value(field.id))
            editorDraft?.speechValues?.get(field.id)?.let { pair.second.setText(it) }
            inputs[field.id]=pair
            val keyField = when (provider) {
                ServiceProvider.TENCENT -> "secretKey"
                ServiceProvider.BAIDU -> "appKey"
                else -> "apiKey"
            }
            if (field.id == keyField) {
                val setup = ServiceSetups.entries.firstOrNull { it.id == provider.wireProvider }
                if (setup != null) {
                    setupKey = MaterialButton(this, null, com.google.android.material.R.attr.borderlessButtonStyle).apply {
                        tag = "speech-setup-key"; setText(R.string.translation_setup_get_key)
                        textSize = 16f; isAllCaps = false; isSingleLine = false; minimumHeight = dp(48)
                        setTextColor(ContextCompat.getColor(context, R.color.mimi_text))
                        contentDescription = "${setup.label}: ${getString(R.string.translation_setup_get_key)}"
                        setOnClickListener { if (isEnabled) openServiceSetup(this@ServiceSettingsActivity, setup.id, TextTranslationSetupPage.API_KEY) }
                    }
                    content.addView(setupKey, LinearLayout.LayoutParams(-2, -2).apply { gravity = Gravity.END; bottomMargin = dp(8) })
                }
            }
        }
        if (provider == ServiceProvider.XAI || provider.supportsSpeechTranslationMode) {
            speechSettings = SpeechModeSettings(this, saved, editorDraft, ::speechProbeConfiguration, ::updateSpeechMode)
            content.addView(speechSettings!!.view, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(12) })
            inputs.values.forEach { (_, edit) -> edit.addTextChangedListener(object : android.text.TextWatcher {
                override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) = Unit
                override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) { speechSettings?.invalidate() }
                override fun afterTextChanged(s: android.text.Editable?) = Unit
            }) }
        }
        if ((provider in listOf(ServiceProvider.DASHSCOPE, ServiceProvider.XAI) || provider.supportsSpeechTranslationMode) && !storageUnavailable) {
            translationSettings = runCatching { TextTranslationSettings(this, profileId, editorDraft?.translation, provider) }.getOrElse {
                storageUnavailable = true; status.setText(R.string.guide_storage_unavailable); status.visibility = View.VISIBLE; null
            }
            translationSettings?.let { content.addView(it.view, LinearLayout.LayoutParams(-1,-2).apply { topMargin=dp(12) }) }
        }
        alibabaMode?.onItemSelectedListener = object : android.widget.AdapterView.OnItemSelectedListener {
            override fun onItemSelected(parent: android.widget.AdapterView<*>?, view: View?, position: Int, id: Long) { updateSpeechMode() }
            override fun onNothingSelected(parent: android.widget.AdapterView<*>?) = Unit
        }
        updateSpeechMode()
        scroll.addView(content); root.addView(scroll,LinearLayout.LayoutParams(-1,0,1f))
        val footer=LinearLayout(this).apply { orientation=LinearLayout.VERTICAL; setPadding(dp(24),dp(8),dp(24),dp(12)) }
        footer.addView(MaterialButton(this).apply {
            id=R.id.save; text=getString(R.string.profiles_save); isSingleLine=false
            setPadding(paddingLeft,dp(12),paddingRight,dp(12)); setOnClickListener { save() }
        },LinearLayout.LayoutParams(-1,-2))
        root.addView(footer); setContentView(root); applySystemBarInsets()
        window.setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE)
    }
    private fun field(container:LinearLayout, tag:String, title:String, secret:Boolean):Pair<TextInputLayout,TextInputEditText> {
        val box=TextInputLayout(this).apply {
            hint=title; if(secret) endIconMode=TextInputLayout.END_ICON_PASSWORD_TOGGLE
            isSaveEnabled=false
        }
        val edit=TextInputEditText(box.context).apply {
            this.tag="credential-$tag"; isSaveEnabled=false; importantForAutofill=View.IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS
            inputType=if(secret) InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD else InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
            isSingleLine=true; textSize=16f; typeface=android.graphics.Typeface.DEFAULT
            if (secret) transformationMethod = android.text.method.PasswordTransformationMethod.getInstance()
        }
        box.addView(edit,LinearLayout.LayoutParams(-1,-2))
        ServiceSettingsUi.addField(this,container,box,edit,title,dp(18))
        return box to edit
    }
    override fun finish() {
        if (::draftHolder.isInitialized) draftHolder.clearDraft()
        super.finish()
    }
    override fun onDestroy() {
        if (isFinishing && ::draftHolder.isInitialized) draftHolder.clearDraft()
        translationSettings?.dispose(); speechSettings?.dispose(); super.onDestroy()
    }
    override fun onStart() { super.onStart(); MimiService.addStateListener(sessionListener); refreshSessionUi() }
    override fun onStop() {
        MimiService.removeStateListener(sessionListener)
        if (::draftHolder.isInitialized && ::nameInput.isInitialized) {
            if (isFinishing) draftHolder.clearDraft()
            else draftHolder.remember(profileId, provider, ServiceEditorDraft(
                nameInput.text.toString(), inputs.mapValues { (_, field) -> field.second.text.toString() },
                workspaceInput?.second?.text?.toString(), alibabaMode?.selectedItemPosition,
                // An unavailable credential store must not erase the previous
                // route drafts; all storage/session guards still disable saves.
                translationSettings?.editorDraft() ?: draftHolder.restore(profileId, provider)?.translation,
                speechSettings?.selectedPosition?.takeIf { provider == ServiceProvider.XAI },
                speechSettings?.selectedPosition?.takeIf { provider.supportsSpeechTranslationMode },
            ))
        }
        translationSettings?.dispose()
        speechSettings?.dispose()
        provider.fields.filter { it.secret }.forEach { field ->
            inputs[field.id]?.second?.transformationMethod = android.text.method.PasswordTransformationMethod.getInstance()
        }
        super.onStop()
    }
    private fun refreshSessionUi() {
        val running = (MimiService.isRunning || MimiService.isStopping)
        nameInput.isEnabled = !running && !storageUnavailable
        moreButton?.isEnabled = !running
        alibabaMode?.isEnabled = !running && !storageUnavailable
        workspaceInput?.second?.isEnabled = !running && !storageUnavailable
        inputs.values.forEach { (_, edit) -> edit.isEnabled = !running && !storageUnavailable }
        setupKey?.isEnabled = !running && !storageUnavailable
        translationSettings?.setEditingEnabled(!running && !storageUnavailable)
        speechSettings?.setEditingEnabled(!running && !storageUnavailable)
        findViewById<View>(R.id.save).isEnabled = !running && !storageUnavailable
        if (running) { status.setText(R.string.service_stop_first); status.visibility = View.VISIBLE }
        else if (sessionBlocked && !storageUnavailable && status.text == getString(R.string.service_stop_first)) status.visibility = View.GONE
        sessionBlocked = running
    }
    private fun openHelp(url: String) {
        try { startActivity(android.content.Intent(android.content.Intent.ACTION_VIEW, android.net.Uri.parse(url))) }
        catch (_: android.content.ActivityNotFoundException) { Toast.makeText(this, R.string.guide_no_browser, Toast.LENGTH_SHORT).show() }
    }
    private fun updateSpeechMode() {
        val live = alibabaMode?.selectedItemPosition == 1
        workspaceInput?.let { ServiceSettingsUi.fieldVisible(it.first, live) }
        val integrated = live || provider == ServiceProvider.XAI && speechSettings?.speechMode == "voice" ||
            provider.supportsSpeechTranslationMode && speechSettings?.speechMode != "asrMt"
        if (provider == ServiceProvider.AZURE) inputs["deployment"]?.let {
            ServiceSettingsUi.fieldVisible(it.first, integrated)
            if (!integrated) it.first.error = null
        }
        translationSettings?.view?.visibility = if (integrated) View.GONE else View.VISIBLE
        if (integrated) translationSettings?.dispose()
    }
    private fun speechDraftValues(mode: String): Map<String, String>? {
        var valid = true
        val values = provider.fields.associate { field ->
            val (layout, edit) = inputs.getValue(field.id)
            val value = edit.text.toString().trim()
            val required = !field.secret && !(provider == ServiceProvider.AZURE && field.id == "deployment" && mode == "asrMt")
            layout.error = if (required && value.isBlank()) getString(R.string.service_required) else null
            if (required && value.isBlank()) valid = false
            field.id to value
        }
        if (!valid) return null
        return try {
            resolveSpeechCredentialDraft(saved, values, mode)
        } catch (error: Exception) {
            // Blank secrets can be retained only by the shared owner rules.
            // An address/account change requires an explicit replacement.
            if (error is IllegalArgumentException && error.message == "missing_credentials_draft") {
                provider.fields.filter { it.secret && values[it.id].isNullOrBlank() }.forEach { field ->
                    inputs.getValue(field.id).first.error = getString(R.string.service_required)
                }
            }
            status.setText(R.string.service_invalid); status.visibility = View.VISIBLE
            null
        }
    }
    private fun speechProbeConfiguration(): org.json.JSONObject? {
        if (storageUnavailable || MimiService.isRunning || MimiService.isStopping || speechSettings == null) return null
        val values = speechDraftValues(if (provider.supportsSpeechTranslationMode) speechSettings!!.speechMode else saved.speechTranslationMode) ?: return null
        val config = ServiceConfiguration(provider, values, saved.endpoint, saved.model,
            xaiSpeechMode = if (provider == ServiceProvider.XAI) speechSettings!!.speechMode else saved.xaiSpeechMode,
            xaiTextModel = saved.xaiTextModel,
            speechTranslationMode = if (provider.supportsSpeechTranslationMode) speechSettings!!.speechMode else saved.speechTranslationMode)
        return runCatching {
            val (source, _) = provider.normalize(SettingsStore.sourceLang(this), "original", TextTranslationProvider.NONE,
                xaiSpeechMode = config.xaiSpeechMode, speechTranslationMode = config.speechTranslationMode)
            buildSpeechProbeConfiguration(config, source).also {
                if (status.text.toString() == getString(R.string.service_invalid)) status.visibility = View.GONE
            }
        }.getOrElse { status.setText(R.string.service_invalid); status.visibility = View.VISIBLE; null }
    }
    private fun save() {
        status.visibility=View.VISIBLE
        if(storageUnavailable) return
        val live = alibabaMode?.selectedItemPosition == 1
        val integrated = live || provider == ServiceProvider.XAI && speechSettings?.speechMode == "voice" ||
            provider.supportsSpeechTranslationMode && speechSettings?.speechMode != "asrMt"
        val retainDraft = provider != ServiceProvider.XAI || !integrated
        val textTranslation = if (retainDraft) translationSettings?.draft(validate = !integrated) else null
        if(retainDraft && translationSettings != null && textTranslation == null) { status.visibility=View.GONE; return }
        if((MimiService.isRunning || MimiService.isStopping)) { status.setText(R.string.service_stop_first); Toast.makeText(this,R.string.service_stop_first,Toast.LENGTH_SHORT).show(); return }
        var valid=true
        val profileName = nameInput.text.toString().trim()
        if (profileName.isBlank()) { nameInput.error = getString(R.string.service_required); nameInput.requestFocus(); status.visibility = View.GONE; return }
        val workspace = workspaceInput?.second?.text?.toString()?.trim().orEmpty()
        workspaceInput?.first?.error = if (live && workspace.isBlank()) getString(R.string.service_required) else null
        if (live && workspace.isBlank()) valid = false
        if(!valid) { status.visibility = View.GONE; return }
        val values = speechDraftValues(if (provider.supportsSpeechTranslationMode) speechSettings?.speechMode ?: saved.speechTranslationMode else saved.speechTranslationMode) ?: return
        // The shared factory owns speech models and endpoints. Retain old overrides
        // for compatibility without offering editable fields that the runtime ignores.
        val config=ServiceConfiguration(provider, values, saved.endpoint, saved.model,
            alibabaRealtimeMode = if (live) "liveTranslate" else "asrMt", alibabaWorkspaceId = workspace,
            xaiSpeechMode = if (provider == ServiceProvider.XAI) speechSettings?.speechMode ?: saved.xaiSpeechMode else saved.xaiSpeechMode,
            xaiTextModel = saved.xaiTextModel,
            speechTranslationMode = if (provider.supportsSpeechTranslationMode) speechSettings?.speechMode ?: saved.speechTranslationMode else saved.speechTranslationMode)
        val (source, target) = runCatching {
            provider.normalize(SettingsStore.sourceLang(this), SettingsStore.targetLang(this), textTranslation?.provider ?: TextTranslationProvider.BUILTIN, config.alibabaRealtimeMode, config.xaiSpeechMode, config.speechTranslationMode)
        }.getOrElse { status.text = getString(R.string.guide_storage_unavailable); return }
        // Construct the same factory as PC locally, without connecting or logging secrets.
        try {
            NativeRuntimeConfiguration.endpoints(buildRuntimeConfiguration(config,
                textTranslation ?: TranslationConfiguration(provider = TextTranslationProvider.BUILTIN),
                source, target, translationSettings?.qwenMtModel ?: "lite"))
        } catch (_:Exception) { status.text=getString(R.string.service_invalid); return }
        val savedId = runCatching { SettingsStore.saveProfile(this, profileId, profileName, config, textTranslation,
            if (retainDraft) translationSettings?.enabled else null, translationSettings?.qwenMtModel, activate = false) }.getOrNull()
        if (savedId == null) {
            status.text=getString(R.string.guide_storage_unavailable); return
        }
        setResult(android.app.Activity.RESULT_OK, android.content.Intent().putExtra("profileId", savedId))
        Toast.makeText(this,R.string.settings_saved,Toast.LENGTH_SHORT).show(); finish()
    }
}
