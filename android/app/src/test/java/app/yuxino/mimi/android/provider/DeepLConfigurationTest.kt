package app.yuxino.mimi.android.provider

import org.junit.Assert.*
import org.junit.Test

class DeepLConfigurationTest {
    private fun rejects(code: String, block: () -> Unit) {
        try { block(); fail("Expected $code") } catch (error: IllegalArgumentException) { assertEquals(code, error.message) }
    }

    @Test fun officialDeepLKeysChooseOnlyTheFreeOrProEndpoint() {
        fun endpoint(key: String) = normalizeTranslationEndpoint(TranslationConfiguration(apiKey = key, provider = TextTranslationProvider.DEEPL))
        assertEquals("https://api-free.deepl.com/v2/translate", endpoint(" synthetic:fx "))
        assertEquals("https://api.deepl.com/v2/translate", endpoint("synthetic"))
        for (key in listOf("", " ", "with space", "private\nkey", "秘密", "x".repeat(4097))) {
            rejects("translation_key") { endpoint(key) }
        }
        assertEquals("https://api-free.deepl.com/v2/translate", validateTranslationConfiguration(
            TranslationConfiguration("https://untrusted.test", "unused model", "synthetic:fx", provider = TextTranslationProvider.DEEPL)))
    }

    @Test fun deepLXPreservesPathPrefixesAndHasAnOptionalToken() {
        for ((input, output) in listOf(
            "https://example.test" to "https://example.test/translate",
            "https://example.test/proxy" to "https://example.test/proxy/translate",
            "https://example.test/proxy/translate/" to "https://example.test/proxy/translate",
        )) {
            assertEquals(output, validateTranslationConfiguration(TranslationConfiguration(endpoint = input, provider = TextTranslationProvider.DEEPLX)))
        }
        rejects("translation_https_required") {
            validateTranslationConfiguration(TranslationConfiguration(endpoint = "http://192.168.1.1/translate", allowLocalHttp = true, provider = TextTranslationProvider.DEEPLX))
        }
        rejects("translation_endpoint") {
            validateTranslationConfiguration(TranslationConfiguration(endpoint = "https://example.test/translate?token=private", provider = TextTranslationProvider.DEEPLX))
        }
    }

    @Test fun builtinAndOriginalOnlyNeedNoTextCredentials() {
        for (provider in listOf(TextTranslationProvider.BUILTIN, TextTranslationProvider.NONE)) {
            assertEquals("", validateTranslationConfiguration(TranslationConfiguration(provider = provider)))
        }
    }
}
