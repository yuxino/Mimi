package app.yuxino.mimi.android.provider

import org.junit.Assert.*
import org.junit.Test

class RuntimeConfigurationTest {
    private val alibaba = ServiceConfiguration(ServiceProvider.DASHSCOPE, mapOf("apiKey" to "synthetic-speech-key"),
        "wss://legacy.example.invalid/realtime", "qwen3.5-livetranslate-flash-realtime")

    @Test fun alibabaUsesIndependentTextModelWithoutBorrowingTheOldRealtimeOverride() {
        val config = buildRuntimeConfiguration(alibaba, TranslationConfiguration(provider = TextTranslationProvider.BUILTIN), "ja", "zh", "flash")
        assertEquals("alibabaCloud", config.getString("provider"))
        assertEquals("apiKey", config.getJSONObject("credentials").getString("kind"))
        assertEquals("flash", config.getString("qwenMtModel"))
        assertEquals("turbo", config.getString("translationMode"))
        assertTrue(config.isNull("textCredentials"))
        assertFalse(config.toString().contains("legacy.example.invalid"))
        assertFalse(config.toString().contains("qwen3.5-livetranslate"))
        assertEquals("qwen3.5-livetranslate-flash-realtime", alibaba.model)
    }

    @Test fun missingEmptyOrLegacyModelValuesUseThePcLiteDefault() {
        for (value in listOf(null, "", "unknown", "qwen3.5-livetranslate-flash-realtime")) assertEquals("lite", normalizeQwenMTModel(value))
        for (value in listOf("lite", "flash", "plus")) assertEquals(value, normalizeQwenMTModel(value))
    }

    @Test fun eachTextDestinationKeepsItsOwnSecretAndSpeechIdentity() {
        val routes = listOf(
            TranslationConfiguration(provider = TextTranslationProvider.DEEPL, apiKey = "synthetic-text-key:fx"),
            TranslationConfiguration(provider = TextTranslationProvider.DEEPLX, endpoint = "https://example.invalid/translate", apiKey = "synthetic-text-token"),
            TranslationConfiguration(provider = TextTranslationProvider.OPENAI_COMPATIBLE, endpoint = "https://example.invalid/v1", model = "synthetic-model", apiKey = "synthetic-text-key"),
            TranslationConfiguration(provider = TextTranslationProvider.CHAT_MOCK, endpoint = "http://127.0.0.1:8000/v1", model = "synthetic-model", allowLocalHttp = true),
        )
        for (route in routes) {
            val config = buildRuntimeConfiguration(alibaba, route, "auto", "fr", "plus")
            assertEquals(if (route.provider == TextTranslationProvider.DEEPLX) "deepLX" else "alibabaCloud", config.getString("provider"))
            val speech = config.getJSONObject("credentials")
            val text = config.getJSONObject("textCredentials")
            assertEquals("synthetic-speech-key", speech.getString("asrApiKey"))
            assertFalse(text.has("asrApiKey"))
            assertEquals(route.provider.runtimeRoute(), text.getString("kind"))
            assertEquals(route.apiKey, text.getString(if (route.provider == TextTranslationProvider.DEEPLX) "token" else "apiKey"))
        }
    }

    @Test fun originalModeChangesOnlyTheNativeTargetAndSendsNoTextCredentials() {
        val config = buildRuntimeConfiguration(alibaba, TranslationConfiguration(provider = TextTranslationProvider.NONE), "nl", "zh", "lite")
        assertEquals("nl", config.getString("sourceLanguage"))
        assertEquals("original", config.getString("targetLanguage"))
        assertTrue(config.isNull("textCredentials"))
        assertEquals("apiKey", config.getJSONObject("credentials").getString("kind"))
    }

    @Test fun allExistingServicesMapTheirSavedFieldsToPcCredentialTags() {
        val expected = mapOf(ServiceProvider.AZURE to "azureOpenAI", ServiceProvider.TENCENT to "tencentCloud", ServiceProvider.BAIDU to "baiduTranslate")
        for (provider in ServiceProvider.entries) {
            val values = provider.fields.associate { it.id to "synthetic-${it.id}" }
            val config = buildRuntimeConfiguration(ServiceConfiguration(provider, values), TranslationConfiguration(provider = TextTranslationProvider.BUILTIN), "auto", "zh", "lite")
            assertEquals(provider.wireProvider, config.getString("provider"))
            val credentials = config.getJSONObject("credentials")
            assertEquals(expected[provider] ?: "apiKey", credentials.getString("kind"))
            for ((field, value) in values) assertEquals(value, credentials.getString(field))
            assertEquals("system", config.getJSONObject("networkProxy").getString("mode"))
        }
    }

    @Test fun localHttpMustBeExplicitAndLimitedToTheAndroidAllowlist() {
        val base = TranslationConfiguration(provider = TextTranslationProvider.CHAT_MOCK, model = "synthetic-model", endpoint = "http://10.0.2.2:8000/v1")
        assertThrows(IllegalArgumentException::class.java) { buildRuntimeConfiguration(alibaba, base, "auto", "zh", "lite") }
        val allowed = base.copy(allowLocalHttp = true)
        assertEquals(allowed.endpoint, buildRuntimeConfiguration(alibaba, allowed, "auto", "zh", "lite").getJSONObject("textCredentials").getString("endpoint"))
        assertThrows(IllegalArgumentException::class.java) {
            buildRuntimeConfiguration(alibaba, allowed.copy(endpoint = "http://public.example.invalid/v1"), "auto", "zh", "lite")
        }
    }
}
