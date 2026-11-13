package app.yuxino.mimi.android.provider

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import java.io.DataInputStream
import java.io.OutputStream
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.nio.file.Files
import java.security.KeyStore
import java.security.MessageDigest
import java.util.Base64
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicReference
import javax.net.ssl.KeyManagerFactory
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLSocket
import javax.net.ssl.TrustManagerFactory
import javax.net.ssl.X509TrustManager
import kotlin.concurrent.thread

/** Actual JNI checks use synthetic credentials; live transport fixtures stay on loopback. */
class NativeRuntimeJniTest {
    @Test fun azureCredentialDraftsRetainKeysOnlyForTheSameResourceAcrossActualJni() {
        val saved = JSONObject().put("kind", "azureOpenAI").put("endpoint", "https://synthetic.openai.azure.com")
            .put("deployment", "translation-alias").put("transcriptionDeployment", "speech-alias").put("apiKey", "synthetic-old-key")
        val blank = JSONObject().put("kind", "azureOpenAI").put("endpoint", "").put("deployment", "")
            .put("transcriptionDeployment", "").put("apiKey", "")
        val retained = NativeRuntimeConfiguration.resolveCredentialsDraft(ServiceProvider.AZURE, "simultaneous", blank, saved)
        assertEquals(saved.getString("endpoint"), retained.getString("endpoint"))
        assertEquals("synthetic-old-key", retained.getString("apiKey"))
        assertEquals("translation-alias", retained.getString("deployment"))
        assertEquals("speech-alias", retained.getString("transcriptionDeployment"))
        assertEquals("invalid_credentials_draft", assertThrows(IllegalArgumentException::class.java) {
            NativeRuntimeConfiguration.resolveCredentialsDraft(ServiceProvider.AZURE, "simultaneous",
                JSONObject(blank.toString()).put("transcriptionDeployment", "invalid deployment"), saved)
        }.message)
        val moved = JSONObject(blank.toString()).put("endpoint", "https://synthetic-replacement.openai.azure.com")
        assertEquals("missing_credentials_draft", assertThrows(IllegalArgumentException::class.java) {
            NativeRuntimeConfiguration.resolveCredentialsDraft(ServiceProvider.AZURE, "simultaneous", moved, saved)
        }.message)
        assertEquals("synthetic-replacement-key", NativeRuntimeConfiguration.resolveCredentialsDraft(
            ServiceProvider.AZURE, "simultaneous", JSONObject(moved.toString()).put("apiKey", "synthetic-replacement-key"), saved).getString("apiKey"))
        assertEquals("synthetic-replacement-key", NativeRuntimeConfiguration.resolveCredentialsDraft(
            ServiceProvider.AZURE, "simultaneous", JSONObject(moved.toString()).put("apiKey", "synthetic-replacement-key"),
            JSONObject(saved.toString()).put("endpoint", "synthetic-damaged-endpoint")).getString("apiKey"))

        val asrSaved = JSONObject(saved.toString()).put("deployment", "")
        val asr = NativeRuntimeConfiguration.resolveCredentialsDraft(ServiceProvider.AZURE, "asrMt", blank, asrSaved)
        assertEquals("", asr.getString("deployment"))
        assertEquals("speech-alias", asr.getString("transcriptionDeployment"))
        assertThrows(IllegalArgumentException::class.java) {
            NativeRuntimeConfiguration.resolveCredentialsDraft(ServiceProvider.AZURE, "simultaneous", blank, asrSaved)
        }
    }

    @Test fun tencentCredentialDraftsBindBothSavedSecretsToTheirOriginalOwnerAcrossActualJni() {
        val saved = JSONObject().put("kind", "tencentCloud").put("appId", "123456789")
            .put("secretId", "synthetic-secret-id").put("secretKey", "synthetic-secret-key")
        val blank = JSONObject().put("kind", "tencentCloud").put("appId", "").put("secretId", "").put("secretKey", "")
        val retained = NativeRuntimeConfiguration.resolveCredentialsDraft(ServiceProvider.TENCENT, "simultaneous", blank, saved)
        assertEquals("123456789", retained.getString("appId"))
        assertEquals("synthetic-secret-id", retained.getString("secretId"))
        assertEquals("synthetic-secret-key", retained.getString("secretKey"))
        for (moved in listOf(JSONObject(blank.toString()).put("appId", "987654321"),
                JSONObject(blank.toString()).put("secretId", "synthetic-replacement-id"))) {
            assertEquals("missing_credentials_draft", assertThrows(IllegalArgumentException::class.java) {
                NativeRuntimeConfiguration.resolveCredentialsDraft(ServiceProvider.TENCENT, "simultaneous", moved, saved)
            }.message)
        }
        val replacement = JSONObject(blank.toString()).put("appId", "987654321")
            .put("secretId", "synthetic-replacement-id").put("secretKey", "synthetic-replacement-key")
        val resolved = NativeRuntimeConfiguration.resolveCredentialsDraft(ServiceProvider.TENCENT, "simultaneous", replacement, saved)
        assertEquals("987654321", resolved.getString("appId"))
        assertEquals("synthetic-replacement-id", resolved.getString("secretId"))
        assertEquals("synthetic-replacement-key", resolved.getString("secretKey"))
    }

    @Test fun legacyAzureEndpointSpellingKeepsTheSameResourceKeyAcrossActualJni() {
        val saved = JSONObject().put("kind", "azureOpenAI").put("endpoint", "https://synthetic.openai.azure.com/")
            .put("deployment", "translation-alias").put("transcriptionDeployment", "speech-alias").put("apiKey", "synthetic-old-key")
        val blank = JSONObject().put("kind", "azureOpenAI").put("endpoint", "").put("deployment", "")
            .put("transcriptionDeployment", "").put("apiKey", "")
        for (endpoint in listOf("", "https://synthetic.openai.azure.com")) {
            val resolved = NativeRuntimeConfiguration.resolveCredentialsDraft(ServiceProvider.AZURE, "simultaneous",
                JSONObject(blank.toString()).put("endpoint", endpoint), saved)
            assertEquals("https://synthetic.openai.azure.com", resolved.getString("endpoint"))
            assertEquals("synthetic-old-key", resolved.getString("apiKey"))
        }
    }

    @Test fun baiduCredentialDraftsNeverReuseAKeyForAnotherApplicationAcrossActualJni() {
        val saved = JSONObject().put("kind", "baiduTranslate").put("appId", "123456789").put("appKey", "synthetic-old-key")
        val blank = JSONObject().put("kind", "baiduTranslate").put("appId", "").put("appKey", "")
        assertEquals("synthetic-old-key", NativeRuntimeConfiguration.resolveCredentialsDraft(
            ServiceProvider.BAIDU, "simultaneous", blank, saved).getString("appKey"))
        val moved = JSONObject(blank.toString()).put("appId", "987654321")
        assertEquals("missing_credentials_draft", assertThrows(IllegalArgumentException::class.java) {
            NativeRuntimeConfiguration.resolveCredentialsDraft(ServiceProvider.BAIDU, "simultaneous", moved, saved)
        }.message)
        assertEquals("synthetic-new-key", NativeRuntimeConfiguration.resolveCredentialsDraft(ServiceProvider.BAIDU,
            "simultaneous", JSONObject(moved.toString()).put("appKey", "synthetic-new-key"), saved).getString("appKey"))
    }

    @Test fun singleKeyCredentialDraftsAndInvalidRequestsUseSharedValidationAcrossActualJni() {
        val saved = JSONObject().put("kind", "apiKey").put("apiKey", "synthetic-old-key")
        val blank = JSONObject().put("kind", "apiKey").put("apiKey", "")
        for (provider in listOf(ServiceProvider.DASHSCOPE, ServiceProvider.OPENAI, ServiceProvider.GEMINI,
                ServiceProvider.VOLCANO, ServiceProvider.XAI)) {
            assertEquals("synthetic-old-key", NativeRuntimeConfiguration.resolveCredentialsDraft(
                provider, "simultaneous", blank, saved).getString("apiKey"))
            assertEquals("synthetic-new-key", NativeRuntimeConfiguration.resolveCredentialsDraft(provider, "simultaneous",
                JSONObject(saved.toString()).put("apiKey", "synthetic-new-key"), null).getString("apiKey"))
            assertEquals("missing_credentials_draft", assertThrows(IllegalArgumentException::class.java) {
                NativeRuntimeConfiguration.resolveCredentialsDraft(provider, "simultaneous", blank, null)
            }.message)
        }
        assertEquals("native_runtime_invalid_request", assertThrows(IllegalArgumentException::class.java) {
            NativeRuntimeConfiguration.resolveCredentialsDraft(ServiceProvider.OPENAI, "synthetic-invalid-mode", saved, null)
        }.message)
        assertEquals("invalid_credentials_draft", assertThrows(IllegalArgumentException::class.java) {
            NativeRuntimeConfiguration.resolveCredentialsDraft(ServiceProvider.VOLCANO, "asrMt", saved, null)
        }.message)
        assertEquals("native_runtime_invalid_request", assertThrows(IllegalArgumentException::class.java) {
            NativeRuntimeConfiguration.resolveCredentialsDraft(ServiceProvider.OPENAI, "simultaneous",
                JSONObject(saved.toString()).put("synthetic-private-field", "synthetic-private-value"), null)
        }.message)
    }

    @Test fun realtimeIndependentModesUseSharedFactoriesAndSeparateTextCredentialsAcrossActualJni() {
        val translator = TranslationConfiguration(provider = TextTranslationProvider.XAI,
            apiKey = "synthetic-independent-key", model = "grok-independent")
        for (provider in listOf(ServiceProvider.OPENAI, ServiceProvider.GEMINI, ServiceProvider.AZURE)) {
            val simultaneous = configuration(provider)
            assertEquals("simultaneous", simultaneous.getString("speechTranslationMode"))
            val originalIntegratedEndpoint = NativeRuntimeConfiguration.endpoints(simultaneous).getString("speechEndpoint")
            val legacy = JSONObject(simultaneous.toString()).apply { remove("speechTranslationMode") }
            assertEquals(originalIntegratedEndpoint, NativeRuntimeConfiguration.endpoints(legacy).getString("speechEndpoint"))
            assertTrue(NativeRuntimeConfiguration.endpoints(legacy).isNull("textEndpoint"))
            val asr = JSONObject(simultaneous.toString()).put("speechTranslationMode", "asrMt")
                .put("sourceLanguage", "en").put("textCredentials", translator.runtimeCredentials())
            if (provider == ServiceProvider.AZURE) asr.getJSONObject("credentials").put("deployment", "")
            val endpoints = NativeRuntimeConfiguration.endpoints(asr)
            assertEquals("https://api.x.ai/v1/chat/completions", endpoints.getString("textEndpoint"))
            assertTrue(endpoints.getJSONArray("modelNames").toString().contains("grok-independent"))
            assertFalse(endpoints.toString().contains("synthetic-independent-key"))
            val handle = create(asr)
            try { assertEquals("connecting", NativeRuntimeConfiguration.poll(handle).getString("connectionStatus")) }
            finally { NativeRuntimeConfiguration.command("stop", handle) }
            val missingText = JSONObject(asr.toString()).put("textCredentials", JSONObject.NULL)
            assertThrows(IllegalArgumentException::class.java) { create(missingText) }
            val original = JSONObject(missingText.toString()).put("targetLanguage", "original")
            assertTrue(NativeRuntimeConfiguration.endpoints(original).isNull("textEndpoint"))
            assertEquals(1, NativeRuntimeConfiguration.endpoints(original).getJSONArray("modelNames").length())
            if (provider == ServiceProvider.AZURE) {
                assertThrows(IllegalArgumentException::class.java) {
                    NativeRuntimeConfiguration.endpoints(JSONObject(asr.toString()).put("speechTranslationMode", "simultaneous"))
                }
            }
        }
    }

    @Test fun realtimeModeCapabilitiesAndInvalidModesStayInSharedRustAcrossActualJni() {
        for (provider in listOf(ServiceProvider.OPENAI, ServiceProvider.GEMINI, ServiceProvider.AZURE)) {
            val independent = NativeRuntimeConfiguration.capabilities(provider.wireProvider, "openAICompatible", "zh",
                speechTranslationMode = "asrMt")
            assertEquals(if (provider == ServiceProvider.GEMINI) 16_000 else 24_000, independent.getInt("sampleRateHz"))
            assertTrue(independent.getJSONArray("sourceLanguages").toString().contains("auto"))
            assertTrue(independent.getJSONArray("targetLanguages").toString().contains("original"))
            val original = NativeRuntimeConfiguration.normalize(provider.wireProvider, "openAICompatible", "en", "original",
                speechTranslationMode = "asrMt")
            assertEquals("en", original.getString("sourceLanguage"))
            assertEquals("original", original.getString("targetLanguage"))
            for (invalid in listOf("", "voice", "unknown")) {
                assertThrows(IllegalArgumentException::class.java) {
                    NativeRuntimeConfiguration.capabilities(provider.wireProvider, "openAICompatible", "zh", speechTranslationMode = invalid)
                }
            }
        }
        assertThrows(IllegalArgumentException::class.java) {
            NativeRuntimeConfiguration.capabilities("volcanoEngine", "openAICompatible", "zh", speechTranslationMode = "asrMt")
        }
    }

    @Test fun geminiStreamingTranscriptContractsUseTheSharedAssemblerAcrossActualJni() {
        val cases = JSONObject(requireNotNull(javaClass.getResourceAsStream("/translation-contracts.json"))
            .bufferedReader().use { it.readText() }).getJSONArray("geminiStreamingTranscripts")
        for (index in 0 until cases.length()) {
            val case = cases.getJSONObject(index)
            val outputs = NativeRuntimeConfiguration.exchange(JSONObject().put("operation", "gemini_streaming_transcripts")
                .put("frames", case.getJSONArray("frames"))).getJSONArray("outputs")
            val expected = case.getJSONArray("expectedOutputs")
            assertEquals(case.getString("id"), expected.length(), outputs.length())
            for (frame in 0 until expected.length()) {
                val wanted = expected.getJSONArray(frame); val actual = outputs.getJSONArray(frame)
                assertEquals(case.getString("id"), wanted.length(), actual.length())
                for (item in 0 until wanted.length()) {
                    assertEquals(case.getString("id"), wanted.getJSONObject(item).getLong("sequence"), actual.getJSONObject(item).getLong("sequence"))
                    assertEquals(case.getString("id"), wanted.getJSONObject(item).getString("text"), actual.getJSONObject(item).getString("text"))
                    assertEquals(case.getString("id"), wanted.getJSONObject(item).getBoolean("isFinal"), actual.getJSONObject(item).getBoolean("isFinal"))
                }
            }
        }
    }

    @Test fun xaiStreamingTranscriptContractsUseTheSharedAssemblerAcrossActualJni() {
        val cases = JSONObject(requireNotNull(javaClass.getResourceAsStream("/translation-contracts.json"))
            .bufferedReader().use { it.readText() }).getJSONArray("xaiStreamingTranscripts")
        for (index in 0 until cases.length()) {
            val case = cases.getJSONObject(index)
            val outputs = NativeRuntimeConfiguration.exchange(JSONObject().put("operation", "xai_streaming_transcripts")
                .put("frames", case.getJSONArray("frames"))).getJSONArray("outputs")
            val expected = case.getJSONArray("expectedOutputs")
            assertEquals(case.getString("id"), expected.length(), outputs.length())
            for (frame in 0 until expected.length()) {
                assertEquals(case.getString("id"), expected.isNull(frame), outputs.isNull(frame))
                if (!expected.isNull(frame)) {
                    val wanted = expected.getJSONObject(frame); val actual = outputs.getJSONObject(frame)
                    assertEquals(case.getString("id"), wanted.getLong("sequence"), actual.getLong("sequence"))
                    assertEquals(case.getString("id"), wanted.getString("text"), actual.getString("text"))
                    assertEquals(case.getString("id"), wanted.getBoolean("isFinal"), actual.getBoolean("isFinal"))
                }
            }
        }
    }

    @Test fun xaiStreamingModeAndLegacyVoiceUseTheirActualSharedFactoriesAcrossJni() {
        val streaming = configuration(ServiceProvider.XAI).put("xaiSpeechMode", "asrMt")
            .put("xaiTextModel", "legacy-profile-model").put("sourceLanguage", "en")
            .put("textCredentials", TranslationConfiguration(provider = TextTranslationProvider.XAI,
                model = "grok-custom-text", apiKey = "synthetic-independent-text-key").runtimeCredentials())
        val endpoints = NativeRuntimeConfiguration.endpoints(streaming)
        assertEquals("wss://api.x.ai/v1/stt", endpoints.getString("speechEndpoint"))
        assertEquals("https://api.x.ai/v1/chat/completions", endpoints.getString("textEndpoint"))
        val models = endpoints.getJSONArray("modelNames")
        assertEquals(listOf("grok-voice-transcribe-2.0", "grok-custom-text"), (0 until models.length()).map(models::getString))
        assertFalse(endpoints.toString().contains("synthetic-apiKey"))
        assertFalse(endpoints.toString().contains("synthetic-independent-text-key"))
        assertFalse(models.toString().contains("legacy-profile-model"))
        val handle = create(streaming)
        try { assertEquals("connecting", NativeRuntimeConfiguration.poll(handle).getString("connectionStatus")) }
        finally { NativeRuntimeConfiguration.command("stop", handle) }

        val original = JSONObject(streaming.toString()).put("targetLanguage", "original")
        val originalEndpoints = NativeRuntimeConfiguration.endpoints(original)
        assertTrue(originalEndpoints.isNull("textEndpoint"))
        assertEquals("[\"grok-voice-transcribe-2.0\"]", originalEndpoints.getJSONArray("modelNames").toString())
        val textProbe = NativeRuntimeConfiguration.exchange(JSONObject().put("operation", "text_probe_endpoint")
            .put("configuration", buildTextProbeConfiguration(TranslationConfiguration(provider = TextTranslationProvider.XAI,
                model = "grok-probe-model", apiKey = "synthetic-probe-key"), "en", "zh", "lite")))
        assertEquals("https://api.x.ai/v1/chat/completions", textProbe.getString("textEndpoint"))
        assertFalse(textProbe.has("speechEndpoint"))

        val legacy = configuration(ServiceProvider.XAI).apply { remove("xaiSpeechMode"); remove("xaiTextModel") }
        val voice = NativeRuntimeConfiguration.endpoints(legacy)
        assertEquals("wss://api.x.ai/v1/realtime", voice.getString("speechEndpoint"))
        assertTrue(voice.isNull("textEndpoint"))
        assertTrue(voice.getJSONArray("modelNames").toString().contains("grok-voice-latest"))
    }

    @Test fun xaiStreamingTransportRecoversAndAuthenticationStopsAcrossActualJni() {
        val cases = JSONObject(requireNotNull(javaClass.getResourceAsStream("/translation-contracts.json"))
            .bufferedReader().use { it.readText() }).getJSONArray("xaiStreamingRecovery")
        for (index in 0 until cases.length()) {
            val case = cases.getJSONObject(index)
            val id = case.getString("id")
            RealtimeLoopback(ServiceProvider.XAI, xaiRecovery = case).use { fixture ->
                val config = configuration(ServiceProvider.XAI).put("xaiSpeechMode", "asrMt")
                    .put("sourceLanguage", "en").put("targetLanguage", "original")
                val handle = NativeRuntimeConfiguration.exchange(JSONObject().put("operation", "create")
                    .put("configuration", config).put("network", fixture.network)).getLong("handle")
                try {
                    NativeRuntimeConfiguration.command("start", handle)
                    awaitRuntimeState(handle, fixture) { it.getString("connectionStatus") == "listening" }
                    fixture.nextMid.countDown()
                    if (case.isNull("expectedErrorCode")) {
                        val recovered = awaitRuntimeState(handle, fixture) {
                            assertFalse("$id terminated instead of reconnecting: ${it.optString("errorCode")}", it.getBoolean("finished"))
                            fixture.reconnected.count == 0L && it.getString("connectionStatus") == "listening"
                        }
                        assertTrue(id, recovered.isNull("errorCode"))
                        assertFalse(id, recovered.getBoolean("finished"))
                        NativeRuntimeConfiguration.command("finish", handle)
                        awaitRuntimeState(handle, fixture) { it.getBoolean("finished") }
                    } else {
                        val failed = awaitRuntimeState(handle, fixture, expectedError = case.getString("expectedErrorCode")) { it.getBoolean("finished") }
                        assertEquals(id, case.getString("expectedErrorCode"), failed.getString("errorCode"))
                        assertFalse(id, failed.toString().contains("synthetic-private-detail"))
                    }
                    assertTrue(id, fixture.noFurtherConnections.await(2, TimeUnit.SECONDS))
                    fixture.checkFailure()
                    assertEquals(id, case.getInt("expectedConnections"), fixture.connectionCount.get())
                } finally { NativeRuntimeConfiguration.command("stop", handle) }
            }
        }
    }

    @Test fun xaiRuntimeNeverFillsAMissingTextKeyFromTheSpeechKeyAcrossActualJni() {
        val speech = ServiceConfiguration(ServiceProvider.XAI, mapOf("apiKey" to "synthetic-speech-only-key"), xaiSpeechMode = "asrMt")
        val missing = buildRuntimeConfiguration(speech, TranslationConfiguration(provider = TextTranslationProvider.NONE), "en", "zh", "lite")
            .put("targetLanguage", "zh")
        assertThrows(IllegalArgumentException::class.java) { create(missing) }
        missing.put("textCredentials", JSONObject().put("kind", "xAI").put("apiKey", "").put("model", "grok-custom-text"))
        assertThrows(IllegalArgumentException::class.java) { create(missing) }
        missing.put("targetLanguage", "original")
        assertTrue(NativeRuntimeConfiguration.endpoints(missing).isNull("textEndpoint"))
    }

    @Test fun xaiRecognitionAcceptsExistingTextServicesAndGrokTextAcceptsAlibabaAcrossActualJni() {
        val speech = ServiceConfiguration(ServiceProvider.XAI, mapOf("apiKey" to "synthetic-speech-key"), xaiSpeechMode = "asrMt")
        val routes = listOf(
            TranslationConfiguration(provider = TextTranslationProvider.DEEPL, apiKey = "synthetic-text-key:fx") to "https://api-free.deepl.com/v2/translate",
            TranslationConfiguration(provider = TextTranslationProvider.DEEPLX, endpoint = "https://deeplx.example.invalid/translate") to "https://deeplx.example.invalid/translate",
            TranslationConfiguration(provider = TextTranslationProvider.OPENAI_COMPATIBLE, endpoint = "https://custom.example.invalid/v1", model = "custom-model") to "https://custom.example.invalid/v1/chat/completions",
            TranslationConfiguration(provider = TextTranslationProvider.CHAT_MOCK, endpoint = "http://127.0.0.1:8000/v1", model = "local-model", allowLocalHttp = true) to "http://127.0.0.1:8000/v1/chat/completions",
            TranslationConfiguration(provider = TextTranslationProvider.XAI, apiKey = "synthetic-grok-key", model = "grok-custom-text") to "https://api.x.ai/v1/chat/completions",
        )
        for ((text, expected) in routes) {
            val configuration = buildRuntimeConfiguration(speech, text, "en", "zh", "lite")
            val endpoints = NativeRuntimeConfiguration.endpoints(configuration)
            assertEquals(text.provider.storageId, "wss://api.x.ai/v1/stt", endpoints.getString("speechEndpoint"))
            assertEquals(text.provider.storageId, expected, endpoints.getString("textEndpoint"))
            assertFalse(endpoints.toString().contains("synthetic-"))
        }
        val alibaba = ServiceConfiguration(ServiceProvider.DASHSCOPE, mapOf("apiKey" to "synthetic-alibaba-key"))
        val grok = routes.last().first
        val endpoints = NativeRuntimeConfiguration.endpoints(buildRuntimeConfiguration(alibaba, grok, "en", "zh", "lite"))
        assertEquals("https://api.x.ai/v1/chat/completions", endpoints.getString("textEndpoint"))
        assertTrue(endpoints.getJSONArray("modelNames").toString().contains("grok-custom-text"))
        for (route in listOf(TextTranslationProvider.BUILTIN, TextTranslationProvider.NONE)) {
            val original = buildRuntimeConfiguration(speech, TranslationConfiguration(provider = route), "en", "zh", "lite")
            assertTrue(NativeRuntimeConfiguration.endpoints(original).isNull("textEndpoint"))
        }
    }

    @Test fun xaiModeCapabilitiesAndNormalizationStayInTheSharedCatalogAcrossJni() {
        val catalogs = JSONObject(requireNotNull(javaClass.getResourceAsStream("/translation-contracts.json"))
            .bufferedReader().use { it.readText() }).getJSONObject("textLanguageCatalogs")
        fun languages(provider: String, field: String): Set<String> = catalogs.getJSONObject(provider).getJSONArray(field).let { values ->
            (0 until values.length()).map(values::getString).toSet()
        }
        val voice = NativeRuntimeConfiguration.capabilities("xAIRealtime", "followService", "zh")
        assertEquals(24_000, voice.getInt("sampleRateHz"))
        val streaming = NativeRuntimeConfiguration.capabilities("xAIRealtime", "followService", "zh", xaiSpeechMode = "asrMt")
        assertEquals(16_000, streaming.getInt("sampleRateHz"))
        val targets = streaming.getJSONArray("targetLanguages")
        assertEquals(listOf("original"), (0 until targets.length()).map(targets::getString))
        val recognition = streaming.getJSONArray("sourceLanguages").let { values ->
            (0 until values.length()).map(values::getString).toSet()
        }
        assertTrue("auto" in recognition)
        assertEquals("original", NativeRuntimeConfiguration.normalize("xAIRealtime", "followService", "en", "original",
            xaiSpeechMode = "asrMt").getString("targetLanguage"))
        val deepL = NativeRuntimeConfiguration.capabilities("xAIRealtime", "deepL", "zh", xaiSpeechMode = "asrMt")
        assertEquals(16_000, deepL.getInt("sampleRateHz"))
        val deepLSources = deepL.getJSONArray("sourceLanguages").let { values -> (0 until values.length()).map(values::getString).toSet() }
        val deepLTargets = deepL.getJSONArray("targetLanguages").let { values -> (0 until values.length()).map(values::getString).toSet() }
        assertEquals(recognition.intersect(languages("deepL", "sourceLanguages") + "auto"), deepLSources)
        assertEquals(languages("deepL", "targetLanguages") + "original", deepLTargets)
        val deepLX = NativeRuntimeConfiguration.capabilities("xAIRealtime", "deepLX", "zh", xaiSpeechMode = "asrMt")
        val deepLXSources = deepLX.getJSONArray("sourceLanguages").let { values -> (0 until values.length()).map(values::getString).toSet() }
        assertEquals(recognition.intersect(languages("deepLX", "sourceLanguages") + "auto"), deepLXSources)
        assertFalse("yue" in deepLXSources)
        val grok = NativeRuntimeConfiguration.capabilities("xAIRealtime", "xAI", "zh", xaiSpeechMode = "asrMt")
        assertTrue(grok.getJSONArray("targetLanguages").toString().contains("zh_tw"))
        for (invalid in listOf("", "unknown")) {
            assertThrows(IllegalArgumentException::class.java) {
                NativeRuntimeConfiguration.capabilities("xAIRealtime", "followService", "zh", xaiSpeechMode = invalid)
            }
        }
    }

    @Test fun qwenCompletionContractsPreventPartialConfirmationsAcrossActualJni() {
        val cases = JSONObject(requireNotNull(javaClass.getResourceAsStream("/translation-contracts.json"))
            .bufferedReader().use { it.readText() }).getJSONArray("qwenCompletionResponses")
        for (index in 0 until cases.length()) {
            val case = cases.getJSONObject(index)
            val id = case.getString("id")
            RealtimeLoopback(ServiceProvider.DASHSCOPE, case).use { fixture ->
                val config = configuration().put("sourceLanguage", "en")
                    .put("qwenMtModel", if (case.getBoolean("stream")) "lite" else "plus")
                val handle = NativeRuntimeConfiguration.exchange(JSONObject().put("operation", "create")
                    .put("configuration", config).put("network", fixture.network)).getLong("handle")
                fun awaitState(predicate: (JSONObject) -> Boolean): JSONObject {
                    val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(8)
                    while (System.nanoTime() < deadline) {
                        fixture.checkFailure()
                        val state = NativeRuntimeConfiguration.command("poll", handle)
                        if (predicate(state)) return state
                        Thread.sleep(10)
                    }
                    error("$id Qwen loopback state did not arrive")
                }
                try {
                    NativeRuntimeConfiguration.command("start", handle)
                    if (case.isNull("expected")) {
                        val failed = awaitState { !it.isNull("errorCode") }
                        assertEquals(id, "provider_error", failed.getString("errorCode"))
                        assertEquals(id, 0, failed.getJSONObject("snapshot").getJSONArray("history").length())
                        assertFalse(id, failed.getJSONObject("snapshot").getBoolean("displayPairFinal"))
                        assertFalse(id, failed.toString().contains("synthetic-apiKey"))
                        val terminal = awaitState { it.getBoolean("finished") }
                        val light = NativeRuntimeConfiguration.poll(handle, terminal.getLong("version"))
                        assertTrue(id, light.getBoolean("unchanged"))
                        assertTrue(id, light.getBoolean("finished"))
                        assertEquals(id, "provider_error", light.getString("errorCode"))
                        assertFalse(id, light.has("snapshot"))
                    } else {
                        val confirmed = awaitState { it.getJSONObject("snapshot").getJSONArray("history").length() == 1 }
                        assertTrue(id, confirmed.isNull("errorCode"))
                        val pair = confirmed.getJSONObject("snapshot").getJSONArray("history").getJSONObject(0)
                        assertEquals(id, "Synthetic Qwen source.", pair.getString("source"))
                        assertEquals(id, case.getString("expected"), pair.getString("translation"))
                        NativeRuntimeConfiguration.command("finish", handle)
                        awaitState { it.getBoolean("finished") }
                    }
                    fixture.checkFailure()
                } finally { NativeRuntimeConfiguration.command("stop", handle) }
            }
        }
    }
    @Test fun stopWatchdogIncludesNativeDrainDisconnectAndSnapshotPublication() {
        val policy = NativeRuntimeConfiguration.policy
        assertEquals(60L, policy.getLong("snapshotPublishIntervalMs"))
        val providerFinish = SharedSubtitleCore.policy.getLong("provider_finish_timeout_ms")
        assertTrue(policy.getLong("sessionFinishTimeoutMs") > providerFinish + 1_000L + 2_000L)
        val openaiFinish = SharedSubtitleCore.policy.getLong("openai_provider_finish_timeout_ms")
        assertTrue(policy.getLong("sessionFinishTimeoutMs") > openaiFinish + 1_000L + 2_000L)
        val geminiFinish = SharedSubtitleCore.policy.getLong("gemini_finish_timeout_ms")
        assertEquals(4_500L, geminiFinish)
        assertTrue(geminiFinish > 2_000L)
        assertTrue(geminiFinish < providerFinish)
    }
    private val network: JSONObject by lazy {
        val factory = TrustManagerFactory.getInstance(TrustManagerFactory.getDefaultAlgorithm())
        factory.init(null as KeyStore?)
        val roots = factory.trustManagers.filterIsInstance<X509TrustManager>().single().acceptedIssuers
        JSONObject().put("speechProxy", JSONObject().put("mode", "direct").put("url", JSONObject.NULL))
            .put("textProxy", JSONObject().put("mode", "direct").put("url", JSONObject.NULL))
            .put("trustRoots", JSONArray(roots.map { Base64.getEncoder().encodeToString(it.encoded) }))
    }

    private fun configuration(provider: ServiceProvider = ServiceProvider.DASHSCOPE): JSONObject {
        val values = provider.fields.associate { field -> field.id to when (field.id) {
            "endpoint" -> "https://synthetic.openai.azure.com"
            "appId" -> "123456789"
            else -> "synthetic-${field.id}"
        } }
        val (source, target) = provider.normalize("auto", "zh")
        return buildRuntimeConfiguration(ServiceConfiguration(provider, values),
            TranslationConfiguration(provider = TextTranslationProvider.BUILTIN), source, target, "lite")
    }

    private fun create(configuration: JSONObject): Long = NativeRuntimeConfiguration.exchange(
        JSONObject().put("operation", "create").put("configuration", configuration).put("network", network)).getLong("handle")

    @Test fun incrementalPollKeepsLegacySnapshotsAndRejectsInvalidVersionsAcrossActualJni() {
        val handle = create(configuration())
        try {
            val first = NativeRuntimeConfiguration.poll(handle)
            val version = first.getLong("version")
            assertTrue(first.has("snapshot"))
            assertFalse(first.optBoolean("unchanged"))
            val mismatched = NativeRuntimeConfiguration.poll(handle, version + 1)
            assertTrue(mismatched.has("snapshot"))
            assertFalse(mismatched.optBoolean("unchanged"))

            val light = NativeRuntimeConfiguration.poll(handle, version)
            assertTrue(light.getBoolean("unchanged"))
            assertEquals(version, light.getLong("version"))
            assertFalse(light.has("snapshot"))
            assertFalse(light.getBoolean("finished"))
            assertTrue(light.isNull("errorCode"))
            val legacy = NativeRuntimeConfiguration.command("poll", handle)
            assertTrue(legacy.has("snapshot"))
            assertEquals(first.getJSONObject("snapshot").toString(), legacy.getJSONObject("snapshot").toString())

            for (invalid in listOf(-1L, "2", true, 1.5)) {
                assertEquals("native_runtime_invalid_request", assertThrows(IllegalArgumentException::class.java) {
                    NativeRuntimeConfiguration.exchange(JSONObject().put("operation", "poll").put("handle", handle)
                        .put("sinceVersion", invalid))
                }.message)
            }
            NativeRuntimeConfiguration.command("stop", handle)
            assertEquals("native_runtime_stale_handle", assertThrows(IllegalArgumentException::class.java) {
                NativeRuntimeConfiguration.poll(handle, version)
            }.message)
        } finally { NativeRuntimeConfiguration.command("stop", handle) }
    }

    @Test fun allSavedProviderCredentialShapesCrossTheActualNativeBoundaryWithoutConnecting() {
        for (provider in ServiceProvider.entries) {
            val config = configuration(provider)
            val handle = create(config)
            try {
                val state = NativeRuntimeConfiguration.command("poll", handle)
                assertTrue(state.has("snapshot"))
                assertEquals("connecting", state.getString("connectionStatus"))
                assertFalse(state.getBoolean("finished"))
                assertFalse(state.toString().contains("synthetic-apiKey"))
                assertFalse(state.toString().contains("synthetic-secretKey"))
                assertFalse(state.toString().contains("synthetic-appKey"))
                assertFalse(state.has("configuration"))
            } finally { NativeRuntimeConfiguration.command("stop", handle) }
        }
    }

    @Test fun everyLiveTranslateTextTargetCrossesActualJniWithoutConnecting() {
        val contract = JSONObject(requireNotNull(javaClass.getResourceAsStream("/translation-contracts.json"))
            .bufferedReader().use { it.readText() })
        val languages = contract.getJSONObject("alibabaRealtimeLanguages").getJSONArray("languages")
        assertEquals(60, languages.length())
        for (index in 0 until languages.length()) {
            val target = languages.getJSONObject(index).getString("code")
            val config = configuration().put("alibabaRealtimeMode", "liveTranslate")
                .put("alibabaWorkspaceId", "synthetic-workspace").put("sourceLanguage", "auto").put("targetLanguage", target)
            val handle = create(config)
            try {
                assertEquals("connecting", NativeRuntimeConfiguration.command("poll", handle).getString("connectionStatus"))
                val endpoints = NativeRuntimeConfiguration.exchange(JSONObject().put("operation", "endpoints").put("configuration", config))
                assertTrue(endpoints.getJSONArray("modelNames").toString().contains("qwen3.8-livetranslate-flash-realtime"))
            } finally { NativeRuntimeConfiguration.command("stop", handle) }
        }
        for (target in listOf("zh_tw", "ta")) {
            val config = configuration().put("alibabaRealtimeMode", "liveTranslate")
                .put("alibabaWorkspaceId", "synthetic-workspace").put("sourceLanguage", "auto").put("targetLanguage", target)
            assertEquals("invalid_configuration", assertThrows(IllegalArgumentException::class.java) { create(config) }.message)
        }
    }

    @Test fun savedLiveTranslateModeUsesSharedCapabilitiesAndIgnoresRetainedTextSettings() {
        val speech = ServiceConfiguration(ServiceProvider.DASHSCOPE, mapOf("apiKey" to "synthetic-speech-key"),
            alibabaRealtimeMode = "liveTranslate", alibabaWorkspaceId = "synthetic-workspace")
        // An incomplete saved independent route must not block simultaneous translation.
        val text = TranslationConfiguration(provider = TextTranslationProvider.DEEPLX, endpoint = "")
        val config = buildRuntimeConfiguration(speech, text, "auto", "fr", "plus")
        assertEquals("alibabaCloud", config.getString("provider"))
        assertEquals("apiKey", config.getJSONObject("credentials").getString("kind"))
        assertEquals("liveTranslate", config.getString("alibabaRealtimeMode"))
        assertTrue(config.isNull("textCredentials"))
        assertTrue(NativeRuntimeConfiguration.endpoints(config).isNull("textEndpoint"))
        assertEquals(listOf("qwen3.8-livetranslate-flash-realtime"), app.yuxino.mimi.android.runtimeModelNames(config))
        val handle = create(config)
        try { assertEquals("connecting", NativeRuntimeConfiguration.command("poll", handle).getString("connectionStatus")) }
        finally { NativeRuntimeConfiguration.command("stop", handle) }

        val provider = ServiceProvider.DASHSCOPE
        assertEquals(listOf("auto"), provider.sourcesForTranslation(text.provider, speech.alibabaRealtimeMode))
        val contract = JSONObject(requireNotNull(javaClass.getResourceAsStream("/translation-contracts.json"))
            .bufferedReader().use { it.readText() }).getJSONObject("alibabaRealtimeLanguages").getJSONArray("languages")
        val targets = (0 until contract.length()).map { contract.getJSONObject(it).getString("code") }.toSet() + "original"
        assertEquals(targets, provider.targetsForTranslation(text.provider, speech.alibabaRealtimeMode).toSet())
        assertEquals("auto" to "fr", provider.normalize("en", "fr", text.provider, speech.alibabaRealtimeMode))
        assertFalse(provider.supportsPair("en", "fr", text.provider, speech.alibabaRealtimeMode))
        assertTrue(provider.supportsPair("auto", "fr", text.provider, speech.alibabaRealtimeMode))
        // A retained ASR-only preference cannot silently disable integrated translation.
        val originalPreference = buildRuntimeConfiguration(speech,
            TranslationConfiguration(provider = TextTranslationProvider.NONE), "auto", "fr", "lite")
        assertEquals("fr", originalPreference.getString("targetLanguage"))
        assertEquals("auto" to "fr", provider.normalize("en", "fr", TextTranslationProvider.NONE, "liveTranslate"))

        for (workspace in listOf("", "bad/workspace", "a".repeat(64))) {
            val invalid = JSONObject(config.toString()).put("alibabaWorkspaceId", workspace)
            assertEquals("invalid_configuration", assertThrows(IllegalArgumentException::class.java) { create(invalid) }.message)
        }
        assertEquals(TextTranslationProvider.DEEPLX, text.provider)
        assertEquals("", text.endpoint)
    }

    @Test fun legacyModeAndReturningToAsrMtKeepIndependentTranslationRouting() {
        val legacy = ServiceConfiguration(ServiceProvider.DASHSCOPE, mapOf("apiKey" to "synthetic-speech-key"))
        assertEquals("asrMt", legacy.alibabaRealtimeMode)
        assertTrue(legacy.usesIndependentTextTranslation)
        val text = TranslationConfiguration(provider = TextTranslationProvider.DEEPLX,
            endpoint = "https://synthetic.example.invalid/translate", apiKey = "synthetic-text-key")
        val config = buildRuntimeConfiguration(legacy, text, "auto", "zh", "lite")
        assertEquals("disabled", config.getString("alibabaRealtimeMode"))
        assertEquals("alibabaCloud", config.getString("provider"))
        assertEquals("apiKey", config.getJSONObject("credentials").getString("kind"))
        assertFalse(config.getJSONObject("credentials").has("asrApiKey"))
        assertEquals("deepLX", config.getJSONObject("textCredentials").getString("kind"))
        val handle = create(config)
        try { assertTrue(NativeRuntimeConfiguration.command("poll", handle).has("snapshot")) }
        finally { NativeRuntimeConfiguration.command("stop", handle) }
        assertTrue(ServiceProvider.DASHSCOPE.sourcesForTranslation(TextTranslationProvider.BUILTIN).size > 1)
    }

    @Test fun selectedQwenModelsCrossActualJniWithoutConnecting() {
        val targets = JSONObject(requireNotNull(NativeRuntimeJniTest::class.java.getResource("/provider-language-catalogs.json")).readText())
            .getJSONObject("qwenMtFlash").getJSONArray("targetLanguages")
        assertEquals(80, targets.length())
        for (model in listOf("flash", "plus")) {
            for (index in 0 until targets.length()) {
                val config = configuration().put("qwenMtModel", model).put("sourceLanguage", "no")
                    .put("targetLanguage", targets.getString(index))
                val handle = create(config)
                try { assertEquals("connecting", NativeRuntimeConfiguration.command("poll", handle).getString("connectionStatus")) }
                finally { NativeRuntimeConfiguration.command("stop", handle) }
            }
        }
        val lite = configuration().put("qwenMtModel", "lite").put("targetLanguage", "no")
        assertEquals("invalid_configuration", assertThrows(IllegalArgumentException::class.java) { create(lite) }.message)
    }

    @Test fun handlesAreDistinctBoundedAndRejectLatePollsAfterStop() {
        val first = create(configuration())
        var second: Long? = null
        try {
            second = create(configuration())
            assertNotEquals(first, second)
            val error = assertThrows(IllegalArgumentException::class.java) { create(configuration()) }
            assertEquals("native_runtime_capacity", error.message)
            assertTrue(NativeRuntimeConfiguration.command("poll", first).has("snapshot"))
            NativeRuntimeConfiguration.command("stop", first)
            assertEquals("native_runtime_stale_handle", assertThrows(IllegalArgumentException::class.java) {
                NativeRuntimeConfiguration.command("poll", first)
            }.message)
            // Idempotent stop cannot disturb the other live owner.
            NativeRuntimeConfiguration.command("stop", first)
            assertTrue(NativeRuntimeConfiguration.command("poll", second).has("snapshot"))
        } finally {
            NativeRuntimeConfiguration.command("stop", first)
            second?.let { NativeRuntimeConfiguration.command("stop", it) }
        }
    }

    @Test fun invalidLanguagesKeysAndTrustFailWithContentFreeLabels() {
        for (invalid in listOf(
            configuration().put("sourceLanguage", "yue"),
            configuration().put("credentials", JSONObject().put("kind", "apiKey").put("apiKey", "")),
            configuration().put("qwenMtModel", "unknown"),
        )) {
            val error = assertThrows(IllegalArgumentException::class.java) { create(invalid) }
            assertEquals("invalid_configuration", error.message)
            assertFalse(error.message!!.contains("synthetic"))
        }
        val invalidNetwork = JSONObject(network.toString()).put("trustRoots", JSONArray())
        assertEquals("native_network_invalid", assertThrows(IllegalArgumentException::class.java) {
            NativeRuntimeConfiguration.exchange(JSONObject().put("operation", "create").put("configuration", configuration()).put("network", invalidNetwork))
        }.message)
    }

    @Test fun independentRoutesAndOriginalUseTheSharedEndpointResolver() {
        val builtIn = NativeRuntimeConfiguration.endpoints(configuration())
        assertEquals("wss://dashscope.aliyuncs.com/api-ws/v1/inference", builtIn.getString("speechEndpoint"))
        assertTrue(builtIn.getString("textEndpoint").contains("/compatible-mode/v1/chat/completions"))
        assertEquals(listOf("qwen-audio-3.0-asr-flash-streaming", "qwen-mt-lite"),
            app.yuxino.mimi.android.runtimeModelNames(configuration()))
        for (model in listOf("lite", "flash", "plus")) {
            assertEquals("qwen-mt-$model", app.yuxino.mimi.android.runtimeModelNames(
                configuration().put("qwenMtModel", model)).last())
        }
        val text = TranslationConfiguration(provider = TextTranslationProvider.OPENAI_COMPATIBLE,
            endpoint = "https://synthetic.example.invalid/v1", model = "synthetic-model", apiKey = "synthetic-text-key")
        val speech = ServiceConfiguration(ServiceProvider.DASHSCOPE, mapOf("apiKey" to "synthetic-speech-key"))
        val independent = buildRuntimeConfiguration(speech, text, "auto", "zh", "plus")
        val endpoints = NativeRuntimeConfiguration.endpoints(independent)
        assertEquals(builtIn.getString("speechEndpoint"), endpoints.getString("speechEndpoint"))
        assertEquals("https://synthetic.example.invalid/v1/chat/completions", endpoints.getString("textEndpoint"))
        assertEquals("synthetic-model", app.yuxino.mimi.android.runtimeModelNames(independent).last())
        val handle = create(independent)
        try { assertFalse(NativeRuntimeConfiguration.command("poll", handle).toString().contains("synthetic-text-key")) }
        finally { NativeRuntimeConfiguration.command("stop", handle) }
        val original = buildRuntimeConfiguration(speech, TranslationConfiguration(provider = TextTranslationProvider.NONE), "nl", "zh", "lite")
        assertTrue(NativeRuntimeConfiguration.endpoints(original).isNull("textEndpoint"))
        assertEquals(listOf("qwen-audio-3.0-asr-flash-streaming"), app.yuxino.mimi.android.runtimeModelNames(original))
        val originalHandle = create(original)
        try { assertTrue(NativeRuntimeConfiguration.command("poll", originalHandle).has("snapshot")) }
        finally { NativeRuntimeConfiguration.command("stop", originalHandle) }
    }

    @Test fun textProbeEndpointUsesSharedCredentialsWithoutASpeechKeyOrRequest() {
        val text = TranslationConfiguration(provider = TextTranslationProvider.CHAT_MOCK,
            endpoint = "https://synthetic.example.invalid/v1", model = "synthetic-model")
        val config = buildTextProbeConfiguration(text, "auto", "zh", "flash")
        val endpoints = NativeRuntimeConfiguration.exchange(JSONObject().put("operation", "text_probe_endpoint").put("configuration", config))
        assertEquals("https://synthetic.example.invalid/v1/chat/completions", endpoints.getString("textEndpoint"))
        assertFalse(endpoints.has("speechEndpoint"))
        assertFalse(endpoints.toString().contains("apiKey"))
    }

    @Test fun baiduFinalAndFollowingDraftKeepSeparateOwnersAcrossActualJni() {
        SubtitleBus.clear()
        SubtitleBus.setHistoryLimit(2)
        val observed = mutableListOf<SubtitleBus.DisplaySnapshot>()
        val listener = object : SubtitleBus.Listener {
            override fun onSubtitleChanged() { observed += SubtitleBus.displaySnapshot() }
        }
        SubtitleBus.addListener(listener)
        try {
            RealtimeLoopback(ServiceProvider.BAIDU).use { fixture ->
                val handle = NativeRuntimeConfiguration.exchange(JSONObject().put("operation", "create")
                    .put("configuration", configuration(ServiceProvider.BAIDU).put("sourceLanguage", "en")).put("network", fixture.network)).getLong("handle")
                fun awaitState(predicate: (JSONObject) -> Boolean): JSONObject {
                    val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(5)
                    while (System.nanoTime() < deadline) {
                        fixture.checkFailure()
                        val state = NativeRuntimeConfiguration.command("poll", handle)
                        if (predicate(state)) {
                            SubtitleBus.onRuntimeSnapshot(state, false)
                            return state
                        }
                        assertTrue("Loopback runtime failed: ${state.optString("errorCode")}", state.isNull("errorCode"))
                        Thread.sleep(10)
                    }
                    error("Baidu loopback state did not arrive")
                }
                try {
                    NativeRuntimeConfiguration.command("start", handle)
                    val first = awaitState { state ->
                        val snapshot = state.getJSONObject("snapshot")
                        snapshot.getJSONArray("history").length() == 1 && snapshot.getJSONObject("source").getString("text") == "Synthetic next draft" &&
                            snapshot.getJSONObject("translation").getString("text") == "合成下一句草稿"
                    }.getJSONObject("snapshot")
                    assertEquals("Synthetic first sentence.", first.getJSONArray("history").getJSONObject(0).getString("source"))
                    assertEquals("合成第一句。", first.getJSONArray("history").getJSONObject(0).getString("translation"))
                    assertEquals("合成下一句草稿", first.getJSONObject("translation").getString("text"))
                    assertFalse(first.getJSONObject("source").getBoolean("isFinal"))
                    assertFalse(first.getJSONObject("translation").getBoolean("isFinal"))
                    assertEquals("Synthetic next draft", first.getJSONObject("realtimePreview").getJSONObject("source").getString("text"))
                    assertEquals("合成下一句草稿", first.getJSONObject("realtimePreview").getJSONObject("translation").getString("text"))
                    assertFalse(observed.last().confirmed)
                    assertFalse(observed.last().currentSource.confirmed)
                    assertEquals("Synthetic first sentence.", observed.last().retainedConfirmed!!.pair.source)
                    fixture.nextMid.countDown()
                    val mid = awaitState {
                        val snapshot = it.getJSONObject("snapshot")
                        snapshot.getJSONObject("source").getString("text") == "Synthetic second sentence." &&
                            snapshot.getJSONObject("translation").getString("text") == "合成第二句预览。"
                    }.getJSONObject("snapshot")
                    assertEquals(1, mid.getJSONArray("history").length())
                    assertEquals("Synthetic first sentence.", mid.getJSONObject("displayPair").getString("source"))
                    assertFalse(observed.last().confirmed)
                    fixture.nextFinal.countDown()
                    val final = awaitState { it.getJSONObject("snapshot").getJSONArray("history").length() == 2 }.getJSONObject("snapshot")
                    val history = final.getJSONArray("history")
                    assertEquals("Synthetic first sentence.", history.getJSONObject(0).getString("source"))
                    assertEquals("合成第一句。", history.getJSONObject(0).getString("translation"))
                    assertEquals("Synthetic second sentence.", history.getJSONObject(1).getString("source"))
                    assertEquals("合成第二句定稿。", history.getJSONObject(1).getString("translation"))
                    assertEquals("system", history.getJSONObject(1).getString("audioSource"))
                    assertTrue(final.getJSONObject("source").getBoolean("isFinal"))
                    assertTrue(final.getJSONObject("translation").getBoolean("isFinal"))
                    assertTrue(final.getBoolean("displayPairFinal"))
                    assertTrue(observed.last().confirmed)
                    assertEquals("合成第二句定稿。", observed.last().translation)
                    NativeRuntimeConfiguration.command("finish", handle)
                    awaitState { it.getBoolean("finished") }
                    fixture.checkFailure()
                } finally { NativeRuntimeConfiguration.command("stop", handle) }
            }
        } finally {
            SubtitleBus.removeListener(listener)
            SubtitleBus.clear()
            SubtitleBus.setHistoryLimit(0)
        }
    }

    @Test fun tencentLateFinalAndReplayPreserveSentenceOwnersAcrossActualJni() {
        SubtitleBus.clear()
        SubtitleBus.setHistoryLimit(3)
        try {
            RealtimeLoopback(ServiceProvider.TENCENT).use { fixture ->
                val handle = NativeRuntimeConfiguration.exchange(JSONObject().put("operation", "create")
                    .put("configuration", configuration(ServiceProvider.TENCENT).put("sourceLanguage", "en"))
                    .put("network", fixture.network)).getLong("handle")
                fun awaitState(predicate: (JSONObject) -> Boolean): JSONObject {
                    val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(5)
                    while (System.nanoTime() < deadline) {
                        fixture.checkFailure()
                        val state = NativeRuntimeConfiguration.command("poll", handle)
                        assertTrue("Loopback runtime failed: ${state.optString("errorCode")}", state.isNull("errorCode"))
                        if (predicate(state)) {
                            SubtitleBus.onRuntimeSnapshot(state, false)
                            return state
                        }
                        Thread.sleep(10)
                    }
                    error("Tencent loopback state did not arrive")
                }
                try {
                    NativeRuntimeConfiguration.command("start", handle)
                    awaitState { it.getJSONObject("snapshot").getJSONObject("source").getString("text") == "Synthetic B draft" }
                    fixture.nextMid.countDown()
                    val late = awaitState { it.getJSONObject("snapshot").getJSONArray("history").length() == 1 }.getJSONObject("snapshot")
                    assertEquals("Synthetic A final", late.getJSONArray("history").getJSONObject(0).getString("source"))
                    val preview = late.getJSONObject("realtimePreview")
                    assertEquals("B", preview.getJSONObject("source").getString("utteranceId"))
                    assertEquals("B", preview.getJSONObject("translation").getString("utteranceId"))
                    assertEquals("Synthetic B draft", preview.getJSONObject("source").getString("text"))
                    assertEquals("Translated Synthetic B draft", preview.getJSONObject("translation").getString("text"))
                    assertEquals("Synthetic B draft", SubtitleBus.displaySnapshot().source)
                    fixture.nextFinal.countDown()
                    val final = awaitState { it.getJSONObject("snapshot").getJSONArray("history").length() == 3 }.getJSONObject("snapshot")
                    val history = final.getJSONArray("history")
                    for (index in 1..2) {
                        assertEquals("Synthetic repeated final", history.getJSONObject(index).getString("source"))
                        assertEquals("Translated Synthetic repeated final", history.getJSONObject(index).getString("translation"))
                        assertEquals("system", history.getJSONObject(index).getString("audioSource"))
                    }
                    assertEquals("C", final.getJSONObject("source").getString("utteranceId"))
                    assertTrue(final.getJSONObject("source").getBoolean("isFinal"))
                    assertTrue(final.getJSONObject("translation").getBoolean("isFinal"))
                    assertTrue(final.isNull("realtimePreview"))
                    assertTrue(SubtitleBus.displaySnapshot().confirmed)
                    NativeRuntimeConfiguration.command("finish", handle)
                    val stopped = awaitState { it.getBoolean("finished") }.getJSONObject("snapshot")
                    assertEquals(3, stopped.getJSONArray("history").length())
                    assertEquals("Synthetic repeated final", stopped.getJSONObject("displayPair").getString("source"))
                    fixture.checkFailure()
                } finally { NativeRuntimeConfiguration.command("stop", handle) }
            }
        } finally { SubtitleBus.clear(); SubtitleBus.setHistoryLimit(0) }
    }

    @Test fun volcanoAudioFramesAndPaddedTailCrossActualJni() {
        RealtimeLoopback(ServiceProvider.VOLCANO).use { fixture ->
            val handle = NativeRuntimeConfiguration.exchange(JSONObject().put("operation", "create")
                .put("configuration", configuration(ServiceProvider.VOLCANO).put("sourceLanguage", "en"))
                .put("network", fixture.network)).getLong("handle")
            try {
                NativeRuntimeConfiguration.command("start", handle)
                awaitRuntimeState(handle, fixture) { it.getString("connectionStatus") == "listening" }
                assertTrue("Volcano rejected synthetic PCM", NativeRuntimeConfiguration.pcmRaw(handle, fixture.volcanoPcm))
                assertTrue("Volcano did not send both complete frames", fixture.volcanoCompleteFrames.await(3, TimeUnit.SECONDS))
                fixture.checkFailure()
                NativeRuntimeConfiguration.command("finish", handle)
                awaitRuntimeState(handle, fixture) { it.getBoolean("finished") }
                assertTrue("Volcano did not send FinishSession", fixture.finishSeen.await(3, TimeUnit.SECONDS))
                assertTrue("Volcano finish kept its transport open", fixture.transportClosed.await(3, TimeUnit.SECONDS))
                fixture.checkFailure()
                assertEquals(1, fixture.connectionCount.get())
            } finally { NativeRuntimeConfiguration.command("stop", handle) }
        }
    }

    @Test fun liveTranslateUnexpectedEndReconnectsButAuthenticationAndManualFinishStopAcrossActualJni() {
        verifyLiveTranslateRecoveryCases("liveTranslateRecovery")
    }

    @Test fun openaiSetupClassificationAndRecoveredPcmCrossActualJni() {
        verifyBuiltinRealtimeRecovery(ServiceProvider.OPENAI)
    }

    @Test fun openaiQuietResumeAndMissingOriginalIsolationCrossActualJni() {
        val case = JSONObject().put("id", "openai-quiet-resume-source-isolation")
            .put("provider", ServiceProvider.OPENAI.id).put("duringSetup", false)
            .put("expectedConnections", 1).put("expectedErrorCode", JSONObject.NULL)
            .put("sourceProgressReplay", true).put("timeoutSeconds", 45)
        verifyBuiltinRealtimeRecovery(ServiceProvider.OPENAI, listOf(case))
    }

    @Test fun openaiLateCloseTailSurvivesBothFinishDeadlinesAcrossActualJni() {
        val case = JSONObject().put("id", "openai-late-close-tail")
            .put("provider", ServiceProvider.OPENAI.id).put("duringSetup", false)
            .put("expectedConnections", 2).put("expectedErrorCode", JSONObject.NULL)
            .put("failureFrame", JSONObject.NULL).put("delayedCloseMs", 7_000)
            .put("timeoutSeconds", 12)
        verifyBuiltinRealtimeRecovery(ServiceProvider.OPENAI, listOf(case))
    }

    @Test fun grokVoiceSetupClassificationAndRecoveredPcmCrossActualJni() {
        verifyBuiltinRealtimeRecovery(ServiceProvider.XAI)
    }

    @Test fun geminiSetupClassificationAndUnavailableRecoveryCrossActualJni() {
        verifyBuiltinRealtimeRecovery(ServiceProvider.GEMINI)
    }

    @Test fun geminiTranscribeBinarySetupAndFinalCrossActualJni() {
        val wire = JSONObject(requireNotNull(javaClass.getResourceAsStream("/translation-contracts.json"))
            .bufferedReader().use { it.readText() }).getJSONObject("geminiTranscribeTransport")
        RealtimeLoopback(ServiceProvider.GEMINI, geminiTranscribe = wire).use { fixture ->
            val config = configuration(ServiceProvider.GEMINI).put("speechTranslationMode", "asrMt")
                .put("sourceLanguage", "en").put("targetLanguage", "original")
            val handle = NativeRuntimeConfiguration.exchange(JSONObject().put("operation", "create")
                .put("configuration", config).put("network", fixture.network)).getLong("handle")
            try {
                NativeRuntimeConfiguration.command("start", handle)
                awaitRuntimeState(handle, fixture) { it.getString("connectionStatus") == "listening" }
                fixture.nextMid.countDown()
                awaitRuntimeState(handle, fixture) {
                    it.getJSONObject("snapshot").getJSONObject("source").optString("text") == wire.getString("expectedDraft")
                }
                fixture.nextFinal.countDown()
                awaitRuntimeState(handle, fixture) {
                    it.getJSONObject("snapshot").getJSONObject("source").optString("text") == wire.getString("expectedFinal")
                }
                stopAndAwaitTransportClosed(handle, fixture)
                assertEquals(1, fixture.connectionCount.get())
            } finally { NativeRuntimeConfiguration.command("stop", handle) }
        }
    }

    @Test fun audio3SetupRejectionAndUnsolicitedFinishRecoveryCrossActualJni() {
        verifyBuiltinRealtimeRecovery(ServiceProvider.DASHSCOPE)
    }

    @Test fun baiduSetupRejectionsAndTransportRecoveryCrossActualJni() {
        verifyBuiltinRealtimeRecovery(ServiceProvider.BAIDU)
    }

    @Test fun baiduSilenceRejectionRecoversAndResumesPcmAcrossActualJni() {
        val case = JSONObject().put("id", "baidu-silent-audio-rejection")
            .put("provider", ServiceProvider.BAIDU.id).put("duringSetup", false)
            .put("expectedConnections", 2).put("expectedErrorCode", JSONObject.NULL)
            .put("failureFrame", JSONObject().put("code", 20316))
            .put("quietAudioFailure", true).put("timeoutSeconds", 12)
        val late = JSONObject(case.toString()).put("id", "baidu-late-silence-rejection")
            .put("lateQuietAudioFailure", true)
        verifyBuiltinRealtimeRecovery(ServiceProvider.BAIDU, listOf(case, late))
    }

    private fun verifyBuiltinRealtimeRecovery(provider: ServiceProvider, selected: List<JSONObject>? = null) {
        val cases = JSONObject(requireNotNull(javaClass.getResourceAsStream("/translation-contracts.json"))
            .bufferedReader().use { it.readText() }).getJSONArray("builtinRealtimeRecovery")
        val matching = selected ?: (0 until cases.length()).map(cases::getJSONObject).filter { it.getString("provider") == provider.id }
        assertTrue("Missing builtin recovery contracts for $provider", matching.isNotEmpty())
        for (case in matching) {
            val id = case.getString("id")
            RealtimeLoopback(provider, builtinRecovery = case).use { fixture ->
                val config = configuration(provider)
                    .put("sourceLanguage", if (provider == ServiceProvider.XAI || provider == ServiceProvider.DASHSCOPE || provider == ServiceProvider.BAIDU) "en" else "auto")
                    .put("targetLanguage", if (provider == ServiceProvider.DASHSCOPE) "original" else "zh")
                val handle = NativeRuntimeConfiguration.exchange(JSONObject().put("operation", "create")
                    .put("configuration", config).put("network", fixture.network)).getLong("handle")
                try {
                    NativeRuntimeConfiguration.command("start", handle)
                    if (!case.getBoolean("duringSetup") && !case.optBoolean("repeatFailures")) {
                        awaitRuntimeState(handle, fixture) { it.getString("connectionStatus") == "listening" }
                        fixture.nextMid.countDown()
                        if (case.optBoolean("quietAudioFailure")) {
                            repeat(550) {
                                assertTrue("$id rejected quiet PCM", NativeRuntimeConfiguration.pcmRaw(handle, ByteArray(1_280)))
                                Thread.sleep(40)
                            }
                            if (case.optBoolean("lateQuietAudioFailure")) {
                                assertTrue("$id rejected resumed PCM", NativeRuntimeConfiguration.pcmRaw(handle, ByteArray(1_280) { 7 }))
                            }
                        }
                    }
                    if (case.isNull("expectedErrorCode")) {
                        var resumedAfterQuiet = false
                        var isolatedSource = false
                        val replay = case.optBoolean("sourceProgressReplay")
                        awaitRuntimeState(handle, fixture, timeoutSeconds = case.optLong("timeoutSeconds", 5)) {
                            assertFalse("$id unexpectedly stopped", it.getBoolean("finished"))
                            val snapshot = it.getJSONObject("snapshot")
                            val original = snapshot.getJSONObject("source").optString("text")
                            if (original == "Original after waiting.") resumedAfterQuiet = true
                            if (original.isEmpty() && snapshot.getJSONObject("translation").optString("text").contains("合成的持续译文片段")) isolatedSource = true
                            if (replay) original == "Fresh original." && snapshot.getJSONObject("translation").optString("text") == "新的译文。"
                            else fixture.reconnected.count == 0L && it.getString("connectionStatus") == "listening"
                        }
                        if (replay) {
                            assertTrue("Quiet session did not resume its original", resumedAfterQuiet)
                            assertTrue("Missing original was still shown beside unrelated translation", isolatedSource)
                            assertTrue("Health checks did not run during the replay", fixture.progressPongs.get() >= 3)
                            stopAndAwaitTransportClosed(handle, fixture, fixture.transportClosed)
                        } else {
                            val frameBytes = when (provider) {
                                ServiceProvider.OPENAI, ServiceProvider.XAI -> 9_600
                                ServiceProvider.BAIDU -> 1_280
                                else -> 3_200
                            }
                            repeat(5) { assertTrue("$id rejected recovered PCM", NativeRuntimeConfiguration.pcmRaw(handle, ByteArray(frameBytes) { 7 })) }
                            val receivedAudio = fixture.recoveredAudio.await(3, TimeUnit.SECONDS)
                            fixture.checkFailure()
                            assertTrue("$id did not send recovered PCM", receivedAudio)
                            if (provider == ServiceProvider.XAI || provider == ServiceProvider.DASHSCOPE) {
                                val recovered = awaitRuntimeState(handle, fixture) {
                                    val snapshot = it.getJSONObject("snapshot")
                                    val source = snapshot.getJSONObject("source")
                                    source.optString("text") == "Synthetic recovered caption." && source.getBoolean("isFinal") &&
                                        (provider != ServiceProvider.XAI || snapshot.getJSONObject("translation").let { translation ->
                                            translation.optString("text") == "合成恢复字幕。" && translation.getBoolean("isFinal")
                                        })
                                }.getJSONObject("snapshot")
                                if (provider == ServiceProvider.XAI) {
                                    assertEquals(id, "合成恢复字幕。", recovered.getJSONObject("translation").getString("text"))
                                    assertTrue(id, recovered.getJSONObject("translation").getBoolean("isFinal"))
                                    // Grok normal finish is covered by client regressions; here
                                    // prove hard Stop cannot reconnect the recovered engine.
                                    stopAndAwaitTransportClosed(handle, fixture, fixture.recoveredTransportClosed)
                                } else {
                                    NativeRuntimeConfiguration.command("finish", handle)
                                    awaitRuntimeState(handle, fixture) { it.getBoolean("finished") }
                                }
                            } else {
                                NativeRuntimeConfiguration.command("finish", handle)
                                val finishedState = awaitRuntimeState(handle, fixture, timeoutSeconds = case.optLong("timeoutSeconds", 5)) { it.getBoolean("finished") }
                                assertTrue(id, finishedState.isNull("errorCode"))
                                val finished = finishedState.getJSONObject("snapshot")
                                assertEquals(id, "Synthetic recovered caption.", finished.getJSONObject("source").getString("text"))
                                assertTrue(id, finished.getJSONObject("source").getBoolean("isFinal"))
                                assertEquals(id, "合成恢复字幕。", finished.getJSONObject("translation").getString("text"))
                                assertTrue(id, finished.getJSONObject("translation").getBoolean("isFinal"))
                            }
                        }
                    } else {
                        val failed = awaitRuntimeState(handle, fixture, expectedError = case.getString("expectedErrorCode"),
                            timeoutSeconds = case.optLong("timeoutSeconds", 5)) { it.getBoolean("finished") }
                        assertEquals(id, case.getString("expectedErrorCode"), failed.getString("errorCode"))
                        assertFalse(id, failed.toString().contains("synthetic-private-detail"))
                    }
                    assertTrue("$id opened another connection", fixture.noFurtherConnections.await(2, TimeUnit.SECONDS))
                    fixture.checkFailure()
                    assertEquals(id, case.getInt("expectedConnections"), fixture.connectionCount.get())
                } finally { NativeRuntimeConfiguration.command("stop", handle) }
            }
        }
    }

    @Test fun liveTranslateSetupRejectionsPreserveRecoveryClassificationAcrossActualJni() {
        verifyLiveTranslateRecoveryCases("liveTranslateSetupRecovery")
    }

    @Test fun liveTranslateRecoveryBudgetSurvivesSetupSuccessUntilStableHealthAcrossActualJni() {
        verifyLiveTranslateRecoveryCases("liveTranslateRecoveryContinuity")
    }

    @Test fun liveTranslateRecoverySetupAuthenticationKeepsLatestFailureAcrossActualJni() {
        verifyLiveTranslateRecoveryCases(JSONArray().put(JSONObject("""{
            "id":"recovery-setup-authentication", "expectedConnections":2,
            "failureFrame":{"type":"error","error":{"code":"response_idle_timeout","message":"synthetic-private-detail"}},
            "retrySetupFailureFrame":{"type":"error","error":{"code":"unauthorized","message":"synthetic-private-detail"}},
            "expectedErrorCode":"credential_authentication_failed"
        }""")))
    }

    private fun verifyLiveTranslateRecoveryCases(contract: String) {
        val cases = JSONObject(requireNotNull(javaClass.getResourceAsStream("/translation-contracts.json"))
            .bufferedReader().use { it.readText() }).getJSONArray(contract)
        verifyLiveTranslateRecoveryCases(cases)
    }

    private fun verifyLiveTranslateRecoveryCases(cases: JSONArray) {
        for (index in 0 until cases.length()) {
            val case = cases.getJSONObject(index)
            val id = case.getString("id")
            RealtimeLoopback(ServiceProvider.DASHSCOPE, liveTranslate = true, liveRecovery = case).use { fixture ->
                val handle = createLiveTranslate(fixture)
                try {
                    NativeRuntimeConfiguration.command("start", handle)
                    if (!case.optBoolean("duringSetup") && !case.optBoolean("repeatFailures")) {
                        awaitRuntimeState(handle, fixture) { it.getString("connectionStatus") == "listening" }
                        fixture.nextMid.countDown()
                    }
                    if (case.isNull("expectedErrorCode")) {
                        awaitRuntimeState(handle, fixture) {
                            assertFalse("$id stopped instead of reconnecting", it.getBoolean("finished"))
                            fixture.reconnected.count == 0L && it.getString("connectionStatus") == "listening"
                        }
                        repeat(5) { assertTrue(NativeRuntimeConfiguration.pcmRaw(handle, ByteArray(3_200) { 7 })) }
                        assertTrue("Recovered audio did not arrive", fixture.recoveredAudio.await(3, TimeUnit.SECONDS))
                        awaitRuntimeState(handle, fixture) {
                            it.getJSONObject("snapshot").optJSONObject("displayPair")?.optString("translation") == "合成恢复字幕。"
                        }
                        NativeRuntimeConfiguration.command("finish", handle)
                        awaitRuntimeState(handle, fixture) { it.getBoolean("finished") }
                    } else {
                        val failed = awaitRuntimeState(handle, fixture, expectedError = case.getString("expectedErrorCode"),
                            timeoutSeconds = case.optLong("timeoutSeconds", 5)) { it.getBoolean("finished") }
                        assertEquals(id, case.getString("expectedErrorCode"), failed.getString("errorCode"))
                        assertFalse(failed.toString().contains("synthetic-private-detail"))
                    }
                    assertTrue("Manual finish opened another connection", fixture.noFurtherConnections.await(2, TimeUnit.SECONDS))
                    fixture.checkFailure()
                    assertEquals(id, case.getInt("expectedConnections"), fixture.connectionCount.get())
                } finally { NativeRuntimeConfiguration.command("stop", handle) }
            }
        }
    }

    @Test fun pcmBackpressureDuringSetupReconnectsTheSameHandleAndResumesAudioAcrossActualJni() {
        RealtimeLoopback(ServiceProvider.DASHSCOPE, liveTranslate = true, recoverOverflow = true).use { fixture ->
            val handle = createLiveTranslate(fixture)
            try {
                NativeRuntimeConfiguration.command("start", handle)
                assertTrue("Provider did not receive session.update", fixture.setupSeen.await(5, TimeUnit.SECONDS))
                fixture.checkFailure()
                val pcm = ByteArray(3_200)
                val accepted = (0 until 22).count { NativeRuntimeConfiguration.pcmRaw(handle, pcm) }
                assertTrue("Setup did not fill the bounded PCM queue", accepted in 20..21)
                fixture.releaseFirstSetup.countDown()

                assertTrue("PCM backpressure did not reconnect", fixture.reconnected.await(5, TimeUnit.SECONDS))
                awaitRuntimeState(handle, fixture) { it.getString("connectionStatus") == "listening" }
                val recoveredPcm = ByteArray(3_200) { 7 }
                repeat(5) {
                    assertTrue("Reconnected handle still rejects PCM", NativeRuntimeConfiguration.pcmRaw(handle, recoveredPcm))
                    Thread.sleep(100)
                }
                assertTrue("Recovered PCM did not reach the provider", fixture.recoveredAudio.await(3, TimeUnit.SECONDS))
                assertEquals(5, fixture.recoveredAudioFrames.get())
                assertEquals(2, fixture.connectionCount.get())
                val listening = NativeRuntimeConfiguration.command("poll", handle)
                assertEquals("listening", listening.getString("connectionStatus"))
                assertTrue(listening.isNull("errorCode"))
                assertFalse(listening.getBoolean("finished"))

                assertEquals(1L, fixture.recoveredTransportClosed.count)
                stopAndAwaitTransportClosed(handle, fixture, fixture.recoveredTransportClosed)
                assertEquals("native_runtime_stale_handle", assertThrows(IllegalArgumentException::class.java) {
                    NativeRuntimeConfiguration.pcmRaw(handle, pcm)
                }.message)
                val noFurtherConnections = fixture.noFurtherConnections.await(2, TimeUnit.SECONDS)
                fixture.checkFailure()
                assertTrue("Hard stop opened another provider connection", noFurtherConnections)
                assertEquals(2, fixture.connectionCount.get())
            } finally { NativeRuntimeConfiguration.command("stop", handle) }
        }
    }

    @Test fun stopDuringProviderFinishClosesTransportAndPreservesOtherHandleAcrossActualJni() {
        RealtimeLoopback(ServiceProvider.DASHSCOPE, liveTranslate = true).use { fixture ->
            val handle = createLiveTranslate(fixture)
            var second: Long? = null
            try {
                second = create(configuration())
                val otherState = NativeRuntimeConfiguration.command("poll", second).toString()
                NativeRuntimeConfiguration.command("start", handle)
                awaitRuntimeState(handle, fixture) { it.getString("connectionStatus") == "listening" }
                NativeRuntimeConfiguration.command("finish", handle)
                assertTrue("Provider did not receive session.finish", fixture.finishSeen.await(5, TimeUnit.SECONDS))
                fixture.checkFailure()

                stopAndAwaitTransportClosed(handle, fixture)
                assertEquals("native_runtime_stale_handle", assertThrows(IllegalArgumentException::class.java) {
                    NativeRuntimeConfiguration.command("poll", handle)
                }.message)
                NativeRuntimeConfiguration.command("stop", handle)
                assertEquals(otherState, NativeRuntimeConfiguration.command("poll", second).toString())
            } finally {
                NativeRuntimeConfiguration.command("stop", handle)
                second?.let { NativeRuntimeConfiguration.command("stop", it) }
            }
        }
    }

    @Test fun stopWhileProviderSetupIsPendingClosesTransportAcrossActualJni() {
        RealtimeLoopback(ServiceProvider.DASHSCOPE, liveTranslate = true, holdSetup = true).use { fixture ->
            val handle = createLiveTranslate(fixture)
            try {
                NativeRuntimeConfiguration.command("start", handle)
                assertTrue("Provider did not receive session.update", fixture.setupSeen.await(5, TimeUnit.SECONDS))
                fixture.checkFailure()
                assertEquals("connecting", NativeRuntimeConfiguration.command("poll", handle).getString("connectionStatus"))

                stopAndAwaitTransportClosed(handle, fixture)
                assertEquals(1L, fixture.finishSeen.count)
                assertEquals("native_runtime_stale_handle", assertThrows(IllegalArgumentException::class.java) {
                    NativeRuntimeConfiguration.command("poll", handle)
                }.message)
            } finally { NativeRuntimeConfiguration.command("stop", handle) }
        }
    }

    @Test fun directStopClosesLiveTranslateTransportAcrossActualJni() {
        RealtimeLoopback(ServiceProvider.DASHSCOPE, liveTranslate = true).use { fixture ->
            val handle = createLiveTranslate(fixture)
            try {
                NativeRuntimeConfiguration.command("start", handle)
                awaitRuntimeState(handle, fixture) { it.getString("connectionStatus") == "listening" }
                stopAndAwaitTransportClosed(handle, fixture)
                assertEquals(1L, fixture.finishSeen.count)
            } finally { NativeRuntimeConfiguration.command("stop", handle) }
        }
    }

    @Test fun gracefulLiveTranslateFinishRetainsProviderTailAcrossActualJni() {
        RealtimeLoopback(ServiceProvider.DASHSCOPE, liveTranslate = true, finishTail = true).use { fixture ->
            val handle = createLiveTranslate(fixture)
            try {
                NativeRuntimeConfiguration.command("start", handle)
                awaitRuntimeState(handle, fixture) { it.getString("connectionStatus") == "listening" }
                NativeRuntimeConfiguration.command("finish", handle)
                val finished = awaitRuntimeState(handle, fixture) { it.getBoolean("finished") }
                assertEquals(0L, fixture.finishSeen.count)
                val history = finished.getJSONObject("snapshot").getJSONArray("history")
                assertEquals(1, history.length())
                assertEquals("Synthetic finish tail.", history.getJSONObject(0).getString("source"))
                assertEquals("合成收尾字幕。", history.getJSONObject(0).getString("translation"))
                assertEquals("system", history.getJSONObject(0).getString("audioSource"))
                val light = NativeRuntimeConfiguration.poll(handle, finished.getLong("version"))
                assertTrue(light.getBoolean("unchanged"))
                assertTrue(light.getBoolean("finished"))
                assertTrue(light.isNull("errorCode"))
                assertFalse(light.has("snapshot"))
                assertTrue("Graceful finish left its transport open", fixture.transportClosed.await(3, TimeUnit.SECONDS))
                fixture.checkFailure()
            } finally { NativeRuntimeConfiguration.command("stop", handle) }
        }
    }

    @Test fun savingIsOptInAndOptOutRejectsLateFinalsAcrossActualJni() {
        val root = Files.createTempDirectory("mimi-history-optout-jni-").toFile()
        val directory = java.io.File(root, "history")
        fun request(operation: String) = JSONObject().put("operation", operation).put("directory", directory.absolutePath)
        try {
            assertEquals(0, NativeRuntimeConfiguration.exchange(request("history_list")).getJSONArray("items").length())
            val unsaved = create(configuration())
            NativeRuntimeConfiguration.command("stop", unsaved)
            assertFalse(directory.exists())
            val journal = NativeRuntimeConfiguration.exchange(request("history_begin")).getLong("historyHandle")
            RealtimeLoopback(ServiceProvider.DASHSCOPE, liveTranslate = true, finishTail = true).use { fixture ->
                val handle = NativeRuntimeConfiguration.exchange(JSONObject().put("operation", "create")
                    .put("configuration", configuration().put("alibabaRealtimeMode", "liveTranslate")
                        .put("alibabaWorkspaceId", "synthetic-workspace").put("sourceLanguage", "auto"))
                    .put("network", fixture.network).put("audioSource", "microphone").put("historyHandle", journal)).getLong("handle")
                try {
                    NativeRuntimeConfiguration.command("start", handle)
                    awaitRuntimeState(handle, fixture) { it.getString("connectionStatus") == "listening" }
                    NativeRuntimeConfiguration.exchange(request("history_disable").put("historyHandle", journal))
                    NativeRuntimeConfiguration.command("finish", handle)
                    val finished = awaitRuntimeState(handle, fixture) { it.getBoolean("finished") }
                    assertEquals(1, finished.getJSONObject("snapshot").getJSONArray("history").length())
                    val status = NativeRuntimeConfiguration.exchange(request("history_state").put("historyHandle", journal))
                    assertTrue(status.getBoolean("finalized"))
                    assertFalse(status.getBoolean("pending"))
                    assertEquals(0, status.getJSONObject("state").getInt("count"))
                    assertEquals(0, NativeRuntimeConfiguration.exchange(request("history_list")).getJSONArray("items").length())
                } finally { NativeRuntimeConfiguration.command("stop", handle) }
            }
        } finally { root.deleteRecursively() }
    }

    @Test fun savedHistoryKeepsBothSourcesAndUnpolledFinalsAcrossActualJni() {
        val root = Files.createTempDirectory("mimi-history-jni-").toFile()
        val historyDirectory = java.io.File(root, "history")
        fun request(operation: String) = JSONObject().put("operation", operation).put("directory", historyDirectory.absolutePath)
        val journal = NativeRuntimeConfiguration.exchange(request("history_begin")).getLong("historyHandle")
        val handles = mutableListOf<Long>()
        try {
            RealtimeLoopback(ServiceProvider.DASHSCOPE, liveTranslate = true, finishTail = true, finishTailCount = 25).use { system ->
                RealtimeLoopback(ServiceProvider.DASHSCOPE, liveTranslate = true, finishTail = true).use { microphone ->
                    fun createSource(source: String, fixture: RealtimeLoopback): Long = NativeRuntimeConfiguration.exchange(
                        JSONObject().put("operation", "create").put("configuration", configuration()
                            .put("alibabaRealtimeMode", "liveTranslate").put("alibabaWorkspaceId", "synthetic-workspace")
                            .put("sourceLanguage", "auto")).put("network", fixture.network)
                            .put("audioSource", source).put("historyHandle", journal)).getLong("handle").also(handles::add)
                    val systemHandle = createSource("system", system)
                    val microphoneHandle = createSource("microphone", microphone)
                    NativeRuntimeConfiguration.command("start", systemHandle)
                    NativeRuntimeConfiguration.command("start", microphoneHandle)
                    awaitRuntimeState(systemHandle, system) { it.getString("connectionStatus") == "listening" }
                    awaitRuntimeState(microphoneHandle, microphone) { it.getString("connectionStatus") == "listening" }
                    NativeRuntimeConfiguration.exchange(request("history_finish").put("historyHandle", journal))
                    NativeRuntimeConfiguration.command("finish", systemHandle)
                    val finishedSystem = awaitRuntimeState(systemHandle, system) { it.getBoolean("finished") }
                    assertEquals(20, finishedSystem.getJSONObject("snapshot").getJSONArray("history").length())
                    assertEquals(0, NativeRuntimeConfiguration.exchange(request("history_list")).getJSONArray("items").length())
                    NativeRuntimeConfiguration.command("finish", microphoneHandle)
                    awaitRuntimeState(microphoneHandle, microphone) { it.getBoolean("finished") }
                    val items = NativeRuntimeConfiguration.exchange(request("history_list")).getJSONArray("items")
                    assertEquals(1, items.length())
                    val item = items.getJSONObject(0)
                    assertEquals(26, item.getInt("count"))
                    assertFalse(item.getBoolean("hasAudio"))
                    val id = item.getString("id")
                    val page = NativeRuntimeConfiguration.exchange(request("history_page").put("id", id).put("page", 0))
                    assertEquals(26, page.getInt("total"))
                    val rows = page.getJSONArray("entries")
                    assertEquals("microphone", rows.getJSONObject(0).getString("audioSource"))
                    assertEquals("system", rows.getJSONObject(1).getString("audioSource"))
                    for ((format, suffix) in listOf("txt" to ".txt", "markdown" to ".md")) {
                        val exported = NativeRuntimeConfiguration.exchange(request("history_export").put("id", id)
                            .put("exportDirectory", java.io.File(root, "exports").absolutePath)
                            .put("options", JSONObject().put("format", format)))
                        val file = java.io.File(exported.getString("path"))
                        assertTrue(file.name.endsWith(suffix))
                        val text = file.readText()
                        assertTrue(text.contains("Synthetic finish tail 0"))
                        assertTrue(text.contains("Synthetic finish tail 24"))
                        assertTrue(text.contains("Microphone"))
                        assertTrue(text.contains("System"))
                    }
                    NativeRuntimeConfiguration.exchange(request("history_delete").put("id", id))
                    assertEquals(0, NativeRuntimeConfiguration.exchange(request("history_list")).getJSONArray("items").length())
                }
            }
        } finally {
            handles.forEach { NativeRuntimeConfiguration.command("stop", it) }
            NativeRuntimeConfiguration.exchange(request("history_finish").put("historyHandle", journal))
            root.deleteRecursively()
        }
    }

    private fun createLiveTranslate(fixture: RealtimeLoopback): Long = NativeRuntimeConfiguration.exchange(
        JSONObject().put("operation", "create").put("configuration", configuration()
            .put("alibabaRealtimeMode", "liveTranslate").put("alibabaWorkspaceId", "synthetic-workspace")
            .put("sourceLanguage", "auto")).put("network", fixture.network)).getLong("handle")

    private fun awaitRuntimeState(handle: Long, fixture: RealtimeLoopback, expectedError: String? = null, timeoutSeconds: Long = 5,
        predicate: (JSONObject) -> Boolean): JSONObject {
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(timeoutSeconds)
        while (System.nanoTime() < deadline) {
            fixture.checkFailure()
            val state = NativeRuntimeConfiguration.command("poll", handle)
            assertTrue("Loopback runtime failed: ${state.optString("errorCode")}",
                state.isNull("errorCode") || (expectedError != null && state.optString("errorCode") == expectedError))
            if (predicate(state)) return state
            Thread.sleep(10)
        }
        error("LiveTranslate loopback state did not arrive")
    }

    private fun stopAndAwaitTransportClosed(handle: Long, fixture: RealtimeLoopback,
        transportClosed: CountDownLatch = fixture.transportClosed) {
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(3)
        NativeRuntimeConfiguration.command("stop", handle)
        val remaining = deadline - System.nanoTime()
        assertTrue("Native stop left provider transport open beyond three seconds",
            remaining > 0 && transportClosed.await(remaining, TimeUnit.NANOSECONDS))
        fixture.checkFailure()
    }

    @Test fun liveTranslateLateFinalKeepsTheFollowingSourceAcrossActualJni() {
        SubtitleBus.clear()
        SubtitleBus.setHistoryLimit(2)
        try {
            RealtimeLoopback(ServiceProvider.DASHSCOPE, liveTranslate = true, lateFinalReplay = true).use { fixture ->
                val handle = createLiveTranslate(fixture)
                fun awaitState(predicate: (JSONObject) -> Boolean): JSONObject {
                    val state = awaitRuntimeState(handle, fixture, predicate = predicate)
                    SubtitleBus.onRuntimeSnapshot(state, false)
                    return state
                }
                fun assertFollowingSource(snapshot: JSONObject) {
                    val source = snapshot.getJSONObject("source")
                    assertEquals("source_a", source.getString("utteranceId"))
                    assertEquals("Synthetic second draft", source.getString("text"))
                    assertFalse(source.getBoolean("isFinal"))
                    assertEquals("Synthetic second draft", SubtitleBus.displaySnapshot().currentSource.text)
                    assertFalse(SubtitleBus.displaySnapshot().currentSource.confirmed)
                }
                try {
                    NativeRuntimeConfiguration.command("start", handle)
                    // This exercises the actual protocol mapper and JNI publication.
                    // Rust channel tests hold the receiver to cover mailbox backlog
                    // deterministically; this bridge exposes no consumer pause gate.
                    val draft = awaitState {
                        it.getJSONObject("snapshot").getJSONObject("source").optString("utteranceId") == "source_a"
                    }.getJSONObject("snapshot")
                    assertFollowingSource(draft)
                    assertEquals(0, draft.getJSONArray("history").length())
                    fixture.nextMid.countDown()
                    val late = awaitState {
                        it.getJSONObject("snapshot").getJSONArray("history").length() == 1
                    }.getJSONObject("snapshot")
                    assertFollowingSource(late)
                    val confirmed = late.getJSONArray("history").getJSONObject(0)
                    assertEquals("Synthetic first sentence.", confirmed.getString("source"))
                    assertEquals("合成第一句。", confirmed.getString("translation"))
                    assertEquals("system", confirmed.getString("audioSource"))
                    assertEquals("Synthetic first sentence.", SubtitleBus.displaySnapshot().retainedConfirmed!!.pair.source)
                    fixture.nextFinal.countDown()
                    val replayed = awaitState {
                        it.getJSONObject("snapshot").getJSONObject("translation").getString("text") == "合成第二句预览"
                    }.getJSONObject("snapshot")
                    assertFollowingSource(replayed)
                    assertEquals(1, replayed.getJSONArray("history").length())
                    assertEquals("source_a", replayed.getJSONObject("translation").getString("utteranceId"))
                    assertFalse(replayed.getJSONObject("translation").getBoolean("isFinal"))
                    assertEquals("Synthetic first sentence.", replayed.getJSONObject("displayPair").getString("source"))
                    assertEquals("Synthetic second draft", SubtitleBus.displaySnapshot().source)
                    assertEquals("合成第二句预览", SubtitleBus.displaySnapshot().translation)
                    NativeRuntimeConfiguration.command("finish", handle)
                    val stopped = awaitState { it.getBoolean("finished") }.getJSONObject("snapshot")
                    assertFollowingSource(stopped)
                    assertEquals(1, stopped.getJSONArray("history").length())
                    fixture.checkFailure()
                } finally { NativeRuntimeConfiguration.command("stop", handle) }
            }
        } finally { SubtitleBus.clear(); SubtitleBus.setHistoryLimit(0) }
    }

    @Test fun liveTranslateSourceOrderAndCompletedHistorySurviveNormalAndFinishDelivery() {
        SubtitleBus.clear()
        SubtitleBus.setHistoryLimit(3)
        try {
            RealtimeLoopback(ServiceProvider.DASHSCOPE, liveTranslate = true, sourceOrderReplay = true).use { fixture ->
                val handle = createLiveTranslate(fixture)
                fun awaitState(predicate: (JSONObject) -> Boolean): JSONObject {
                    val state = awaitRuntimeState(handle, fixture, predicate = predicate)
                    SubtitleBus.onRuntimeSnapshot(state, false)
                    return state
                }
                fun assertCurrent(snapshot: JSONObject) {
                    assertEquals("source_a", snapshot.getJSONObject("source").getString("utteranceId"))
                    assertEquals("Synthetic current final.", snapshot.getJSONObject("source").getString("text"))
                    assertTrue(snapshot.getJSONObject("source").getBoolean("isFinal"))
                    assertEquals("Synthetic current final.", SubtitleBus.displaySnapshot().currentSource.text)
                }
                try {
                    NativeRuntimeConfiguration.command("start", handle)
                    awaitState { it.getJSONObject("snapshot").getJSONObject("source").optString("utteranceId") == "source_a" }
                    fixture.nextMid.countDown()
                    val previous = awaitState { it.getJSONObject("snapshot").getJSONArray("history").length() == 1 }.getJSONObject("snapshot")
                    assertCurrent(previous)
                    assertEquals("Synthetic late old final.", previous.getJSONArray("history").getJSONObject(0).getString("source"))
                    assertEquals("Synthetic previous translation.", previous.getJSONObject("displayPair").getString("translation"))
                    fixture.nextFinal.countDown()
                    val preview = awaitState { it.getJSONObject("snapshot").getJSONObject("translation").getString("text") == "Synthetic current preview" }.getJSONObject("snapshot")
                    assertCurrent(preview)
                    assertFalse(preview.getJSONObject("translation").getBoolean("isFinal"))
                    NativeRuntimeConfiguration.command("finish", handle)
                    val finished = awaitState { it.getBoolean("finished") }.getJSONObject("snapshot")
                    assertCurrent(finished)
                    assertEquals(2, finished.getJSONArray("history").length())
                    assertEquals("Synthetic current translation.", finished.getJSONObject("displayPair").getString("translation"))
                    assertTrue(finished.getBoolean("displayPairFinal"))
                    assertEquals("Synthetic late old final.", finished.getJSONArray("history").getJSONObject(0).getString("source"))
                    assertEquals("Synthetic current final.", finished.getJSONArray("history").getJSONObject(1).getString("source"))
                    fixture.checkFailure()
                } finally { NativeRuntimeConfiguration.command("stop", handle) }
            }
        } finally { SubtitleBus.clear(); SubtitleBus.setHistoryLimit(0) }
    }

    @Test fun identifiedSourceActivityCannotRestartAfterTheCurrentPairCompleted() {
        RealtimeLoopback(ServiceProvider.DASHSCOPE, liveTranslate = true, translationStartReplay = true).use { fixture ->
            val handle = createLiveTranslate(fixture)
            try {
                NativeRuntimeConfiguration.command("start", handle)
                val completed = awaitRuntimeState(handle, fixture) { it.getJSONObject("snapshot").getJSONArray("history").length() == 1 }
                assertFalse(completed.getBoolean("isTranslationPending"))
                fixture.nextMid.countDown()
                val old = awaitRuntimeState(handle, fixture) { it.getJSONObject("snapshot").getJSONArray("history").length() == 2 }
                assertFalse(old.getBoolean("isTranslationPending"))
                assertFalse(old.getBoolean("isTranslationTimedOut"))
                assertEquals("source_current", old.getJSONObject("snapshot").getJSONObject("source").getString("utteranceId"))
                assertEquals("Synthetic current translation.", old.getJSONObject("snapshot").getJSONObject("displayPair").getString("translation"))
                fixture.nextFinal.countDown()
                awaitRuntimeState(handle, fixture) {
                    it.getBoolean("isTranslationPending") && it.getJSONObject("snapshot").getJSONObject("source").optString("utteranceId") == "source_next"
                }
                NativeRuntimeConfiguration.command("finish", handle)
                val finished = awaitRuntimeState(handle, fixture) { it.getBoolean("finished") }
                assertFalse(finished.getBoolean("isTranslationPending"))
                assertEquals(3, finished.getJSONObject("snapshot").getJSONArray("history").length())
                assertEquals("Synthetic next translation.", finished.getJSONObject("snapshot").getJSONObject("displayPair").getString("translation"))
                fixture.checkFailure()
            } finally { NativeRuntimeConfiguration.command("stop", handle) }
        }
    }

    /** CONNECT/TLS/WebSocket fixtures intercept the unchanged production endpoints on loopback. */
    private class RealtimeLoopback(private val provider: ServiceProvider, private val qwenResponse: JSONObject? = null,
        private val liveTranslate: Boolean = false, private val finishTail: Boolean = false,
        private val holdSetup: Boolean = false, private val recoverOverflow: Boolean = false,
        private val lateFinalReplay: Boolean = false, private val sourceOrderReplay: Boolean = false,
        private val translationStartReplay: Boolean = false, private val finishTailCount: Int = 1,
        private val xaiRecovery: JSONObject? = null, private val liveRecovery: JSONObject? = null,
        private val builtinRecovery: JSONObject? = null,
        private val geminiTranscribe: JSONObject? = null) : AutoCloseable {
        private val volcano = provider == ServiceProvider.VOLCANO
        private val tencent = provider == ServiceProvider.TENCENT
        private val qwen = provider == ServiceProvider.DASHSCOPE && !liveTranslate && builtinRecovery == null
        private val host = if (geminiTranscribe != null) "generativelanguage.googleapis.com"
            else if (volcano) "openspeech.bytedance.com" else if (builtinRecovery != null) when (provider) {
            ServiceProvider.OPENAI -> "api.openai.com"
            ServiceProvider.XAI -> "api.x.ai"
            ServiceProvider.GEMINI -> "generativelanguage.googleapis.com"
            ServiceProvider.DASHSCOPE -> "dashscope.aliyuncs.com"
            ServiceProvider.BAIDU -> "aip.baidubce.com"
            else -> error("Unsupported builtin recovery fixture")
        } else if (xaiRecovery != null) "api.x.ai"
            else if (liveTranslate) "synthetic-workspace.cn-beijing.maas.aliyuncs.com"
            else if (qwen) "dashscope.aliyuncs.com" else if (tencent) "asr.cloud.tencent.com" else "aip.baidubce.com"
        private val phases = JSONObject(requireNotNull(javaClass.getResourceAsStream("/translation-contracts.json"))
            .bufferedReader().use { it.readText() }).let { contract ->
            if (volcano || xaiRecovery != null || builtinRecovery != null) JSONArray()
            else if (translationStartReplay) JSONObject(requireNotNull(javaClass.getResourceAsStream("/live-pair-contracts.json"))
                .bufferedReader().use { it.readText() }).getJSONObject("translationStartActivity").getJSONArray("phases")
            else if (sourceOrderReplay) JSONArray(listOf(
                JSONArray(listOf(
                    """{"type":"conversation.item.created","item":{"id":"translation_z"},"previous_item_id":"source_z"}""",
                    """{"type":"conversation.item.created","item":{"id":"translation_a"},"previous_item_id":"source_a"}""",
                    """{"type":"conversation.item.input_audio_transcription.delta","item_id":"source_z","delta":"Synthetic old draft"}""",
                    """{"type":"conversation.item.input_audio_transcription.delta","item_id":"source_a","delta":"Synthetic current draft"}""",
                    """{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_a","transcript":"Synthetic current final."}""",
                    """{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_z","transcript":"Synthetic late old final."}""",
                )),
                JSONArray(listOf(
                    """{"type":"response.done","response":{"id":"response_z","status":"completed","output":[{"id":"translation_z","content":[{"type":"output_text","text":"Synthetic previous translation."}]}]}}""",
                )),
                JSONArray(listOf(
                    """{"type":"conversation.item.input_audio_transcription.delta","item_id":"source_z","delta":"Synthetic old replay"}""",
                    """{"type":"response.text.delta","item_id":"translation_a","delta":"Synthetic current preview"}""",
                )),
            ))
            else if (lateFinalReplay) JSONArray(listOf(
                JSONArray(listOf(
                    """{"type":"conversation.item.created","item":{"id":"translation_z"},"previous_item_id":"source_z"}""",
                    """{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_z","transcript":"Synthetic first sentence.","language":"en"}""",
                    """{"type":"conversation.item.input_audio_transcription.delta","item_id":"source_a","delta":"Synthetic second draft","language":"en"}""",
                )),
                JSONArray(listOf(
                    """{"type":"response.done","response":{"id":"response_z","status":"completed","output":[{"id":"translation_z","content":[{"type":"output_text","text":"合成第一句。"}]}]}}""",
                )),
                JSONArray(listOf(
                    """{"type":"conversation.item.input_audio_transcription.delta","item_id":"source_z","delta":"Stale first draft"}""",
                    """{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_z","transcript":"Stale first final."}""",
                    """{"type":"conversation.item.created","item":{"id":"translation_a"},"previous_item_id":"source_a"}""",
                    """{"type":"response.text.delta","response_id":"response_a","item_id":"translation_a","delta":"合成第二句预览"}""",
                )),
            ))
            else if (liveTranslate) JSONArray()
            else if (qwen) JSONArray(listOf(JSONArray(listOf("""{"header":{"event":"result-generated"},"payload":{"output":{"sentence":{"sentence_id":1,"sentence_end":true,"text":"Synthetic Qwen source."}}}}"""))))
            else if (tencent) contract.getJSONObject("tencentSpeechTranslation").getJSONArray("jniReplayPhases")
            else contract.getJSONObject("baiduRealtimeTranslation").getJSONArray("streams").let { streams ->
                val frames = (0 until streams.length()).map(streams::getJSONObject)
                    .single { it.getString("id") == "final-with-following-draft" }.getJSONArray("frames")
                JSONArray((0 until frames.length()).map { JSONArray(listOf(frames.getString(it))) })
            }
        }
        private val directory = Files.createTempDirectory("mimi-realtime-jni-").toFile()
        private val server = ServerSocket(0, 1, InetAddress.getByName("127.0.0.1"))
        private val textServer = if (qwen) ServerSocket(0, 1, InetAddress.getByName("127.0.0.1")) else null
        private val failure = AtomicReference<Throwable?>()
        private var socket: Socket? = null
        private var textSocket: Socket? = null
        @Volatile private var closed = false
        val nextMid = CountDownLatch(1)
        val nextFinal = CountDownLatch(1)
        val setupSeen = CountDownLatch(1)
        val releaseFirstSetup = CountDownLatch(1)
        val reconnected = CountDownLatch(1)
        val recoveredAudio = CountDownLatch(5)
        val recoveredAudioFrames = AtomicInteger(0)
        val recoveredTransportClosed = CountDownLatch(1)
        val noFurtherConnections = CountDownLatch(1)
        val connectionCount = AtomicInteger(0)
        val progressPongs = AtomicInteger(0)
        private val volcanoWire by lazy {
            JSONObject(requireNotNull(javaClass.getResourceAsStream("/translation-contracts.json"))
                .bufferedReader().use { it.readText() }).getJSONObject("volcanoAudioRequests")
        }
        val volcanoPcm: ByteArray by lazy {
            ByteArray(2 * volcanoWire.getInt("frameBytes") + 6) { (it * 73 + it / 256 + 19).toByte() }
        }
        val volcanoCompleteFrames = CountDownLatch(2)
        val finishSeen = CountDownLatch(1)
        val transportClosed = CountDownLatch(1)
        val network: JSONObject
        private val worker: Thread
        private val textWorker: Thread?

        init {
            // Generate a short-lived, synthetic server identity with the JDK used by Gradle.
            val keytool = java.io.File(System.getProperty("java.home"), "bin/keytool${if (System.getProperty("os.name").orEmpty().startsWith("Windows")) ".exe" else ""}")
            val storeFile = java.io.File(directory, "fixture.p12")
            val process = ProcessBuilder(keytool.absolutePath, "-genkeypair", "-alias", "fixture", "-keyalg", "EC", "-groupname", "secp256r1",
                "-dname", "CN=$host", "-ext", "SAN=dns:$host", "-ext", "EKU=serverAuth", "-validity", "2",
                "-storetype", "PKCS12", "-keystore", storeFile.absolutePath, "-storepass", "mimi-fixture", "-keypass", "mimi-fixture", "-noprompt")
                .redirectErrorStream(true).redirectOutput(java.io.File(directory, "keytool.log")).start()
            check(process.waitFor(20, TimeUnit.SECONDS)) { process.destroyForcibly(); "Synthetic TLS identity timed out" }
            check(process.exitValue() == 0) { "Synthetic TLS identity failed" }
            val password = "mimi-fixture".toCharArray()
            val store = KeyStore.getInstance("PKCS12").apply { storeFile.inputStream().use { load(it, password) } }
            val managers = KeyManagerFactory.getInstance(KeyManagerFactory.getDefaultAlgorithm()).apply { init(store, password) }
            val tls = SSLContext.getInstance("TLS").apply { init(managers.keyManagers, null, null) }
            network = JSONObject().put("speechProxy", JSONObject().put("mode", "custom").put("url", "http://127.0.0.1:${server.localPort}"))
                .put("textProxy", if (textServer == null) JSONObject().put("mode", "direct").put("url", JSONObject.NULL)
                    else JSONObject().put("mode", "custom").put("url", "http://127.0.0.1:${textServer.localPort}"))
                .put("trustRoots", JSONArray(listOf(Base64.getEncoder().encodeToString(store.getCertificate("fixture").encoded))))
            textWorker = textServer?.let { listener -> thread(name = "mimi-qwen-text-loopback", isDaemon = true) {
                try {
                    while (!closed) {
                        val tcp = listener.accept().also { textSocket = it; it.soTimeout = 5_000 }
                        check(headers(DataInputStream(tcp.getInputStream())).startsWith("CONNECT $host:443 HTTP/1.1\r\n"))
                        tcp.getOutputStream().write("HTTP/1.1 200 Connection Established\r\n\r\n".toByteArray(Charsets.US_ASCII))
                        val secure = (tls.socketFactory.createSocket(tcp, host, 443, true) as SSLSocket).apply { useClientMode = false; startHandshake() }
                        textSocket = secure
                        secure.use {
                            val input = DataInputStream(secure.inputStream)
                            val header = headers(input)
                            check(header.startsWith("POST /compatible-mode/v1/chat/completions HTTP/1.1\r\n"))
                            val length = header.lineSequence().first { it.startsWith("content-length:", true) }.substringAfter(':').trim().toInt()
                            check(length in 1..65_536)
                            val body = JSONObject(ByteArray(length).also(input::readFully).toString(Charsets.UTF_8))
                            check(body.getBoolean("stream") == qwenResponse!!.getBoolean("stream"))
                            val response = qwenResponse.getString("body").toByteArray(Charsets.UTF_8)
                            secure.getOutputStream().write(("HTTP/1.1 200 OK\r\nContent-Length: ${response.size}\r\nConnection: close\r\n\r\n").toByteArray(Charsets.US_ASCII))
                            secure.getOutputStream().write(response)
                        }
                        textSocket = null
                    }
                } catch (error: Throwable) { if (!closed) failure.set(error) }
            } }
            worker = thread(name = "mimi-realtime-loopback", isDaemon = true) {
                try {
                    server.soTimeout = 5_000
                    if (geminiTranscribe != null) {
                        acceptWebSocket(tls).use { secure ->
                            val input = DataInputStream(secure.inputStream)
                            val output = secure.outputStream
                            val setup = JSONObject(readText(input, output)).getJSONObject("setup")
                            check(setup.getString("model") == "models/gemini-3.5-transcribe-live")
                            fun binary(field: String) = frame(output, 2,
                                geminiTranscribe.getJSONObject(field).toString().toByteArray(Charsets.UTF_8))
                            binary("setup")
                            check(nextMid.await(5, TimeUnit.SECONDS))
                            binary("draft")
                            check(nextFinal.await(5, TimeUnit.SECONDS))
                            binary("final")
                            check(readText(input, output).isEmpty())
                        }
                        return@thread
                    }
                    if (builtinRecovery != null) {
                        builtinRecoverySession(tls)
                        return@thread
                    }
                    if (xaiRecovery != null) {
                        xaiRecoverySession(tls)
                        return@thread
                    }
                    if (liveRecovery != null) {
                        liveRecoverySession(tls)
                        return@thread
                    }
                    if (recoverOverflow) {
                        repeat(2) { index ->
                            acceptWebSocket(tls).use { secure ->
                                liveTranslateSession(DataInputStream(secure.inputStream), secure.outputStream, index + 1)
                            }
                        }
                        server.soTimeout = 500
                        try {
                            server.accept().use {
                                connectionCount.incrementAndGet()
                                error("Unexpected extra provider connection after PCM recovery")
                            }
                        } catch (_: java.net.SocketTimeoutException) {
                            noFurtherConnections.countDown()
                        }
                        return@thread
                    }
                    val secure = acceptWebSocket(tls)
                    val input = DataInputStream(secure.inputStream)
                    val output = secure.outputStream
                    if (volcano) {
                        volcanoSession(input, output)
                        return@thread
                    } else if (liveTranslate) {
                        liveTranslateSession(input, output)
                        return@thread
                    } else if (qwen) {
                        check(JSONObject(readText(input, output)).getJSONObject("header").getString("action") == "run-task")
                        send(output, """{"header":{"event":"task-started"}}""")
                    } else if (tencent) send(output, """{"code":0}""") else {
                        val start = JSONObject(readText(input, output))
                        check(start.getString("type") == "START" && start.getString("app_key") == "synthetic-appKey")
                        check(start.getString("from") == "en" && start.getString("to") == "zh")
                        send(output, """{"code":0,"data":{"status":"STA"}}""")
                    }
                    fun phase(index: Int) {
                        val frames = phases.getJSONArray(index)
                        repeat(frames.length()) { send(output, frames.getString(it)) }
                    }
                    phase(0)
                    if (qwen) {
                        val finish = readText(input, output)
                        if (finish.isNotEmpty()) {
                            check(JSONObject(finish).getJSONObject("header").getString("action") == "finish-task")
                            send(output, """{"header":{"event":"task-finished"}}""")
                        }
                    } else {
                        check(nextMid.await(10, TimeUnit.SECONDS))
                        phase(1)
                        check(nextFinal.await(10, TimeUnit.SECONDS))
                        phase(2)
                        check(JSONObject(readText(input, output)).getString("type") == if (tencent) "end" else "FINISH")
                        send(output, if (tencent) """{"code":0,"final":1}""" else """{"code":0,"data":{"status":"END"}}""")
                    }
                } catch (error: Throwable) { if (!closed) failure.set(error) }
            }
        }

        private fun acceptWebSocket(tls: SSLContext): SSLSocket {
            val tcp = server.accept().also { socket = it; it.soTimeout = 5_000 }
            connectionCount.incrementAndGet()
            check(headers(DataInputStream(tcp.getInputStream())).startsWith("CONNECT $host:443 HTTP/1.1\r\n"))
            tcp.getOutputStream().write("HTTP/1.1 200 Connection Established\r\n\r\n".toByteArray(Charsets.US_ASCII))
            val secure = (tls.socketFactory.createSocket(tcp, host, 443, true) as SSLSocket).apply { useClientMode = false; startHandshake() }
            socket = secure
            val input = DataInputStream(secure.inputStream)
            val request = headers(input)
            check(request.startsWith(if (geminiTranscribe != null)
                "GET /ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent HTTP/1.1\r\n"
                else if (volcano) "GET /api/v4/ast/v2/translate HTTP/1.1\r\n"
                else if (builtinRecovery != null) when (provider) {
                ServiceProvider.OPENAI -> "GET /v1/realtime/translations?model=gpt-realtime-translate HTTP/1.1\r\n"
                ServiceProvider.XAI -> "GET /v1/realtime?model=grok-voice-latest HTTP/1.1\r\n"
                ServiceProvider.GEMINI -> "GET /ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent?key=synthetic-apiKey HTTP/1.1\r\n"
                ServiceProvider.DASHSCOPE -> "GET /api-ws/v1/inference HTTP/1.1\r\n"
                ServiceProvider.BAIDU -> "GET /ws/realtime_speech_trans HTTP/1.1\r\n"
                else -> error("Unsupported builtin recovery fixture")
            } else if (xaiRecovery != null) "GET /v1/stt?model=grok-voice-transcribe-2.0&sample_rate=16000&encoding=pcm&interim_results=true&language=en HTTP/1.1\r\n"
                else if (liveTranslate) "GET /api-ws/v1/realtime?model=qwen3.8-livetranslate-flash-realtime HTTP/1.1\r\n"
                else if (qwen) "GET /api-ws/v1/inference HTTP/1.1\r\n" else if (tencent) "GET /asr/speech_translate/123456789?" else "GET /ws/realtime_speech_trans HTTP/1.1\r\n"))
            if (volcano) {
                fun header(name: String) = request.lineSequence().first { it.startsWith("$name:", true) }.substringAfter(':').trim()
                check(header("X-Api-Key") == "synthetic-apiKey")
                check(header("X-Api-Resource-Id") == "volc.service_type.10053")
            }
            val key = request.lineSequence().first { it.startsWith("sec-websocket-key:", true) }.substringAfter(':').trim()
            val accept = Base64.getEncoder().encodeToString(MessageDigest.getInstance("SHA-1").digest((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").toByteArray(Charsets.US_ASCII)))
            secure.outputStream.write("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: $accept\r\n\r\n".toByteArray(Charsets.US_ASCII))
            return secure
        }

        private fun volcanoSession(input: DataInputStream, output: OutputStream) {
            fun hexBytes(hex: String) = hex.chunked(2).map { it.toInt(16).toByte() }.toByteArray()
            val wire = volcanoWire
            val metadataPrefix = hexBytes(wire.getString("metadataPrefixHex"))
            val setup = checkNotNull(readBinary(input, output))
            val sessionEnd = metadataPrefix.size + wire.getInt("sessionBytes")
            check(setup.size >= sessionEnd)
            assertArrayEquals(metadataPrefix, setup.copyOfRange(0, metadataPrefix.size))
            val session = setup.copyOfRange(metadataPrefix.size, sessionEnd)
            java.util.UUID.fromString(session.toString(Charsets.US_ASCII))
            val metadata = metadataPrefix + session
            assertArrayEquals(metadata + hexBytes(wire.getString("startSuffixHex")), setup)
            frame(output, 2, hexBytes(wire.getString("sessionStartedHex")))

            val frameBytes = volcanoWire.getInt("frameBytes")
            val audioPrefix = metadata + hexBytes(volcanoWire.getString("audioSuffixHex"))
            repeat(2) { index ->
                val expected = audioPrefix + volcanoPcm.copyOfRange(index * frameBytes, (index + 1) * frameBytes)
                assertArrayEquals(expected, checkNotNull(readBinary(input, output)))
                volcanoCompleteFrames.countDown()
            }
            val tail = ByteArray(frameBytes)
            volcanoPcm.copyInto(tail, 0, frameBytes * 2)
            assertArrayEquals(audioPrefix + tail, checkNotNull(readBinary(input, output)))
            assertArrayEquals(metadata + hexBytes(wire.getString("finishSuffixHex")), checkNotNull(readBinary(input, output)))
            finishSeen.countDown()
            frame(output, 2, hexBytes(wire.getString("sessionFinishedHex")))
            check(readBinary(input, output) == null) { "Volcano sent another request after FinishSession" }
        }

        private fun builtinRecoverySession(tls: SSLContext) {
            val recovery = requireNotNull(builtinRecovery)
            repeat(recovery.getInt("expectedConnections")) { index ->
                val connection = index + 1
                acceptWebSocket(tls).use { secure ->
                    val input = DataInputStream(secure.inputStream)
                    val output = secure.outputStream
                    val setup = JSONObject(readText(input, output, connection))
                    when (provider) {
                        ServiceProvider.GEMINI -> check(setup.getJSONObject("setup").getJSONObject("generationConfig")
                            .getJSONObject("translationConfig").getString("targetLanguageCode") == "zh-Hans")
                        ServiceProvider.DASHSCOPE -> check(setup.getJSONObject("header").getString("action") == "run-task")
                        ServiceProvider.BAIDU -> {
                            check(setup.getString("type") == "START")
                            check(setup.getString("from") == "en" && setup.getString("to") == "zh")
                            check(setup.getInt("sampling_rate") == 16_000)
                        }
                        else -> check(setup.getString("type") == "session.update")
                    }
                    if (index > 0 || !recovery.getBoolean("duringSetup")) send(output, builtinReadyFrame())
                    if (recovery.optBoolean("repeatFailures")) {
                        send(output, recovery.getJSONObject("failureFrame").toString())
                    } else if (index == 0) {
                        if (!recovery.getBoolean("duringSetup")) check(nextMid.await(5, TimeUnit.SECONDS))
                        if (recovery.optBoolean("quietAudioFailure")) {
                            var quietFrames = 0
                            check(readText(input, output, connection, onBinary = { pcm ->
                                if (quietFrames < 550) {
                                    check(pcm.contentEquals(ByteArray(1_280)))
                                    if (++quietFrames == 550 && !recovery.optBoolean("lateQuietAudioFailure")) {
                                        send(output, recovery.getJSONObject("failureFrame").toString())
                                    }
                                } else {
                                    check(recovery.optBoolean("lateQuietAudioFailure"))
                                    check(pcm.contentEquals(ByteArray(1_280) { 7 }))
                                    Thread.sleep(1_500)
                                    send(output, recovery.getJSONObject("failureFrame").toString())
                                }
                            }).isEmpty())
                            check(quietFrames == 550)
                        } else if (recovery.optBoolean("sourceProgressReplay")) {
                            // No microphone, system capture, provider account or wall-clock
                            // shortcut: exercise the real receiver and native health loop.
                            secure.soTimeout = 15_000
                            val writer = thread(name = "mimi-source-progress-fixture", isDaemon = true) {
                                try {
                                    builtinRecoveredCaption(output)
                                    Thread.sleep(15_000)
                                    send(output, """{"type":"session.input_transcript.delta","delta":"Original after waiting."}""")
                                    send(output, """{"type":"session.output_transcript.delta","delta":"等待后的字幕。"}""")
                                    repeat(12) {
                                        Thread.sleep(2_000)
                                        send(output, """{"type":"session.output_transcript.delta","delta":"合成的持续译文片段"}""")
                                    }
                                    send(output, """{"type":"session.input_transcript.delta","delta":"Fresh original."}""")
                                    send(output, """{"type":"session.output_transcript.delta","delta":"新的译文。"}""")
                                } catch (_: InterruptedException) {
                                    // Explicit Stop retires the socket and its writer.
                                } catch (error: Throwable) { if (!closed) failure.set(error) }
                            }
                            try {
                                check(readText(input, output, connection, afterPong = { progressPongs.incrementAndGet() }).isEmpty())
                                transportClosed.countDown()
                            } finally {
                                writer.interrupt()
                                writer.join(2_000)
                                check(!writer.isAlive)
                            }
                        } else if (recovery.isNull("failureFrame")) frame(output, 8, byteArrayOf(3, -24))
                        else send(output, recovery.getJSONObject("failureFrame").toString())
                    } else {
                        reconnected.countDown()
                        if (provider == ServiceProvider.DASHSCOPE || provider == ServiceProvider.BAIDU) {
                            // These providers send binary PCM; emit captions only after all
                            // recovered frames have crossed JNI and the real socket.
                            val frameBytes = if (provider == ServiceProvider.BAIDU) 1_280 else 3_200
                            val finish = JSONObject(readText(input, output, connection, onBinary = { pcm ->
                                check(pcm.contentEquals(ByteArray(frameBytes) { 7 }))
                                recoveredAudioFrames.incrementAndGet()
                                recoveredAudio.countDown()
                                if (recoveredAudioFrames.get() == 5) {
                                    if (provider == ServiceProvider.BAIDU) builtinRecoveredCaption(output)
                                    else send(output, """{"header":{"event":"result-generated"},"payload":{"output":{"sentence":{"sentence_id":1,"sentence_end":true,"text":"Synthetic recovered caption."}}}}""")
                                }
                            }))
                            check(recoveredAudioFrames.get() == 5)
                            if (provider == ServiceProvider.BAIDU) {
                                check(finish.getString("type") == "FINISH")
                                send(output, """{"code":0,"data":{"status":"END"}}""")
                            } else {
                                check(finish.getJSONObject("header").getString("action") == "finish-task")
                                send(output, """{"header":{"event":"task-finished"}}""")
                            }
                        } else {
                            repeat(5) {
                                val audio = JSONObject(readText(input, output, connection))
                                val encoded = if (provider == ServiceProvider.GEMINI) {
                                    val value = audio.getJSONObject("realtimeInput").getJSONObject("audio")
                                    check(value.getString("mimeType") == "audio/pcm;rate=16000")
                                    value.getString("data")
                                } else {
                                    val expectedType = if (provider == ServiceProvider.OPENAI) "session.input_audio_buffer.append" else "input_audio_buffer.append"
                                    check(audio.getString("type") == expectedType) { "Unexpected recovered audio frame type" }
                                    audio.getString("audio")
                                }
                                val frameBytes = if (provider == ServiceProvider.GEMINI) 3_200 else 9_600
                                check(Base64.getDecoder().decode(encoded).contentEquals(ByteArray(frameBytes) { 7 }))
                                recoveredAudioFrames.incrementAndGet()
                                recoveredAudio.countDown()
                            }
                            if (recovery.optLong("delayedCloseMs") == 0L) builtinRecoveredCaption(output)
                            when (provider) {
                                ServiceProvider.OPENAI -> {
                                    check(JSONObject(readText(input, output, connection)).getString("type") == "session.close")
                                    if (recovery.optLong("delayedCloseMs") > 0L) {
                                        Thread.sleep(recovery.getLong("delayedCloseMs"))
                                        builtinRecoveredCaption(output)
                                    }
                                    send(output, """{"type":"session.closed"}""")
                                }
                                ServiceProvider.GEMINI -> {
                                    check(JSONObject(readText(input, output, connection)).getJSONObject("realtimeInput").getBoolean("audioStreamEnd"))
                                    send(output, """{"serverContent":{"turnComplete":true}}""")
                                }
                                // Grok is hard-stopped after its final pair is observed.
                                ServiceProvider.XAI -> Unit
                                else -> error("Unsupported recovered caption fixture")
                            }
                        }
                    }
                    if (!(index == 0 && (recovery.optBoolean("sourceProgressReplay") || recovery.optBoolean("quietAudioFailure")))) {
                        check(readText(input, output, connection).isEmpty())
                    }
                }
            }
            server.soTimeout = 500
            try {
                server.accept().use { connectionCount.incrementAndGet(); error("Unexpected extra builtin connection") }
            } catch (_: java.net.SocketTimeoutException) { noFurtherConnections.countDown() }
        }

        private fun builtinReadyFrame(): String = when (provider) {
            ServiceProvider.OPENAI -> """{"type":"session.updated","session":{"audio":{"input":{"transcription":{"model":"gpt-realtime-whisper"}},"output":{"language":"zh"}}}}"""
            ServiceProvider.XAI -> """{"type":"session.updated","session":{"reasoning":{"effort":"none"},"turn_detection":{"type":"server_vad"},"audio":{"input":{"format":{"type":"audio/pcm","rate":24000},"transcription":{"model":"grok-transcribe"}}}}}"""
            ServiceProvider.GEMINI -> """{"setupComplete":{}}"""
            ServiceProvider.DASHSCOPE -> """{"header":{"event":"task-started"}}"""
            ServiceProvider.BAIDU -> """{"code":0,"data":{"status":"STA"}}"""
            else -> error("Unsupported builtin ready fixture")
        }

        private fun builtinRecoveredCaption(output: OutputStream) {
            val frames = when (provider) {
                ServiceProvider.OPENAI -> listOf(
                    """{"type":"session.input_transcript.delta","delta":"Synthetic recovered caption."}""",
                    """{"type":"session.output_transcript.delta","delta":"合成恢复字幕。"}""",
                )
                ServiceProvider.XAI -> listOf(
                    """{"type":"conversation.item.input_audio_transcription.completed","item_id":"recovered-source","transcript":"Synthetic recovered caption."}""",
                    """{"type":"response.created","response":{"id":"recovered-response"}}""",
                    """{"type":"response.output_audio_transcript.done","response_id":"recovered-response","transcript":"合成恢复字幕。"}""",
                    """{"type":"response.done","response":{"id":"recovered-response","status":"completed"}}""",
                )
                ServiceProvider.GEMINI -> listOf("""{"serverContent":{"inputTranscription":{"text":"Synthetic recovered caption."},"outputTranscription":{"text":"合成恢复字幕。"}}}""")
                ServiceProvider.BAIDU -> listOf("""{"code":0,"data":{"status":"TRN","result":{"type":"FIN","asr":"","asr_trans":"","sentence":"Synthetic recovered caption.","sentence_trans":"合成恢复字幕。"}}}""")
                else -> error("Unsupported builtin subtitle fixture")
            }
            frames.forEach { send(output, it) }
        }

        private fun xaiRecoverySession(tls: SSLContext) {
            val recovery = requireNotNull(xaiRecovery)
            acceptWebSocket(tls).use { first ->
                send(first.outputStream, """{"type":"transcript.created"}""")
                check(nextMid.await(5, TimeUnit.SECONDS))
                if (!recovery.isNull("failureFrame")) {
                    send(first.outputStream, recovery.getJSONObject("failureFrame").toString())
                    check(readText(DataInputStream(first.inputStream), first.outputStream).isEmpty())
                }
            }
            if (recovery.getInt("expectedConnections") == 2) {
                acceptWebSocket(tls).use { second ->
                    send(second.outputStream, """{"type":"transcript.created"}""")
                    reconnected.countDown()
                    val finish = JSONObject(readText(DataInputStream(second.inputStream), second.outputStream))
                    check(finish.getString("type") == "audio.done")
                    send(second.outputStream, """{"type":"transcript.done","duration":0}""")
                }
            }
            server.soTimeout = 500
            try {
                server.accept().use {
                    connectionCount.incrementAndGet()
                    error("Unexpected extra xAI connection after recovery completed")
                }
            } catch (_: java.net.SocketTimeoutException) {
                noFurtherConnections.countDown()
            }
        }

        private fun liveRecoverySession(tls: SSLContext) {
            val recovery = requireNotNull(liveRecovery)
            repeat(recovery.getInt("expectedConnections")) { connection ->
                acceptWebSocket(tls).use { secure ->
                    val input = DataInputStream(secure.inputStream)
                    val output = secure.outputStream
                    check(JSONObject(readText(input, output)).getString("type") == "session.update")
                    val retrySetupFailure = if (connection > 0) recovery.optJSONObject("retrySetupFailureFrame") else null
                    if (retrySetupFailure == null && (connection != 0 || !recovery.optBoolean("duringSetup"))) {
                        send(output, """{"type":"session.updated"}""")
                    }
                    if (recovery.optBoolean("repeatFailures")) {
                        if (connection + 1 == recovery.optInt("stableConnection")) {
                            // The second periodic Ping proves the native health worker
                            // completed its first successful probe; no arbitrary sleep
                            // or subtitle event is used as a stability signal.
                            secure.soTimeout = 15_000
                            var healthProbes = 0
                            check(readText(input, output, connection + 1) {
                                healthProbes++
                                if (healthProbes == 2) send(output, recovery.getJSONObject("failureFrame").toString())
                            }.isEmpty())
                            check(healthProbes == 2)
                            return@use
                        }
                        send(output, recovery.getJSONObject("failureFrame").toString())
                    } else if (retrySetupFailure != null) {
                        send(output, retrySetupFailure.toString())
                    } else if (connection == 0) {
                        if (!recovery.optBoolean("duringSetup")) check(nextMid.await(5, TimeUnit.SECONDS))
                        if (recovery.isNull("failureFrame")) frame(output, 8, byteArrayOf(3, -24))
                        else send(output, recovery.getJSONObject("failureFrame").toString())
                    } else {
                        reconnected.countDown()
                        repeat(5) {
                            val audio = JSONObject(readText(input, output, 2))
                            check(audio.getString("type") == "input_audio_buffer.append")
                            check(Base64.getDecoder().decode(audio.getString("audio")).contentEquals(ByteArray(3_200) { 7 }))
                            recoveredAudio.countDown()
                        }
                        send(output, """{"type":"conversation.item.input_audio_transcription.completed","item_id":"recovered-source","transcript":"Synthetic recovered caption."}""")
                        send(output, """{"type":"conversation.item.created","previous_item_id":"recovered-source","item":{"id":"recovered-output","role":"assistant"}}""")
                        send(output, """{"type":"response.done","response":{"id":"recovered-response","status":"completed","output":[{"id":"recovered-output","content":[{"type":"output_text","text":"合成恢复字幕。"}]}]}}""")
                        check(JSONObject(readText(input, output, 2)).getString("type") == "session.finish")
                        send(output, """{"type":"session.finished"}""")
                    }
                    check(readText(input, output, connection + 1).isEmpty())
                }
            }
            server.soTimeout = 500
            try {
                server.accept().use { connectionCount.incrementAndGet(); error("Unexpected extra LiveTranslate connection") }
            } catch (_: java.net.SocketTimeoutException) { noFurtherConnections.countDown() }
        }

        private fun liveTranslateSession(input: DataInputStream, output: OutputStream, connection: Int = 1) {
            val update = JSONObject(readText(input, output, connection))
            check(update.getString("type") == "session.update")
            check(update.getJSONObject("session").getJSONObject("translation").getString("language") == "zh")
            setupSeen.countDown()
            if (recoverOverflow && connection == 1) check(releaseFirstSetup.await(5, TimeUnit.SECONDS))
            if (!holdSetup) send(output, """{"type":"session.updated"}""")
            if (recoverOverflow && connection == 2) reconnected.countDown()
            if (lateFinalReplay || sourceOrderReplay || translationStartReplay) {
                fun phase(index: Int) {
                    val frames = phases.getJSONArray(index)
                    repeat(frames.length()) { send(output, if (translationStartReplay) frames.getJSONObject(it).toString() else frames.getString(it)) }
                }
                phase(0)
                check(nextMid.await(10, TimeUnit.SECONDS))
                phase(1)
                check(nextFinal.await(10, TimeUnit.SECONDS))
                phase(2)
            }
            while (true) {
                val message = readText(input, output, connection)
                if (message.isEmpty()) return
                val value = JSONObject(message)
                val type = value.getString("type")
                if (recoverOverflow && type == "input_audio_buffer.append") {
                    if (connection == 2) {
                        check(Base64.getDecoder().decode(value.getString("audio")).contentEquals(ByteArray(3_200) { 7 }))
                        recoveredAudioFrames.incrementAndGet()
                        recoveredAudio.countDown()
                    }
                    continue
                }
                check(type == "session.finish")
                check(finishSeen.count == 1L)
                finishSeen.countDown()
                // Holding the response keeps the production finish future alive.
                // Read only Close/EOF after stop; no subtitle probe can retire it.
                if (translationStartReplay) {
                    val tail = JSONObject(requireNotNull(javaClass.getResourceAsStream("/live-pair-contracts.json"))
                        .bufferedReader().use { it.readText() }).getJSONObject("translationStartActivity").getJSONArray("finishFrames")
                    repeat(tail.length()) { send(output, tail.getJSONObject(it).toString()) }
                } else if (sourceOrderReplay) {
                    send(output, """{"type":"response.done","response":{"id":"response_a","status":"completed","output":[{"id":"translation_a","content":[{"type":"output_text","text":"Synthetic current translation."}]}]}}""")
                    send(output, """{"type":"session.finished"}""")
                } else if (finishTail) {
                    repeat(finishTailCount) { index ->
                        val source = if (finishTailCount == 1) "Synthetic finish tail." else "Synthetic finish tail $index."
                        for (event in listOf(
                            """{"type":"conversation.item.input_audio_transcription.delta","item_id":"source$index","delta":"$source"}""",
                            """{"type":"conversation.item.created","previous_item_id":"source$index","item":{"id":"out$index","role":"assistant"}}""",
                            """{"type":"response.text.done","item_id":"out$index","response_id":"response$index","text":"合成收尾字幕。"}""",
                            """{"type":"conversation.item.input_audio_transcription.completed","item_id":"source$index","transcript":"$source"}""",
                            """{"type":"response.done","response":{"id":"response$index","status":"completed","output":[]}}""",
                        )) send(output, event)
                    }
                    send(output, """{"type":"session.finished"}""")
                } else if (lateFinalReplay) {
                    send(output, """{"type":"session.finished"}""")
                }
            }
        }

        fun checkFailure() { failure.get()?.let { throw AssertionError("$provider loopback server failed", it) } }
        private fun headers(input: DataInputStream): String {
            val bytes = java.io.ByteArrayOutputStream()
            var tail = 0
            while (bytes.size() < 16_384) {
                val byte = input.readUnsignedByte()
                bytes.write(byte)
                tail = (tail shl 8) or byte
                if (tail == 0x0d0a0d0a) return bytes.toString("US-ASCII")
            }
            error("Oversized loopback HTTP header")
        }
        private fun send(output: OutputStream, text: String) = frame(output, 1, text.toByteArray(Charsets.UTF_8))
        private fun frame(output: OutputStream, opcode: Int, bytes: ByteArray) = synchronized(output) {
            require(bytes.size < 65_536)
            output.write(0x80 or opcode)
            if (bytes.size < 126) output.write(bytes.size) else {
                output.write(126); output.write(bytes.size shr 8); output.write(bytes.size and 255)
            }
            output.write(bytes)
            output.flush()
        }
        private fun readFrame(input: DataInputStream): Pair<Int, ByteArray> {
            val opcode = input.readUnsignedByte() and 15
            val flags = input.readUnsignedByte()
            check(flags and 128 != 0)
            val size = when (val short = flags and 127) { 126 -> input.readUnsignedShort(); 127 -> error("Oversized loopback frame"); else -> short }
            check(size <= if (builtinRecovery != null) 16_384 else if (liveTranslate) 8_192 else 4_096)
            val mask = ByteArray(4).also(input::readFully)
            val payload = ByteArray(size).also(input::readFully)
            for (index in payload.indices) payload[index] = (payload[index].toInt() xor mask[index % 4].toInt()).toByte()
            return opcode to payload
        }
        private fun readBinary(input: DataInputStream, output: OutputStream): ByteArray? {
            try {
                while (true) {
                    val (opcode, payload) = readFrame(input)
                    when (opcode) {
                        2 -> return payload
                        9 -> frame(output, 10, payload)
                        8 -> { observeTransportClosed(1); return null }
                        else -> error("Unexpected Volcano client frame")
                    }
                }
            } catch (_: java.io.EOFException) {
                observeTransportClosed(1)
                return null
            }
        }
        private fun readText(input: DataInputStream, output: OutputStream, connection: Int = 1,
            onBinary: ((ByteArray) -> Unit)? = null, afterPong: (() -> Unit)? = null): String {
            try {
                while (true) {
                    val (opcode, payload) = readFrame(input)
                    if ((qwen || liveTranslate || xaiRecovery != null || builtinRecovery != null || geminiTranscribe != null) && opcode == 8) {
                        if (liveTranslate || builtinRecovery != null || geminiTranscribe != null) observeTransportClosed(connection)
                        return ""
                    }
                    if (builtinRecovery != null && opcode == 2) {
                        check((provider == ServiceProvider.DASHSCOPE || provider == ServiceProvider.BAIDU) && onBinary != null) {
                            "Unexpected binary frame for builtin recovery protocol"
                        }
                        onBinary(payload)
                        continue
                    }
                    if ((qwen || liveTranslate) && opcode == 2) continue
                    if (opcode == 9) {
                        frame(output, 10, payload)
                        afterPong?.invoke()
                    } else {
                        check(opcode == 1) { "Unexpected loopback client frame" }
                        return payload.toString(Charsets.UTF_8)
                    }
                }
            } catch (error: java.io.EOFException) {
                if (!liveTranslate && xaiRecovery == null && builtinRecovery == null && geminiTranscribe == null) throw error
                observeTransportClosed(connection)
                return ""
            }
        }
        private fun observeTransportClosed(connection: Int) {
            if (connection == 2) recoveredTransportClosed.countDown() else transportClosed.countDown()
        }
        override fun close() {
            closed = true
            nextMid.countDown(); nextFinal.countDown(); releaseFirstSetup.countDown()
            socket?.close(); textSocket?.close(); server.close(); textServer?.close()
            worker.join(2_000)
            textWorker?.join(2_000)
            check(!worker.isAlive && textWorker?.isAlive != true) { "Synthetic realtime transport did not stop" }
            directory.deleteRecursively()
        }
    }
}
