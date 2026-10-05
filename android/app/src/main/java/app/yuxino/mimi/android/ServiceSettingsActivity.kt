package app.yuxino.mimi.android

import android.os.Bundle
import android.text.InputType
import android.view.Gravity
import android.view.View
import android.view.WindowManager
import android.widget.ImageButton
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.provider.*
import com.google.android.material.button.MaterialButton
import com.google.android.material.textfield.TextInputEditText
import com.google.android.material.textfield.TextInputLayout

class ServiceSettingsActivity : AppCompatActivity() {
    private val inputs = linkedMapOf<String, Pair<TextInputLayout,TextInputEditText>>()
    private lateinit var provider: ServiceProvider
    private lateinit var saved: ServiceConfiguration
    private var storageUnavailable = false
    private lateinit var endpointInput: TextInputEditText
    private lateinit var modelInput: TextInputEditText
    private lateinit var modelLayout: TextInputLayout
    private var hotwordsLayout: TextInputLayout? = null
    private var hotwordsInput: TextInputEditText? = null
    private var translationSettings: TextTranslationSettings? = null
    private lateinit var status: android.widget.TextView
    private fun dp(value:Int)=ServiceSettingsUi.dp(this,value)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        provider=ServiceProvider.fromId(intent.getStringExtra("provider").orEmpty())
        saved=runCatching { SettingsStore.configuration(this,provider) }.getOrElse { storageUnavailable = true; ServiceConfiguration(provider, emptyMap()) }
        val root=LinearLayout(this).apply { orientation=LinearLayout.VERTICAL }
        val header=LinearLayout(this).apply { gravity=Gravity.CENTER_VERTICAL; setPadding(dp(12),0,dp(24),0) }
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
        header.addView(ServiceSettingsUi.label(this,providerTitle(this, provider),21f).apply { maxLines=2; ellipsize=android.text.TextUtils.TruncateAt.END }, LinearLayout.LayoutParams(0,-2,1f))
        header.addView(helpButton(this, R.string.translation_speech_help_title, providerHelp(provider).setup, "speech-help").apply {
            setOnClickListener {
                val help = providerHelp(provider)
                val message = android.text.SpannableStringBuilder(getString(help.setup))
                help.setupLinks.forEach { link ->
                    message.append("\n\n")
                    val start = message.length
                    message.append(getString(link.label))
                    message.setSpan(object : android.text.style.ClickableSpan() {
                        override fun onClick(widget: View) { openHelp(link.url) }
                    }, start, message.length, android.text.Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
                }
                message.append("\n\n").append(getString(R.string.guide_local_save))
                val dialog = com.google.android.material.dialog.MaterialAlertDialogBuilder(this@ServiceSettingsActivity)
                    .setTitle(providerTitle(this@ServiceSettingsActivity, provider))
                    .setMessage(message)
                    .setPositiveButton(android.R.string.ok, null)
                    .setNeutralButton(R.string.guide_official) { _, _ -> openHelp(help.documentation) }
                    .setNegativeButton(R.string.guide_billing) { _, _ -> openHelp(help.billing) }.show()
                dialog.findViewById<android.widget.TextView>(android.R.id.message)?.movementMethod = android.text.method.LinkMovementMethod.getInstance()
            }
        }, LinearLayout.LayoutParams(dp(48),dp(48)))
        root.addView(header,LinearLayout.LayoutParams(-1,dp(64)))
        val scroll=ScrollView(this).apply { isFillViewport=true }
        val content=LinearLayout(this).apply { orientation=LinearLayout.VERTICAL; setPadding(dp(24),dp(12),dp(24),dp(24)) }
        status=ServiceSettingsUi.label(this,"",16f).apply {
            visibility=View.GONE; accessibilityLiveRegion=View.ACCESSIBILITY_LIVE_REGION_POLITE
        }
        if (storageUnavailable) { status.text=getString(R.string.guide_storage_unavailable); status.visibility=View.VISIBLE }
        content.addView(status,LinearLayout.LayoutParams(-1,-2).apply { bottomMargin=dp(16) })
        if (provider == ServiceProvider.DASHSCOPE) content.addView(ServiceSettingsUi.label(this,getString(R.string.translation_speech_title),18f), LinearLayout.LayoutParams(-1,-2).apply { bottomMargin=dp(16) })
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
                    pair.first.hint=getString(R.string.translation_saved_field,title)
                    pair.first.placeholderText=getString(R.string.service_secret_saved)
                }
            } else pair.second.setText(saved.value(field.id))
            inputs[field.id]=pair
        }
        val advanced=LinearLayout(this).apply { orientation=LinearLayout.VERTICAL; visibility=View.GONE }
        if(provider.hasAdvanced || provider == ServiceProvider.DASHSCOPE) {
            val toggle=MaterialButton(this,null,com.google.android.material.R.attr.borderlessButtonStyle).apply {
                id=R.id.advanced_toggle; text=getString(R.string.settings_advanced); isAllCaps=false
                setTextColor(ContextCompat.getColor(context,R.color.mimi_text))
                setOnClickListener { advanced.visibility=if(advanced.visibility==View.VISIBLE) View.GONE else View.VISIBLE
                    setText(if(advanced.visibility==View.VISIBLE) R.string.settings_advanced_collapse else R.string.settings_advanced) }
            }
            content.addView(toggle,LinearLayout.LayoutParams(-1,dp(48)))
        }
        val endpointField=field(advanced,"baseUrl",getString(R.string.service_endpoint),false)
        endpointInput=endpointField.second
        val modelField=field(advanced,"model",getString(R.string.service_model),false)
        modelLayout=modelField.first; modelInput=modelField.second
        endpointInput.setText(saved.endpoint); endpointField.first.placeholderText=provider.endpoint
        modelInput.setText(saved.model); modelLayout.placeholderText=provider.model
        if(provider == ServiceProvider.DASHSCOPE) {
            val hotwordsField=field(advanced,"hotwords",getString(R.string.service_glossary),false)
            hotwordsLayout=hotwordsField.first
            hotwordsInput=hotwordsField.second.apply {
                setText(runCatching { SettingsStore.hotwordsText(this@ServiceSettingsActivity) }.getOrDefault("")); hotwordsLayout?.placeholderText="Mimi=mimi"
            }
        }
        if(provider.hasAdvanced) content.addView(advanced)
        if (provider == ServiceProvider.DASHSCOPE && !storageUnavailable) {
            translationSettings = TextTranslationSettings(this) { custom ->
                modelLayout.visibility = if (custom) View.GONE else View.VISIBLE
                hotwordsLayout?.let { it.visibility = if (custom) View.GONE else View.VISIBLE }
            }
            content.addView(translationSettings!!.view, LinearLayout.LayoutParams(-1,-2).apply { topMargin=dp(12) })
        }
        scroll.addView(content); root.addView(scroll,LinearLayout.LayoutParams(-1,0,1f))
        val footer=LinearLayout(this).apply { orientation=LinearLayout.VERTICAL; setPadding(dp(24),dp(8),dp(24),dp(12)) }
        footer.addView(MaterialButton(this).apply {
            id=R.id.save; text=getString(R.string.service_save_use); setOnClickListener { save() }
        },LinearLayout.LayoutParams(-1,dp(52)))
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
        }
        box.addView(edit,LinearLayout.LayoutParams(-1,-2))
        container.addView(box,LinearLayout.LayoutParams(-1,-2).apply { bottomMargin=dp(18) })
        return box to edit
    }
    override fun onDestroy() { translationSettings?.dispose(); super.onDestroy() }
    private fun openHelp(url: String) {
        try { startActivity(android.content.Intent(android.content.Intent.ACTION_VIEW, android.net.Uri.parse(url))) }
        catch (_: android.content.ActivityNotFoundException) { Toast.makeText(this, R.string.guide_no_browser, Toast.LENGTH_SHORT).show() }
    }
    private fun save() {
        status.visibility=View.VISIBLE
        if(storageUnavailable) return
        val textTranslation = translationSettings?.draft()
        if(translationSettings != null && textTranslation == null) { status.visibility=View.GONE; return }
        if(MimiService.isRunning) { Toast.makeText(this,R.string.service_stop_first,Toast.LENGTH_SHORT).show(); return }
        var valid=true
        val values=provider.fields.associate { field ->
            val (layout,edit)=inputs.getValue(field.id)
            val value=edit.text.toString().trim().ifBlank { if(field.secret) saved.value(field.id) else "" }
            layout.error=if(value.isBlank()) getString(R.string.service_required) else null
            if(value.isBlank()) valid=false
            field.id to value
        }
        if(!valid) return
        val config=ServiceConfiguration(provider,values,endpointInput.text.toString().trim(),modelInput.text.toString().trim())
        val (source, target) = runCatching {
            provider.normalize(SettingsStore.sourceLang(this), SettingsStore.targetLang(this), textTranslation?.provider ?: TextTranslationProvider.BUILTIN)
        }.getOrElse { status.text = getString(R.string.guide_storage_unavailable); return }
        // Validate URL/signing requirements locally without contacting a provider or logging secrets.
        try {
            if(provider.hasAdvanced && config.endpoint.isNotBlank()) endpoint(config)
            if(provider !in listOf(ServiceProvider.DASHSCOPE,ServiceProvider.OPENAI)) createProtocol(config,source,target).request()
        } catch (_:Exception) { status.text=getString(R.string.service_invalid); return }
        if(!runCatching { SettingsStore.saveConfiguration(this,config,textTranslation,translationSettings?.enabled) && SettingsStore.activateProvider(this,provider) }.getOrDefault(false)) {
            status.text=getString(R.string.guide_storage_unavailable); return
        }
        hotwordsInput?.let { SettingsStore.setHotwords(this,it.text.toString()) }
        Toast.makeText(this,R.string.settings_saved,Toast.LENGTH_SHORT).show(); finish()
    }
}
