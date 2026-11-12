package app.yuxino.mimi.android.provider

import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test

/** Shared Rust/JNI contracts plus the native settings-validation boundary. */
class SharedTranslationContractTest {
    private val contract = requireNotNull(javaClass.getResourceAsStream("/translation-contracts.json")) {
        "Missing shared translation contract resource"
    }.bufferedReader().use { JSONObject(it.readText()) }.also { assertEquals(1, it.getInt("schemaVersion")) }

    @Test fun sharedOpenaiFinishPolicyCrossesActualJni() {
        val expected = contract.getJSONObject("openaiFinishPolicy")
        val policy = SharedSubtitleCore.policy
        assertEquals(expected.getLong("finishTimeoutMs"), policy.getLong("openai_finish_timeout_ms"))
        assertEquals(expected.getLong("providerFinishTimeoutMs"), policy.getLong("openai_provider_finish_timeout_ms"))
        assertTrue(policy.getLong("openai_finish_timeout_ms") < policy.getLong("openai_provider_finish_timeout_ms"))
        assertEquals(2_000L, policy.getLong("realtime_finish_timeout_ms"))
    }

    @Test fun sharedGeminiFinishPolicyCrossesActualJni() {
        val expected = contract.getJSONObject("geminiFinishPolicy")
        val policy = SharedSubtitleCore.policy
        assertEquals(expected.getLong("finishTimeoutMs"), policy.getLong("gemini_finish_timeout_ms"))
        assertEquals(expected.getLong("providerFinishTimeoutMs"), policy.getLong("provider_finish_timeout_ms"))
        assertTrue(policy.getLong("gemini_finish_timeout_ms") > expected.getLong("quietCheckpointMs"))
    }

    @Test fun initialPreviewCandidatesCrossActualJniWithoutProviderRequests() = cases("initialPreviewCandidates") { case ->
        val actual = NativeRuntimeConfiguration.exchange(JSONObject(case.toString()).put("operation", "preview_candidate"))
        assertEquals(case.getString("id"), case.get("expected"), actual.get("candidate"))
    }

    @Test fun qwenStreamingTextContractsCrossActualJniWithoutProviderRequests() = cases("qwenStreamingText") { case ->
        val actual = NativeRuntimeConfiguration.exchange(JSONObject(case.toString()).put("operation", "qwen_streaming_text"))
        assertTrue(case.getString("id"), case.getJSONArray("expected").similar(actual.getJSONArray("outputs")))
    }

    @Test fun invalidQwenStreamingTextHasSanitizedNativeErrors() {
        fun request() = JSONObject().put("operation", "qwen_streaming_text")
            .put("model", "plus").put("contents", org.json.JSONArray().put("Synthetic"))
        fun reject(value: JSONObject) {
            val error = assertThrows(IllegalArgumentException::class.java) { NativeRuntimeConfiguration.exchangeRaw(value.toString()) }
            assertEquals("native_runtime_invalid_request", error.message)
        }
        reject(request().put("model", "unknown-private-model"))
        reject(request().put("contents", org.json.JSONArray().put(12)))
        reject(request().put("contents", org.json.JSONArray(List(33) { "x" })))
        reject(request().put("contents", org.json.JSONArray().put("x".repeat(65_537))))
    }

    @Test fun previewContentChangesCrossActualJniWithoutProviderRequests() = cases("previewContentChanges") { case ->
        val actual = NativeRuntimeConfiguration.exchangeRaw(JSONObject(case.toString())
            .put("operation", "preview_content_changed").toString())
        assertEquals(case.getString("id"), case.getBoolean("expected"), actual.toBooleanStrict())
    }

    @Test fun invalidPreviewContentFieldsHaveSanitizedNativeErrors() {
        fun request() = JSONObject().put("operation", "preview_content_changed")
            .put("draft", "synthetic-private-marker").put("previousDraft", "synthetic-private-marker")
            .put("language", "en").put("previousLanguage", "en")
        fun reject(value: JSONObject) {
            val error = assertThrows(IllegalArgumentException::class.java) { NativeRuntimeConfiguration.exchangeRaw(value.toString()) }
            assertEquals("native_runtime_invalid_request", error.message)
        }
        for (field in listOf("draft", "previousDraft")) {
            for (invalid in listOf(JSONObject.NULL, 12, true, "x".repeat(65_537), "字".repeat(21_846))) {
                reject(request().put(field, invalid))
            }
        }
        for (field in listOf("language", "previousLanguage")) {
            for (invalid in listOf(12, true, "", "auto", "synthetic-private-marker", "x".repeat(65_537))) {
                reject(request().put(field, invalid))
            }
        }
        for (field in listOf("draft", "previousDraft", "language", "previousLanguage")) {
            reject(request().apply { remove(field) })
        }
        val maximum = "x".repeat(65_536)
        val unchanged = request().put("draft", maximum).put("previousDraft", maximum)
            .put("language", JSONObject.NULL).put("previousLanguage", JSONObject.NULL)
        assertEquals("false", NativeRuntimeConfiguration.exchangeRaw(unchanged.toString()))
    }

    @Test fun volcanoProto3TimingContractsCrossActualJniWithoutProviderRequests() = cases("volcanoSubtitleTimingEvents") { case ->
        val response = NativeRuntimeConfiguration.exchange(JSONObject().put("operation", "volcano_decode")
            .put("hex", case.getString("hex")))
        val expected = case.get("expected")
        val actual = response.get("event")
        assertTrue(case.getString("id"), if (expected is JSONObject) expected.similar(actual) else expected == actual)
    }

    @Test fun invalidVolcanoDecodeRequestsHaveSanitizedNativeErrors() {
        for (hex in listOf(JSONObject.NULL, 12, "0", "synthetic-private-marker", "日", "00".repeat(1_048_577))) {
            val error = assertThrows(IllegalArgumentException::class.java) {
                NativeRuntimeConfiguration.exchange(JSONObject().put("operation", "volcano_decode").put("hex", hex))
            }
            assertEquals("native_runtime_invalid_request", error.message)
        }
    }


    @Test fun invalidPreviewCandidateFieldsHaveSanitizedNativeErrors() {
        fun request() = JSONObject().put("operation", "preview_candidate").put("minimumCharacters", 12)
            .put("opening", true).put("draft", "synthetic-private-marker")
        for (invalid in listOf(request().put("minimumCharacters", 0), request().put("minimumCharacters", 65_537),
            request().put("opening", "invalid"), request().put("draft", "x".repeat(65_537)))) {
            val error = assertThrows(IllegalArgumentException::class.java) { NativeRuntimeConfiguration.exchange(invalid) }
            assertEquals("native_runtime_invalid_request", error.message)
        }
    }

    private fun cases(name: String, check: (JSONObject) -> Unit) {
        val entries = contract.getJSONArray(name)
        assertTrue("Empty $name contract", entries.length() > 0)
        for (index in 0 until entries.length()) {
            val case = entries.getJSONObject(index)
            require(case.has("id") && case.has("expected")) { "Incomplete contract case" }
            if (case.has("provider")) require(case.getString("provider") in setOf("deepL", "deepLX", "openaiCompatible", "googleGeminiLive"))
            check(case)
        }
    }

    @Test fun sharedSpeechLanguageCatalogsMatchNativeCapabilities() {
        val catalogs = contract.getJSONArray("speechLanguageCatalogs")
        repeat(catalogs.length()) { index ->
            val case = catalogs.getJSONObject(index)
            val provider = when(case.getString("provider")) { "openAIRealtime" -> ServiceProvider.OPENAI; "azureOpenAIRealtime" -> ServiceProvider.AZURE; "googleGeminiLive" -> ServiceProvider.GEMINI; "xAIRealtime" -> ServiceProvider.XAI; else -> error("unknown_provider") }
            val expected = case.getJSONObject("expected")
            assertTrue(case.getString("id"), expected.getJSONArray("sourceLanguages").similar(org.json.JSONArray(provider.sources)))
            assertTrue(case.getString("id"), expected.getJSONArray("targetLanguages").similar(org.json.JSONArray(provider.targets)))
        }
    }

    @Test fun sharedEndpointContracts() = cases("endpoints") { case ->
        val provider = TextTranslationProvider.entries.single { it.storageId == case.getString("provider") }
        val input = case.getString("input")
        val actual = runCatching {
            normalizeTranslationEndpoint(TranslationConfiguration(
                endpoint = if (provider == TextTranslationProvider.DEEPL) "" else input,
                apiKey = if (provider == TextTranslationProvider.DEEPL) input else "",
                model = "synthetic-model",
                allowLocalHttp = true,
                provider = provider,
            ))
        }
        if (case.isNull("expected")) assertTrue(case.getString("id"), actual.isFailure)
        else assertEquals(case.getString("id"), case.getString("expected"), actual.getOrThrow())
    }

    @Test fun sharedModelContracts() = cases("models") { case ->
        val actual = runCatching {
            val model = case.getString("input")
            validateTranslationConfiguration(TranslationConfiguration("https://example.test/v1", model))
            model.trim()
        }
        if (case.isNull("expected")) assertTrue(case.getString("id"), actual.isFailure)
        else assertEquals(case.getString("id"), case.getString("expected"), actual.getOrThrow())
    }

    private fun credentialsAccepted(provider: TextTranslationProvider, key: String): Boolean = runCatching {
        validateTranslationConfiguration(TranslationConfiguration("https://example.test/v1", "synthetic-model", key, provider = provider))
    }.isSuccess

    @Test fun sharedSavedCredentialContractsAndLimits() {
        cases("credentials") { case ->
            val provider = TextTranslationProvider.entries.single { it.storageId == case.getString("provider") }
            assertEquals(case.getString("id"), case.getBoolean("expected"), credentialsAccepted(provider, case.getString("apiKey")))
        }
        val limits = contract.getJSONObject("configurationLimits")
        val limit = limits.getInt("credentialUnicodeScalars")
        for (provider in listOf(TextTranslationProvider.DEEPL, TextTranslationProvider.DEEPLX, TextTranslationProvider.OPENAI_COMPATIBLE, TextTranslationProvider.CHAT_MOCK)) {
            assertTrue(provider.name, credentialsAccepted(provider, "s".repeat(limit)))
            assertFalse(provider.name, credentialsAccepted(provider, "s".repeat(limit + 1)))
            if (provider != TextTranslationProvider.DEEPL) {
                assertTrue(provider.name, credentialsAccepted(provider, "🐱".repeat(limit)))
                assertFalse(provider.name, credentialsAccepted(provider, "🐱".repeat(limit + 1)))
            }
        }
        val modelLimit = limits.getInt("modelUtf8Bytes")
        val configuration = TranslationConfiguration("https://example.test/v1", "m".repeat(modelLimit))
        assertTrue(runCatching { validateTranslationConfiguration(configuration) }.isSuccess)
        assertTrue(runCatching { validateTranslationConfiguration(configuration.copy(model = "m".repeat(modelLimit + 1))) }.isFailure)
    }

}
