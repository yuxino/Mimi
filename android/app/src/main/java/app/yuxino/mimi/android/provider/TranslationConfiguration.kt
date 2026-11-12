package app.yuxino.mimi.android.provider

/** Stable IDs keep text translation settings independent from the speech provider. */
enum class TextTranslationProvider(val storageId: String) {
    BUILTIN("builtin"),
    NONE("none"),
    CHAT_MOCK("chatMock"),
    OPENAI_COMPATIBLE("openaiCompatible"),
    XAI("xAI"),
    DEEPL("deepL"),
    DEEPLX("deepLX");

    val usesOpenAIProtocol: Boolean get() = this == CHAT_MOCK || this == OPENAI_COMPATIBLE || this == XAI
    val hasCustomEndpoint: Boolean get() = this == CHAT_MOCK || this == OPENAI_COMPATIBLE || this == DEEPLX

    companion object {
        fun fromStorageId(id: String): TextTranslationProvider = entries.firstOrNull { it.storageId == id } ?: BUILTIN
    }
}

/** Credentials belong only to this text-translation service, never to the speech provider. */
data class TranslationConfiguration(
    val endpoint: String = "",
    val model: String = "",
    val apiKey: String = "",
    val allowLocalHttp: Boolean = false,
    val provider: TextTranslationProvider = TextTranslationProvider.OPENAI_COMPATIBLE,
) {
    override fun toString(): String = "TranslationConfiguration(provider=${provider.storageId}, credentials=redacted)"
}

/** Resolve only the endpoint shapes supported by the selected text protocol. */
fun normalizeTranslationEndpoint(config: TranslationConfiguration): String = when (config.provider) {
    TextTranslationProvider.BUILTIN, TextTranslationProvider.NONE -> ""
    // The shared runtime supplies xAI's fixed endpoint; native settings store no override.
    TextTranslationProvider.XAI -> ""
    TextTranslationProvider.DEEPL -> {
        val key = config.apiKey.trim()
        require(key.length <= 4096) { "translation_key" }
        NativeRuntimeConfiguration.textEndpoint(config.provider, key, false)
    }
    TextTranslationProvider.DEEPLX, TextTranslationProvider.CHAT_MOCK, TextTranslationProvider.OPENAI_COMPATIBLE -> {
        // Bound the JSON crossing JNI before allocating it. Shared Rust owns
        // UTF-8 limits, URL parsing, path completion and Android transport policy.
        require(config.endpoint.length in 1..2048) { "translation_endpoint" }
        NativeRuntimeConfiguration.textEndpoint(config.provider, config.endpoint, config.allowLocalHttp)
    }
}

internal fun validateTranslationConfiguration(config: TranslationConfiguration): String {
    if (config.provider == TextTranslationProvider.BUILTIN || config.provider == TextTranslationProvider.NONE) return ""
    val endpoint = normalizeTranslationEndpoint(config)
    if (config.provider.usesOpenAIProtocol) {
        require(config.model.trim().toByteArray(Charsets.UTF_8).size in 1..256 && config.model.none { it.isISOControl() }) { "translation_model" }
    }
    val key = config.apiKey.trim()
    require(config.provider != TextTranslationProvider.XAI || key.isNotEmpty()) { "translation_key" }
    // Match desktop saved credentials: Unicode scalar count, optional blank keys and trimmed values.
    val checkedKey = if (config.provider.usesOpenAIProtocol) config.apiKey else key
    require(key.codePointCount(0, key.length) <= 1024 && checkedKey.none { it.isISOControl() }) { "translation_key" }
    return endpoint
}

