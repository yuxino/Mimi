package app.yuxino.mimi.android.provider

import org.json.JSONObject

internal const val OPENAI_REALTIME_ENDPOINT = "wss://api.openai.com/v1/realtime/translations"
internal const val OPENAI_REALTIME_MODEL = "gpt-realtime-translate"

/** Pure metadata shared by setup, language selection and session creation. */
data class CredentialField(val id: String, val label: String, val secret: Boolean = true)

enum class ServiceProvider(
    val id: String, val title: String, val description: String, val sampleRate: Int,
    val fields: List<CredentialField>,
    val endpoint: String = "", val model: String = "",
) {
    DASHSCOPE("dashscope", "阿里云", "Audio 3.0 · Qwen-MT", 16000,
        listOf(CredentialField("apiKey", "API Key"))),
    OPENAI("openai", "OpenAI", "Realtime Translation", 24000,
        listOf(CredentialField("apiKey", "API Key")), OPENAI_REALTIME_ENDPOINT, OPENAI_REALTIME_MODEL),
    GEMINI("gemini", "Google Gemini", "Live Translation", 16000,
        listOf(CredentialField("apiKey", "API Key")),
        "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent", "gemini-3.5-live-translate-preview"),
    AZURE("azure", "Azure OpenAI", "使用自己的 Azure 部署", 24000,
        listOf(CredentialField("endpoint", "资源地址", false), CredentialField("deployment", "翻译部署名称", false),
            CredentialField("transcriptionDeployment", "转写部署名称", false), CredentialField("apiKey", "API Key"))),
    VOLCANO("volcano", "火山引擎", "豆包同声传译 2.0", 16000,
        listOf(CredentialField("apiKey", "API Key"))),
    TENCENT("tencent", "腾讯云", "实时语音翻译", 16000,
        listOf(CredentialField("appId", "AppID", false), CredentialField("secretId", "SecretID"), CredentialField("secretKey", "SecretKey"))),
    BAIDU("baidu", "百度翻译", "实时语音翻译", 16000,
        listOf(CredentialField("appId", "AppID", false), CredentialField("appKey", "AppKey"))),
    XAI("xai", "xAI Grok", "Grok Voice · 按语音轮次翻译", 24000,
        listOf(CredentialField("apiKey", "API Key")), "wss://api.x.ai/v1/realtime", "grok-voice-latest");

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

    private fun route(translation: TextTranslationProvider, alibabaRealtimeMode: String): String =
        if (this == DASHSCOPE && alibabaRealtimeMode != "liveTranslate") translation.runtimeRoute() else "followService"

    private fun runtimeTarget(target: String, translation: TextTranslationProvider, alibabaRealtimeMode: String): String =
        if (this == DASHSCOPE && alibabaRealtimeMode != "liveTranslate" && translation == TextTranslationProvider.NONE) "original" else target

    private fun capabilities(translation: TextTranslationProvider, alibabaRealtimeMode: String): JSONObject =
        NativeRuntimeConfiguration.capabilities(wireProvider, route(translation, alibabaRealtimeMode),
            runtimeTarget("zh", translation, alibabaRealtimeMode), alibabaRealtimeMode)

    fun sourcesForTranslation(translation: TextTranslationProvider, alibabaRealtimeMode: String = "asrMt"): List<String> =
        capabilities(translation, alibabaRealtimeMode).strings("sourceLanguages")

    fun targetsForTranslation(translation: TextTranslationProvider, alibabaRealtimeMode: String = "asrMt"): List<String> =
        capabilities(translation, alibabaRealtimeMode).strings("targetLanguages")

    fun supportsPair(source: String, target: String, translation: TextTranslationProvider = TextTranslationProvider.BUILTIN,
        alibabaRealtimeMode: String = "asrMt"): Boolean {
        val effectiveTarget = runtimeTarget(target, translation, alibabaRealtimeMode)
        val normalized = try { NativeRuntimeConfiguration.normalize(wireProvider, route(translation, alibabaRealtimeMode),
            source, effectiveTarget, alibabaRealtimeMode) }
            catch (_: IllegalArgumentException) { return false }
        return normalized.getString("sourceLanguage") == source && normalized.getString("targetLanguage") == effectiveTarget
    }

    fun normalize(source: String, target: String, translation: TextTranslationProvider = TextTranslationProvider.BUILTIN,
        alibabaRealtimeMode: String = "asrMt"): Pair<String, String> {
        val normalized = NativeRuntimeConfiguration.normalize(wireProvider, route(translation, alibabaRealtimeMode), source,
            runtimeTarget(target, translation, alibabaRealtimeMode), alibabaRealtimeMode)
        return normalized.getString("sourceLanguage") to
            if (this == DASHSCOPE && alibabaRealtimeMode != "liveTranslate" && translation == TextTranslationProvider.NONE) target
            else normalized.getString("targetLanguage")
    }
    fun configured(values: Map<String, String>): Boolean = fields.all { !values[it.id].isNullOrBlank() }
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
}

internal fun normalizeQwenMTModel(value: String?): String = value?.takeIf { it in listOf("lite", "flash", "plus") } ?: "lite"

/** Android-only cleartext permission is checked before shared typed credentials. */
internal fun TranslationConfiguration.runtimeCredentials(): JSONObject? {
    if (provider in listOf(TextTranslationProvider.BUILTIN, TextTranslationProvider.NONE)) return null
    validateTranslationConfiguration(this)
    val credentials = JSONObject().put("kind", provider.runtimeRoute())
    return when (provider) {
        TextTranslationProvider.DEEPL -> credentials.put("apiKey", apiKey.trim())
        TextTranslationProvider.DEEPLX -> credentials.put("endpoint", endpoint.trim()).put("token", apiKey.trim())
        TextTranslationProvider.CHAT_MOCK, TextTranslationProvider.OPENAI_COMPATIBLE ->
            credentials.put("endpoint", endpoint.trim()).put("model", model.trim()).put("apiKey", apiKey.trim())
        else -> null
    }
}

/** Adapt saved platform fields only. Recognition, translation and validation run in shared Rust. */
internal fun buildRuntimeConfiguration(
    speech: ServiceConfiguration,
    translation: TranslationConfiguration,
    sourceLanguage: String,
    targetLanguage: String,
    qwenMtModel: String,
): JSONObject {
    require(speech.provider.configured(speech.credentials)) { "speech_credentials" }
    val route = if (speech.usesIndependentTextTranslation) translation.provider else TextTranslationProvider.BUILTIN
    fun credential(kind: String) = JSONObject().put("kind", kind)
    fun value(field: String) = speech.value(field).trim()
    val text = if (speech.usesIndependentTextTranslation) translation.runtimeCredentials() else null
    val credentials = when (speech.provider) {
        ServiceProvider.DASHSCOPE -> when (route) {
            TextTranslationProvider.BUILTIN, TextTranslationProvider.NONE -> credential("apiKey").put("apiKey", value("apiKey"))
            else -> JSONObject(text!!.toString()).put("asrApiKey", value("apiKey"))
        }
        ServiceProvider.AZURE -> credential("azureOpenAI").put("endpoint", value("endpoint")).put("deployment", value("deployment"))
            .put("transcriptionDeployment", value("transcriptionDeployment")).put("apiKey", value("apiKey"))
        ServiceProvider.TENCENT -> credential("tencentCloud").put("appId", value("appId")).put("secretId", value("secretId")).put("secretKey", value("secretKey"))
        ServiceProvider.BAIDU -> credential("baiduTranslate").put("appId", value("appId")).put("appKey", value("appKey"))
        else -> credential("apiKey").put("apiKey", value("apiKey"))
    }
    return JSONObject()
        .put("provider", if (speech.provider == ServiceProvider.DASHSCOPE && route == TextTranslationProvider.DEEPLX) "deepLX" else speech.provider.wireProvider)
        .put("credentials", credentials)
        .put("textCredentials", text ?: JSONObject.NULL)
        .put("sourceLanguage", sourceLanguage)
        .put("targetLanguage", if (route == TextTranslationProvider.NONE) "original" else targetLanguage)
        .put("qwenMtModel", normalizeQwenMTModel(qwenMtModel))
        .put("alibabaRealtimeMode", if (speech.usesAlibabaLiveTranslate) "liveTranslate" else "disabled")
        .put("alibabaWorkspaceId", if (speech.provider == ServiceProvider.DASHSCOPE) speech.alibabaWorkspaceId.trim() else "")
        .put("translationMode", "turbo")
        .put("networkProxy", JSONObject().put("mode", "system").put("url", JSONObject.NULL))
        .put("textNetworkProxy", JSONObject().put("mode", "system").put("url", JSONObject.NULL))
}

/** Never use a data class: generated toString must not expose credentials. */
class ServiceConfiguration(val provider: ServiceProvider, val credentials: Map<String, String>,
    val endpoint: String = "", val model: String = "",
    val alibabaRealtimeMode: String = "asrMt", val alibabaWorkspaceId: String = "") {
    val usesAlibabaLiveTranslate: Boolean get() = provider == ServiceProvider.DASHSCOPE && alibabaRealtimeMode == "liveTranslate"
    val usesIndependentTextTranslation: Boolean get() = provider == ServiceProvider.DASHSCOPE && !usesAlibabaLiveTranslate
    fun value(field: String): String = credentials[field].orEmpty()
    override fun toString(): String = "ServiceConfiguration(${provider.id}, redacted)"
}
