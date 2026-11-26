package app.yuxino.mimi.android.provider

import org.json.JSONObject

/** Pure metadata shared by setup, language selection and session creation. */
data class CredentialField(val id: String, val label: String, val secret: Boolean = true)

enum class ServiceProvider(
    val id: String, val title: String,
    val fields: List<CredentialField>,
) {
    DASHSCOPE("dashscope", "阿里云", listOf(CredentialField("apiKey", "API Key"))),
    OPENAI("openai", "OpenAI", listOf(CredentialField("apiKey", "API Key"))),
    GEMINI("gemini", "Google Gemini", listOf(CredentialField("apiKey", "API Key"))),
    AZURE("azure", "Azure OpenAI",
        listOf(CredentialField("endpoint", "资源地址", false), CredentialField("deployment", "翻译部署名称", false),
            CredentialField("transcriptionDeployment", "转写部署名称", false), CredentialField("apiKey", "API Key"))),
    VOLCANO("volcano", "火山引擎", listOf(CredentialField("apiKey", "API Key"))),
    TENCENT("tencent", "腾讯云",
        listOf(CredentialField("appId", "AppID", false), CredentialField("secretId", "SecretID"), CredentialField("secretKey", "SecretKey"))),
    BAIDU("baidu", "百度翻译",
        listOf(CredentialField("appId", "AppID", false), CredentialField("appKey", "AppKey"))),
    XAI("xai", "xAI Grok", listOf(CredentialField("apiKey", "API Key")));

    val wireProvider: String get() = when (this) {
        DASHSCOPE -> "alibabaCloud"
        OPENAI -> "openAIRealtime"
        GEMINI -> "googleGeminiLive"
        AZURE -> "azureOpenAIRealtime"
        VOLCANO -> "volcanoEngine"
        TENCENT -> "tencentCloud"
        BAIDU -> "baiduTranslate"
        XAI -> "xAIRealtime"
    }
    val sources: List<String> get() = sourcesForTranslation(TextTranslationProvider.BUILTIN)
    val targets: List<String> get() = targetsForTranslation(TextTranslationProvider.BUILTIN)

    val supportsSpeechTranslationMode: Boolean get() = this in listOf(OPENAI, GEMINI, AZURE)

    private fun usesIndependentTranslation(alibabaRealtimeMode: String, xaiSpeechMode: String, speechTranslationMode: String): Boolean =
        this == DASHSCOPE && alibabaRealtimeMode != "liveTranslate" || this == XAI && xaiSpeechMode == "asrMt" ||
            supportsSpeechTranslationMode && speechTranslationMode == "asrMt"

    private fun route(translation: TextTranslationProvider, alibabaRealtimeMode: String, xaiSpeechMode: String, speechTranslationMode: String): String =
        if (usesIndependentTranslation(alibabaRealtimeMode, xaiSpeechMode, speechTranslationMode)) translation.runtimeRoute() else "followService"

    private fun originalOnly(translation: TextTranslationProvider, alibabaRealtimeMode: String, xaiSpeechMode: String, speechTranslationMode: String): Boolean =
        usesIndependentTranslation(alibabaRealtimeMode, xaiSpeechMode, speechTranslationMode) && (translation == TextTranslationProvider.NONE ||
            this == XAI && translation == TextTranslationProvider.BUILTIN)

    private fun runtimeTarget(target: String, translation: TextTranslationProvider, alibabaRealtimeMode: String, xaiSpeechMode: String, speechTranslationMode: String): String =
        if (originalOnly(translation, alibabaRealtimeMode, xaiSpeechMode, speechTranslationMode)) "original" else target

    private fun capabilities(translation: TextTranslationProvider, alibabaRealtimeMode: String, xaiSpeechMode: String, speechTranslationMode: String): JSONObject =
        NativeRuntimeConfiguration.capabilities(wireProvider, route(translation, alibabaRealtimeMode, xaiSpeechMode, speechTranslationMode),
            runtimeTarget("zh", translation, alibabaRealtimeMode, xaiSpeechMode, speechTranslationMode), alibabaRealtimeMode, xaiSpeechMode, speechTranslationMode)

    fun sourcesForTranslation(translation: TextTranslationProvider, alibabaRealtimeMode: String = "asrMt", xaiSpeechMode: String = "voice", speechTranslationMode: String = "simultaneous"): List<String> =
        capabilities(translation, alibabaRealtimeMode, xaiSpeechMode, speechTranslationMode).strings("sourceLanguages")

    fun targetsForTranslation(translation: TextTranslationProvider, alibabaRealtimeMode: String = "asrMt", xaiSpeechMode: String = "voice", speechTranslationMode: String = "simultaneous"): List<String> =
        capabilities(translation, alibabaRealtimeMode, xaiSpeechMode, speechTranslationMode).strings("targetLanguages")

    fun supportsPair(source: String, target: String, translation: TextTranslationProvider = TextTranslationProvider.BUILTIN,
        alibabaRealtimeMode: String = "asrMt", xaiSpeechMode: String = "voice", speechTranslationMode: String = "simultaneous"): Boolean {
        val effectiveTarget = runtimeTarget(target, translation, alibabaRealtimeMode, xaiSpeechMode, speechTranslationMode)
        val normalized = try { NativeRuntimeConfiguration.normalize(wireProvider, route(translation, alibabaRealtimeMode, xaiSpeechMode, speechTranslationMode),
            source, effectiveTarget, alibabaRealtimeMode, xaiSpeechMode, speechTranslationMode) }
            catch (_: IllegalArgumentException) { return false }
        return normalized.getString("sourceLanguage") == source && normalized.getString("targetLanguage") == effectiveTarget
    }

    fun normalize(source: String, target: String, translation: TextTranslationProvider = TextTranslationProvider.BUILTIN,
        alibabaRealtimeMode: String = "asrMt", xaiSpeechMode: String = "voice", speechTranslationMode: String = "simultaneous"): Pair<String, String> {
        val normalized = NativeRuntimeConfiguration.normalize(wireProvider, route(translation, alibabaRealtimeMode, xaiSpeechMode, speechTranslationMode), source,
            runtimeTarget(target, translation, alibabaRealtimeMode, xaiSpeechMode, speechTranslationMode), alibabaRealtimeMode, xaiSpeechMode, speechTranslationMode)
        return normalized.getString("sourceLanguage") to
            if (originalOnly(translation, alibabaRealtimeMode, xaiSpeechMode, speechTranslationMode)) target
            else normalized.getString("targetLanguage")
    }
    fun configured(values: Map<String, String>, speechTranslationMode: String = "simultaneous"): Boolean =
        fields.filterNot { this == AZURE && speechTranslationMode == "asrMt" && it.id == "deployment" }
            .all { !values[it.id].isNullOrBlank() }
    companion object {
        fun fromId(id: String): ServiceProvider = entries.firstOrNull { it.id == id } ?: DASHSCOPE
    }
}

private fun JSONObject.strings(key: String): List<String> = getJSONArray(key).let { values ->
    (0 until values.length()).map(values::getString)
}

internal fun TextTranslationProvider.runtimeRoute(): String = when (this) {
    TextTranslationProvider.BUILTIN, TextTranslationProvider.NONE -> "followService"
    TextTranslationProvider.DEEPL -> "deepL"
    TextTranslationProvider.DEEPLX -> "deepLX"
    TextTranslationProvider.OPENAI_COMPATIBLE -> "openAICompatible"
    TextTranslationProvider.CHAT_MOCK -> "chatMock"
    TextTranslationProvider.XAI -> "xAI"
}

internal fun normalizeQwenMTModel(value: String?): String = value?.takeIf { it in listOf("lite", "flash", "plus") } ?: "lite"

/** Android-only cleartext permission is checked before shared typed credentials. */
internal fun TranslationConfiguration.runtimeCredentials(): JSONObject? {
    fun validated() = validateTranslationConfiguration(this).let { JSONObject().put("kind", provider.runtimeRoute()) }
    return when (provider) {
        TextTranslationProvider.BUILTIN, TextTranslationProvider.NONE -> null
        TextTranslationProvider.DEEPL -> validated().put("apiKey", apiKey.trim())
        TextTranslationProvider.DEEPLX -> validated().put("endpoint", endpoint.trim()).put("token", apiKey.trim())
        TextTranslationProvider.CHAT_MOCK, TextTranslationProvider.OPENAI_COMPATIBLE ->
            validated().put("endpoint", endpoint.trim()).put("model", model.trim()).put("apiKey", apiKey.trim())
        TextTranslationProvider.XAI -> validated().put("model", model.trim()).put("apiKey", apiKey.trim())
    }
}

/** Serialize native fields without resolving blanks; shared Rust owns identity-bound retention. */
internal fun ServiceConfiguration.runtimeCredentials(): JSONObject {
    fun credential(kind: String) = JSONObject().put("kind", kind)
    fun value(field: String) = this.value(field).trim()
    return when (provider) {
        ServiceProvider.AZURE -> credential("azureOpenAI").put("endpoint", value("endpoint")).put("deployment", value("deployment"))
            .put("transcriptionDeployment", value("transcriptionDeployment")).put("apiKey", value("apiKey"))
        ServiceProvider.TENCENT -> credential("tencentCloud").put("appId", value("appId")).put("secretId", value("secretId")).put("secretKey", value("secretKey"))
        ServiceProvider.BAIDU -> credential("baiduTranslate").put("appId", value("appId")).put("appKey", value("appKey"))
        else -> credential("apiKey").put("apiKey", value("apiKey"))
    }
}

/** UI adapters must not turn retained secrets into explicit replacements before shared validation. */
internal fun resolveSpeechCredentialDraft(saved: ServiceConfiguration, values: Map<String, String>, speechTranslationMode: String): Map<String, String> {
    val draft = ServiceConfiguration(saved.provider, values, speechTranslationMode = speechTranslationMode)
    val credentials = NativeRuntimeConfiguration.resolveCredentialsDraft(saved.provider, speechTranslationMode,
        draft.runtimeCredentials(), saved.runtimeCredentials())
    return saved.provider.fields.associate { it.id to credentials.getString(it.id) }
}

/** Adapt saved platform fields only. Recognition, translation and validation run in shared Rust. */
internal fun buildRuntimeConfiguration(
    speech: ServiceConfiguration,
    translation: TranslationConfiguration,
    sourceLanguage: String,
    targetLanguage: String,
    qwenMtModel: String,
): JSONObject {
    require(speech.provider.configured(speech.credentials, speech.speechTranslationMode)) { "speech_credentials" }
    require(speech.speechTranslationMode in listOf("simultaneous", "asrMt") &&
        (speech.speechTranslationMode == "simultaneous" || speech.provider.supportsSpeechTranslationMode)) { "speech_mode" }
    val route = if (speech.usesIndependentTextTranslation) translation.provider else TextTranslationProvider.BUILTIN
    val target = if (speech.originalOnly(route)) "original" else targetLanguage
    val text = if (speech.usesIndependentTextTranslation && target != "original") translation.runtimeCredentials() else null
    val credentials = speech.runtimeCredentials()
    return JSONObject()
        .put("provider", speech.provider.wireProvider)
        .put("credentials", credentials)
        .put("textCredentials", text ?: JSONObject.NULL)
        .put("sourceLanguage", sourceLanguage)
        .put("targetLanguage", target)
        .put("qwenMtModel", normalizeQwenMTModel(qwenMtModel))
        .put("alibabaRealtimeMode", if (speech.usesAlibabaLiveTranslate) "liveTranslate" else "disabled")
        .put("alibabaWorkspaceId", if (speech.provider == ServiceProvider.DASHSCOPE) speech.alibabaWorkspaceId.trim() else "")
        .put("xaiSpeechMode", if (speech.provider == ServiceProvider.XAI) speech.xaiSpeechMode else "voice")
        .put("xaiTextModel", speech.xaiTextModel)
        .put("speechTranslationMode", speech.speechTranslationMode)
        .put("translationMode", "turbo")
        .put("networkProxy", JSONObject().put("mode", "system").put("url", JSONObject.NULL))
        .put("textNetworkProxy", JSONObject().put("mode", "system").put("url", JSONObject.NULL))
}

/** A recognition check owns no unsaved text-model draft. Rust validates the resulting configuration. */
internal fun buildSpeechProbeConfiguration(
    speechDraft: ServiceConfiguration,
    sourceLanguage: String,
): JSONObject = buildRuntimeConfiguration(speechDraft,
    TranslationConfiguration(provider = TextTranslationProvider.NONE), sourceLanguage, "original", "lite")

/** Never use a data class: generated toString must not expose credentials. */
class ServiceConfiguration(val provider: ServiceProvider, val credentials: Map<String, String>,
    val endpoint: String = "", val model: String = "",
    val alibabaRealtimeMode: String = "asrMt", val alibabaWorkspaceId: String = "",
    val xaiSpeechMode: String = "voice", val xaiTextModel: String = "grok-4.3",
    val speechTranslationMode: String = "simultaneous") {
    val usesAlibabaLiveTranslate: Boolean get() = provider == ServiceProvider.DASHSCOPE && alibabaRealtimeMode == "liveTranslate"
    val usesIndependentTextTranslation: Boolean get() = provider == ServiceProvider.DASHSCOPE && !usesAlibabaLiveTranslate ||
        provider == ServiceProvider.XAI && xaiSpeechMode == "asrMt" ||
        provider.supportsSpeechTranslationMode && speechTranslationMode == "asrMt"
    fun originalOnly(translation: TextTranslationProvider): Boolean = usesIndependentTextTranslation &&
        (translation == TextTranslationProvider.NONE || provider == ServiceProvider.XAI && translation == TextTranslationProvider.BUILTIN)
    fun value(field: String): String = credentials[field].orEmpty()
    override fun toString(): String = "ServiceConfiguration(${provider.id}, redacted)"
}
