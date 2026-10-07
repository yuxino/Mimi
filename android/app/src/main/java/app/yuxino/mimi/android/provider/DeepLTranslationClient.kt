package app.yuxino.mimi.android.provider

import okhttp3.OkHttpClient
import org.json.JSONArray
import org.json.JSONObject

/** Official DeepL API: the key selects the Free or Pro origin; user endpoints are never used. */
class DeepLTranslationClient(
    private val configuration: TranslationConfiguration,
    client: OkHttpClient = defaultTranslationHttpClient(),
) : TranslationClient {
    override fun resolveSourceLanguage(configured: String, reported: String?): String =
        translationSourceLanguage(configured, reported, DEEPL_SOURCE_CODES)

    private val transport = BoundedTranslationHttpClient(client)

    override fun translate(text: String, sourceLanguage: String, targetLanguage: String, callback: (TranslationResult) -> Unit): TranslationCall =
        transport.execute({
            require(configuration.provider == TextTranslationProvider.DEEPL) { "translation_provider" }
            val endpoint = validateTranslationConfiguration(configuration)
            translationJsonRequest(endpoint, deepLRequest(text, sourceLanguage, targetLanguage))
                .header("Authorization", "DeepL-Auth-Key ${configuration.apiKey.trim()}").build()
        }, ::decodeDeepLResponse, callback)
}

internal fun deepLEndpoint(apiKey: String): String {
    val key = apiKey.trim()
    require(key.isNotEmpty() && key.length <= 4096 && key.all { it.code in 33..126 }) { "translation_key" }
    return if (key.endsWith(":fx")) "https://api-free.deepl.com/v2/translate" else "https://api.deepl.com/v2/translate"
}

internal fun deepLRequest(text: String, source: String, target: String): JSONObject {
    val body = JSONObject().put("text", JSONArray().put(boundedTranslationInput(text)))
        .put("target_lang", deepLLanguage(target, source = false))
    if (source != "auto") body.put("source_lang", deepLLanguage(source, source = true))
    return body
}

internal fun decodeDeepLResponse(body: String): String {
    val translations = JSONObject(body).getJSONArray("translations")
    require(translations.length() == 1) { "translation_response" }
    return boundedTranslationText(translations.getJSONObject(0).get("text"))
}
