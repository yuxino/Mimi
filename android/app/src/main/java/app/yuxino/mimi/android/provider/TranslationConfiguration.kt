package app.yuxino.mimi.android.provider

import okhttp3.HttpUrl
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull

/** Stable IDs keep text translation settings independent from the speech provider. */
enum class TextTranslationProvider(val storageId: String) {
    BUILTIN("builtin"),
    NONE("none"),
    CHAT_MOCK("chatMock"),
    OPENAI_COMPATIBLE("openaiCompatible"),
    DEEPL("deepL"),
    DEEPLX("deepLX");

    val usesOpenAIProtocol: Boolean get() = this == CHAT_MOCK || this == OPENAI_COMPATIBLE

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
    TextTranslationProvider.DEEPL -> deepLEndpoint(config.apiKey)
    TextTranslationProvider.DEEPLX -> {
        val url = validatedTranslationUrl(config)
        val path = url.encodedPath.trimEnd('/')
        url.newBuilder().encodedPath(if (path.endsWith("/translate")) path else "$path/translate").build().toString()
    }
    TextTranslationProvider.CHAT_MOCK, TextTranslationProvider.OPENAI_COMPATIBLE -> {
        val url = validatedTranslationUrl(config)
        val path = url.encodedPath.trimEnd('/')
        val completedPath = if (path.endsWith("/chat/completions")) path else "$path/chat/completions"
        url.newBuilder().encodedPath(completedPath).build().toString()
    }
}

private fun validatedTranslationUrl(config: TranslationConfiguration): HttpUrl {
    require(config.endpoint.toByteArray(Charsets.UTF_8).size in 1..2048 && config.endpoint.none { it.isISOControl() }) { "translation_endpoint" }
    val endpoint = config.endpoint.trim()
    require(endpoint.startsWith("https://") || endpoint.startsWith("http://")) { "translation_endpoint" }
    require(!endpoint.contains('\\') && !endpoint.substringAfter("://").substringBefore('/').contains('@')) { "translation_endpoint" }
    val url = endpoint.toHttpUrlOrNull() ?: throw IllegalArgumentException("translation_endpoint")
    require(url.username.isEmpty() && url.password.isEmpty() && url.query == null && url.fragment == null) { "translation_endpoint" }
    require(url.isHttps || (config.allowLocalHttp && isLocalTranslationHost(url.host))) { "translation_https_required" }
    return url
}

// This explicit allowlist also matches Android's network_security_config.xml.
private fun isLocalTranslationHost(host: String): Boolean =
    host in setOf("localhost", "127.0.0.1", "::1", "10.0.2.2")

internal fun validateTranslationConfiguration(config: TranslationConfiguration): String {
    if (config.provider == TextTranslationProvider.BUILTIN || config.provider == TextTranslationProvider.NONE) return ""
    val endpoint = normalizeTranslationEndpoint(config)
    if (config.provider.usesOpenAIProtocol) {
        require(config.model.trim().toByteArray(Charsets.UTF_8).size in 1..256 && config.model.none { it.isISOControl() }) { "translation_model" }
    }
    val key = config.apiKey.trim()
    // Match desktop saved credentials: Unicode scalar count, optional blank keys and trimmed values.
    val checkedKey = if (config.provider.usesOpenAIProtocol) config.apiKey else key
    require(key.codePointCount(0, key.length) <= 1024 && checkedKey.none { it.isISOControl() }) { "translation_key" }
    return endpoint
}

