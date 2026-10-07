package app.yuxino.mimi.android.provider

import okhttp3.OkHttpClient
import org.json.JSONObject

/** DeepLX's JSON protocol is independent from the official DeepL API. */
class DeepLXTranslationClient(
    private val configuration: TranslationConfiguration,
    client: OkHttpClient = defaultTranslationHttpClient(),
) : TranslationClient {
    override fun resolveSourceLanguage(configured: String, reported: String?): String =
        translationSourceLanguage(configured, reported, DEEPLX_SOURCE_CODES)

    private val transport = BoundedTranslationHttpClient(client)

    override fun translate(text: String, sourceLanguage: String, targetLanguage: String, callback: (TranslationResult) -> Unit): TranslationCall =
        transport.execute({
            require(configuration.provider == TextTranslationProvider.DEEPLX) { "translation_provider" }
            val endpoint = validateTranslationConfiguration(configuration)
            translationJsonRequest(endpoint, deepLXRequest(text, sourceLanguage, targetLanguage))
                .apply { if (configuration.apiKey.isNotBlank()) header("Authorization", "Bearer ${configuration.apiKey.trim()}") }
                .build()
        }, ::decodeDeepLXResponse, callback)
}

internal fun deepLXRequest(text: String, source: String, target: String): JSONObject =
    JSONObject().put("text", boundedTranslationInput(text))
        .put("source_lang", if (source == "auto") "auto" else deepLLanguage(source, source = true, compatible = true))
        .put("target_lang", deepLLanguage(target, source = false, compatible = true))

internal fun decodeDeepLXResponse(body: String): String {
    val response = JSONObject(body)
    val code = response.get("code")
    require(code is Int || code is Long) { "translation_response" }
    val number = (code as Number).toLong()
    require(number in 0..65535) { "translation_response" }
    require(number == 200L) { "translation_rejected_$number" }
    return boundedTranslationText(response.get("data"))
}
