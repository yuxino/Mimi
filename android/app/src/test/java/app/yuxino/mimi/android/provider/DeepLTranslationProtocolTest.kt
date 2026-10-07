package app.yuxino.mimi.android.provider

import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test

class DeepLTranslationProtocolTest {
    private fun rejects(code: String, block: () -> Unit) {
        try { block(); fail("Expected $code") } catch (error: IllegalArgumentException) { assertEquals(code, error.message) }
    }

    @Test fun officialDeepLKeysChooseOnlyTheFreeOrProEndpoint() {
        assertEquals("https://api-free.deepl.com/v2/translate", deepLEndpoint(" synthetic:fx "))
        assertEquals("https://api.deepl.com/v2/translate", deepLEndpoint("synthetic"))
        for (key in listOf("", " ", "with space", "private\nkey", "秘密", "x".repeat(4097))) {
            rejects("translation_key") { deepLEndpoint(key) }
        }
        assertEquals("https://api-free.deepl.com/v2/translate", validateTranslationConfiguration(
            TranslationConfiguration("https://untrusted.test", "unused model", "synthetic:fx", provider = TextTranslationProvider.DEEPL)))
    }

    @Test fun deepLAndDeepLXUseDistinctDesktopProtocolShapes() {
        for ((source, code) in listOf("auto" to null, "zh" to "ZH", "en" to "EN", "ja" to "JA", "ko" to "KO")) {
            for ((target, targetCode) in listOf("zh" to "ZH", "en" to "EN", "ja" to "JA")) {
                val official = deepLRequest(" synthetic ", source, target)
                assertEquals("synthetic", official.getJSONArray("text").getString(0))
                assertEquals(1, official.getJSONArray("text").length())
                assertEquals(targetCode, official.getString("target_lang"))
                if (code == null) assertFalse(official.has("source_lang")) else assertEquals(code, official.getString("source_lang"))
                val compatible = deepLXRequest(" synthetic ", source, target)
                assertEquals("synthetic", compatible.getString("text"))
                assertEquals(code ?: "auto", compatible.getString("source_lang"))
                assertEquals(targetCode, compatible.getString("target_lang"))
            }
        }
        rejects("translation_language") { deepLRequest("synthetic", "unknown", "zh") }
        rejects("translation_language") { deepLXRequest("synthetic", "en", "bn") }
        rejects("translation_empty_source") { deepLRequest(" ", "en", "zh") }
        rejects("translation_too_large") { deepLXRequest("x".repeat(MAX_TRANSLATION_TEXT_CHARS + 1), "en", "zh") }
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

    @Test fun officialDeepLAcceptsExactlyOneNonemptyStringResult() {
        assertEquals("合成字幕", decodeDeepLResponse("""{"translations":[{"text":" 合成字幕 "}]}"""))
        rejects("translation_response") { decodeDeepLResponse("""{"translations":[]}""") }
        rejects("translation_response") { decodeDeepLResponse("""{"translations":[{"text":"one"},{"text":"two"}]}""") }
        rejects("translation_response") { decodeDeepLResponse("""{"translations":[{"text":123}]}""") }
        rejects("translation_empty_response") { decodeDeepLResponse("""{"translations":[{"text":" "}]}""") }
    }

    @Test fun deepLXChecksItsApplicationStatusEvenWhenHttpSucceeded() {
        assertEquals("合成字幕", decodeDeepLXResponse("""{"code":200,"data":" 合成字幕 "}"""))
        rejects("translation_rejected_429") { decodeDeepLXResponse("""{"code":429,"data":"private upstream error"}""") }
        rejects("translation_response") { decodeDeepLXResponse("""{"code":"200","data":"synthetic"}""") }
        rejects("translation_response") { decodeDeepLXResponse("""{"code":200.0,"data":"synthetic"}""") }
        rejects("translation_response") { decodeDeepLXResponse("""{"code":200,"data":123}""") }
        rejects("translation_empty_response") { decodeDeepLXResponse("""{"code":200,"data":" "}""") }
        val oversized = JSONObject().put("code", 200).put("data", "x".repeat(MAX_TRANSLATION_TEXT_CHARS + 1)).toString()
        rejects("translation_too_large") { decodeDeepLXResponse(oversized) }
    }

    @Test fun builtinAndOriginalOnlyNeedNoTextCredentialsAndCannotCreateNetworkClients() {
        for (provider in listOf(TextTranslationProvider.BUILTIN, TextTranslationProvider.NONE)) {
            val config = TranslationConfiguration(provider = provider)
            assertEquals("", validateTranslationConfiguration(config))
            rejects("translation_provider") { createTranslationClient(config) }
        }
        assertEquals(setOf("builtin", "none", "chatMock", "openaiCompatible", "deepL", "deepLX"), TextTranslationProvider.entries.map { it.storageId }.toSet())
        assertTrue(createTranslationClient(TranslationConfiguration(provider = TextTranslationProvider.DEEPL)) is DeepLTranslationClient)
        assertTrue(createTranslationClient(TranslationConfiguration(provider = TextTranslationProvider.DEEPLX)) is DeepLXTranslationClient)
        assertTrue(createTranslationClient(TranslationConfiguration()) is OpenAITranslationClient)
        assertTrue(createTranslationClient(TranslationConfiguration(provider = TextTranslationProvider.CHAT_MOCK)) is OpenAITranslationClient)
    }
}
