package app.yuxino.mimi.android.provider

/** Pure metadata shared by setup, language selection and session creation. */
data class CredentialField(val id: String, val label: String, val secret: Boolean = true)

enum class ServiceProvider(
    val id: String, val title: String, val description: String, val sampleRate: Int,
    val sources: List<String>, val fields: List<CredentialField>,
    val endpoint: String = "", val model: String = "",
) {
    DASHSCOPE("dashscope", "阿里云", "通义实时翻译", 16000, listOf("auto") + DASHSCOPE_LIVE_LANGUAGE_CODES,
        listOf(CredentialField("apiKey", "API Key")), DashScopeEngine.DASHSCOPE_REALTIME_WS, DashScopeEngine.MODEL),
    OPENAI("openai", "OpenAI", "Realtime Translation", 24000, listOf("auto"),
        listOf(CredentialField("apiKey", "API Key")), OpenAIRealtimeEngine.ENDPOINT, OpenAIRealtimeEngine.MODEL),
    GEMINI("gemini", "Google Gemini", "Live Translation", 16000, listOf("auto"),
        listOf(CredentialField("apiKey", "API Key")),
        "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent", "gemini-3.5-live-translate-preview"),
    AZURE("azure", "Azure OpenAI", "使用自己的 Azure 部署", 24000, listOf("auto"),
        listOf(CredentialField("endpoint", "资源地址", false), CredentialField("deployment", "翻译部署名称", false),
            CredentialField("transcriptionDeployment", "转写部署名称", false), CredentialField("apiKey", "API Key"))),
    VOLCANO("volcano", "火山引擎", "豆包同声传译 2.0", 16000, VOLCANO_SOURCE_CODES,
        listOf(CredentialField("apiKey", "API Key"))),
    TENCENT("tencent", "腾讯云", "实时语音翻译", 16000, TENCENT_LANGUAGE_PAIRS.keys.toList(),
        listOf(CredentialField("appId", "AppID", false), CredentialField("secretId", "SecretID"), CredentialField("secretKey", "SecretKey"))),
    BAIDU("baidu", "百度翻译", "实时语音翻译", 16000, BAIDU_LANGUAGE_CODES.keys.toList(),
        listOf(CredentialField("appId", "AppID", false), CredentialField("appKey", "AppKey"))),
    XAI("xai", "xAI Grok", "Grok Voice · 按语音轮次翻译", 24000, listOf("auto") + XAI_LANGUAGE_NAMES.keys,
        listOf(CredentialField("apiKey", "API Key")), "wss://api.x.ai/v1/realtime", "grok-voice-latest");

    val targets: List<String> get() = when (this) {
        DASHSCOPE -> DASHSCOPE_LIVE_LANGUAGE_CODES
        OPENAI, AZURE -> listOf("zh", "en", "ja", "ko", "ru", "es", "fr", "pt", "de", "it", "vi", "id", "hi")
        GEMINI -> GEMINI_TRANSLATION_LANGUAGE_CODES
        TENCENT -> TENCENT_LANGUAGE_PAIRS.keys.toList()
        BAIDU -> BAIDU_LANGUAGE_CODES.keys.toList()
        XAI -> XAI_LANGUAGE_NAMES.keys.toList()
        VOLCANO -> VOLCANO_TARGET_CODES
    }
    val hasAdvanced: Boolean get() = endpoint.isNotEmpty()

    fun sourcesForTranslation(translation: TextTranslationProvider): List<String> {
        if (this != DASHSCOPE || translation == TextTranslationProvider.BUILTIN) return sources
        val recognition = listOf("auto") + DASHSCOPE_ASR_LANGUAGE_CODES
        val translationSources = when (translation) {
            TextTranslationProvider.DEEPL -> DEEPL_SOURCE_CODES
            TextTranslationProvider.DEEPLX -> DEEPLX_SOURCE_CODES
            else -> return recognition
        }
        return recognition.filter { it == "auto" || it in translationSources }
    }

    // Independent text routes use the ASR-only model and their own text catalogs.
    fun targetsForTranslation(translation: TextTranslationProvider): List<String> =
        if (this != DASHSCOPE) targets else when (translation) {
            TextTranslationProvider.CHAT_MOCK, TextTranslationProvider.OPENAI_COMPATIBLE -> OPENAI_COMPATIBLE_TARGET_LANGUAGE_NAMES.keys.toList()
            TextTranslationProvider.DEEPL -> DEEPL_TARGET_CODES
            TextTranslationProvider.DEEPLX -> DEEPLX_TARGET_CODES
            else -> targets
        }

    fun supportsPair(source: String, target: String, translation: TextTranslationProvider = TextTranslationProvider.BUILTIN): Boolean {
        if (source !in sourcesForTranslation(translation)) return false
        if (this == DASHSCOPE && translation == TextTranslationProvider.NONE) return true
        if (target !in targetsForTranslation(translation)) return false
        if (this == TENCENT) return target in TENCENT_LANGUAGE_PAIRS[source].orEmpty()
        if (this == VOLCANO) return target in VOLCANO_LANGUAGE_PAIRS[source].orEmpty()
        return source != target || (this == DASHSCOPE && translation != TextTranslationProvider.BUILTIN)
    }

    fun normalize(source: String, target: String, translation: TextTranslationProvider = TextTranslationProvider.BUILTIN): Pair<String, String> {
        val sources = sourcesForTranslation(translation)
        val targets = targetsForTranslation(translation)
        val normalizedSource = source.takeIf { it in sources }
            ?: sources.firstOrNull { supportsPair(it, target, translation) } ?: sources.first()
        val normalizedTarget = target.takeIf { supportsPair(normalizedSource, it, translation) }
            ?: targets.first { supportsPair(normalizedSource, it, translation) }
        return normalizedSource to normalizedTarget
    }
    fun configured(values: Map<String, String>): Boolean = fields.all { !values[it.id].isNullOrBlank() }
    companion object {
        fun fromId(id: String): ServiceProvider = entries.firstOrNull { it.id == id } ?: DASHSCOPE
    }
}

/** Never use a data class: generated toString must not expose credentials. */
class ServiceConfiguration(val provider: ServiceProvider, val credentials: Map<String, String>,
    val endpoint: String = "", val model: String = "") {
    fun value(field: String): String = credentials[field].orEmpty()
    override fun toString(): String = "ServiceConfiguration(${provider.id}, redacted)"
}
