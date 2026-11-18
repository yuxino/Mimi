package app.yuxino.mimi.android.provider.runtime

import app.yuxino.mimi.android.provider.ServiceConfiguration
import app.yuxino.mimi.android.provider.ServiceProvider
import app.yuxino.mimi.android.provider.TextTranslationProvider
import app.yuxino.mimi.android.provider.TranslationConfiguration
import app.yuxino.mimi.android.provider.buildRuntimeConfiguration
import app.yuxino.mimi.android.provider.buildSpeechProbeConfiguration
import app.yuxino.mimi.android.provider.buildTextProbeConfiguration
import app.yuxino.mimi.android.provider.normalizeQwenMTModel
import app.yuxino.mimi.android.provider.runtimeCredentials
import app.yuxino.mimi.android.provider.runtimeRoute
import app.yuxino.mimi.android.provider.validateTranslationConfiguration
import org.junit.Assert.*
import org.junit.Test

class RuntimeConfigurationTest {
    private val alibaba = ServiceConfiguration(ServiceProvider.DASHSCOPE, mapOf("apiKey" to "synthetic-speech-key"),
        "wss://legacy.example.invalid/realtime", "qwen3.5-livetranslate-flash-realtime")

    @Test fun realtimeModesDefaultToSimultaneousAndKeepTextKeysIndependent() {
        for (provider in listOf(ServiceProvider.OPENAI, ServiceProvider.GEMINI, ServiceProvider.AZURE)) {
            val values = provider.fields.associate { it.id to "synthetic-${it.id}" }
            val simultaneous = ServiceConfiguration(provider, values)
            val asr = ServiceConfiguration(provider, values, speechTranslationMode = "asrMt")
            val translator = TranslationConfiguration(provider = TextTranslationProvider.XAI,
                model = "grok-independent", apiKey = "synthetic-text-key")
            assertEquals("simultaneous", simultaneous.speechTranslationMode)
            assertFalse(simultaneous.usesIndependentTextTranslation)
            assertTrue(asr.usesIndependentTextTranslation)
            val integrated = buildRuntimeConfiguration(simultaneous, translator, "auto", "zh", "lite")
            assertEquals("simultaneous", integrated.getString("speechTranslationMode"))
            assertTrue(integrated.isNull("textCredentials"))
            val independent = buildRuntimeConfiguration(asr, translator, "en", "zh", "lite")
            assertEquals("asrMt", independent.getString("speechTranslationMode"))
            assertEquals("synthetic-apiKey", independent.getJSONObject("credentials").getString("apiKey"))
            assertEquals("synthetic-text-key", independent.getJSONObject("textCredentials").getString("apiKey"))
            val probe = buildSpeechProbeConfiguration(asr, "en")
            assertEquals("original", probe.getString("targetLanguage"))
            assertTrue(probe.isNull("textCredentials"))
            val original = buildRuntimeConfiguration(asr, TranslationConfiguration(provider = TextTranslationProvider.NONE), "en", "zh", "lite")
            assertEquals("original", original.getString("targetLanguage"))
            assertTrue(original.isNull("textCredentials"))
        }
    }

    @Test fun azureRecognitionNeedsNoTranslationDeploymentButRestoredSimultaneousDoes() {
        val values = mapOf("endpoint" to "https://synthetic.openai.azure.com", "transcriptionDeployment" to "synthetic-asr", "apiKey" to "synthetic-key")
        val asr = ServiceConfiguration(ServiceProvider.AZURE, values, speechTranslationMode = "asrMt")
        assertTrue(ServiceProvider.AZURE.configured(values, "asrMt"))
        assertFalse(ServiceProvider.AZURE.configured(values))
        assertEquals("", buildSpeechProbeConfiguration(asr, "en").getJSONObject("credentials").getString("deployment"))
        assertThrows(IllegalArgumentException::class.java) {
            buildRuntimeConfiguration(ServiceConfiguration(ServiceProvider.AZURE, values), TranslationConfiguration(), "en", "zh", "lite")
        }
        assertThrows(IllegalArgumentException::class.java) {
            buildSpeechProbeConfiguration(ServiceConfiguration(ServiceProvider.AZURE, values - "transcriptionDeployment", speechTranslationMode = "asrMt"), "en")
        }
    }

    @Test fun xaiSpeechAndGrokTextKeepIndependentKeysAndModelsWhileVoiceIgnoresTheTranslator() {
        val voice = ServiceConfiguration(ServiceProvider.XAI, mapOf("apiKey" to "synthetic-xai-key"))
        val streaming = ServiceConfiguration(ServiceProvider.XAI, voice.credentials, xaiSpeechMode = "asrMt")
        val translator = TranslationConfiguration(provider = TextTranslationProvider.XAI, model = "grok-custom-text", apiKey = "synthetic-text-key")
        for (speech in listOf(voice, streaming)) {
            val configuration = buildRuntimeConfiguration(speech, translator, "en", "zh", "lite")
            assertEquals(speech.xaiSpeechMode, configuration.getString("xaiSpeechMode"))
            assertEquals(speech.xaiTextModel, configuration.getString("xaiTextModel"))
            assertEquals("apiKey", configuration.getJSONObject("credentials").getString("kind"))
            assertEquals("synthetic-xai-key", configuration.getJSONObject("credentials").getString("apiKey"))
            if (speech === voice) assertTrue(configuration.isNull("textCredentials")) else {
                val text = configuration.getJSONObject("textCredentials")
                assertEquals(setOf("kind", "model", "apiKey"), text.keys().asSequence().toSet())
                assertEquals("xAI", text.getString("kind"))
                assertEquals("grok-custom-text", text.getString("model"))
                assertEquals("synthetic-text-key", text.getString("apiKey"))
                assertFalse(text.toString().contains("synthetic-xai-key"))
            }
            assertEquals("system", configuration.getJSONObject("networkProxy").getString("mode"))
            assertEquals("system", configuration.getJSONObject("textNetworkProxy").getString("mode"))
        }
        assertEquals("voice", voice.xaiSpeechMode)
    }

    @Test fun recognitionProbeOwnsOnlySpeechFieldsAndTextProbeOwnsOnlyTextFields() {
        val speech = ServiceConfiguration(ServiceProvider.XAI, mapOf("apiKey" to "synthetic-current-key"), xaiSpeechMode = "asrMt")
        val recognition = buildSpeechProbeConfiguration(speech, "en")
        assertEquals("original", recognition.getString("targetLanguage"))
        assertEquals("synthetic-current-key", recognition.getJSONObject("credentials").getString("apiKey"))
        assertEquals("en", recognition.getString("sourceLanguage"))
        assertTrue(recognition.isNull("textCredentials"))
        val text = buildTextProbeConfiguration(TranslationConfiguration(provider = TextTranslationProvider.XAI,
            model = "grok-custom-text", apiKey = "synthetic-text-key"), "en", "zh", "lite")
        assertEquals("synthetic-text-key", text.getJSONObject("credentials").getString("apiKey"))
        assertEquals("grok-custom-text", text.getJSONObject("credentials").getString("model"))
        assertFalse(text.toString().contains("synthetic-current-key"))
        assertFalse(text.has("sessionConfiguration"))
    }

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
            TranslationConfiguration(provider = TextTranslationProvider.XAI, model = "grok-custom-text", apiKey = "synthetic-text-key"),
        )
        val xai = ServiceConfiguration(ServiceProvider.XAI, mapOf("apiKey" to "synthetic-speech-key"), xaiSpeechMode = "asrMt")
        for (speechConfiguration in listOf(alibaba, xai)) for (route in routes) {
            val config = buildRuntimeConfiguration(speechConfiguration, route, "auto", "fr", "plus")
            assertEquals(speechConfiguration.provider.wireProvider, config.getString("provider"))
            val speech = config.getJSONObject("credentials")
            val text = config.getJSONObject("textCredentials")
            assertEquals(setOf("kind", "apiKey"), speech.keys().asSequence().toSet())
            assertEquals("synthetic-speech-key", speech.getString("apiKey"))
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
        val xai = ServiceConfiguration(ServiceProvider.XAI, mapOf("apiKey" to "synthetic-speech-key"), xaiSpeechMode = "asrMt")
        for (route in listOf(TextTranslationProvider.BUILTIN, TextTranslationProvider.NONE)) {
            val original = buildRuntimeConfiguration(xai, TranslationConfiguration(provider = route), "en", "zh", "lite")
            assertEquals("original", original.getString("targetLanguage"))
            assertTrue(original.isNull("textCredentials"))
        }
        for (speech in listOf(alibaba, xai)) {
            // A disabled stage does not validate an unrelated, unfinished translator draft.
            val original = buildRuntimeConfiguration(speech, TranslationConfiguration(provider = TextTranslationProvider.XAI,
                model = "bad\nmodel"), "en", "original", "lite")
            assertTrue(original.isNull("textCredentials"))
        }
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

    @Test fun grokUsesItsOwnRequiredKeyAndModelWithoutAcceptingAnEndpointOverride() {
        val config = TranslationConfiguration(provider = TextTranslationProvider.XAI,
            endpoint = "http://untrusted.example.invalid/v1", allowLocalHttp = true,
            model = "grok-custom", apiKey = "synthetic-xai-text-key")
        assertEquals("", validateTranslationConfiguration(config))
        val credentials = config.runtimeCredentials()!!
        assertEquals(setOf("kind", "apiKey", "model"), credentials.keys().asSequence().toSet())
        assertThrows(IllegalArgumentException::class.java) { config.copy(apiKey = "").runtimeCredentials() }
        for (model in listOf("", "bad\nmodel", "x".repeat(257))) {
            assertThrows(IllegalArgumentException::class.java) { config.copy(model = model).runtimeCredentials() }
        }
    }
}
