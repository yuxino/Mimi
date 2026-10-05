package app.yuxino.mimi.android.provider

import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Protocol
import okhttp3.Request
import okhttp3.Response
import okhttp3.ResponseBody.Companion.toResponseBody
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

/** One checked-in synthetic contract is exercised by both native implementations. */
class SharedTranslationContractTest {
    private val contract = requireNotNull(javaClass.getResourceAsStream("/translation-contracts.json")) {
        "Missing shared translation contract resource"
    }.bufferedReader().use { JSONObject(it.readText()) }.also { assertEquals(1, it.getInt("schemaVersion")) }

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

    @Test fun sharedSpeechLanguageSetupsAndCatalogs() {
        val setups = contract.getJSONArray("speechLanguageSetups")
        repeat(setups.length()) { index ->
            val case = setups.getJSONObject(index)
            val actual = runCatching {
                val target = case.getString("target")
                when(case.getString("provider")) {
                    "openAIRealtime" -> JSONObject().put("targetCode", openAITranslationSetup(target).getJSONObject("session").getJSONObject("audio").getJSONObject("output").getString("language"))
                    "googleGeminiLive" -> JSONObject().put("targetCode", JSONObject((GeminiProtocol(ServiceConfiguration(ServiceProvider.GEMINI, emptyMap()), target).setup() as WireFrame.Text).value).getJSONObject("setup").getJSONObject("generationConfig").getJSONObject("translationConfig").getString("targetLanguageCode"))
                    "xAIRealtime" -> {
                        val session = JSONObject((GrokProtocol(ServiceConfiguration(ServiceProvider.XAI, emptyMap()), target, case.getString("source")).setup() as WireFrame.Text).value)
                        JSONObject().put("sourceHint", session.getJSONObject("session").getJSONObject("audio").getJSONObject("input").getJSONObject("transcription").opt("language_hint") ?: JSONObject.NULL)
                    }
                    else -> error("unknown_provider")
                }
            }
            if (case.isNull("expected")) assertTrue(case.getString("id"), actual.isFailure)
            else assertTrue(case.getString("id"), case.getJSONObject("expected").similar(actual.getOrThrow()))
        }
        val catalogs = contract.getJSONArray("speechLanguageCatalogs")
        repeat(catalogs.length()) { index ->
            val case = catalogs.getJSONObject(index)
            val provider = when(case.getString("provider")) { "openAIRealtime" -> ServiceProvider.OPENAI; "googleGeminiLive" -> ServiceProvider.GEMINI; else -> ServiceProvider.XAI }
            val expected = case.getJSONObject("expected")
            assertTrue(case.getString("id"), expected.getJSONArray("sourceLanguages").similar(org.json.JSONArray(provider.sources)))
            assertTrue(case.getString("id"), expected.getJSONArray("targetLanguages").similar(org.json.JSONArray(provider.targets)))
        }
    }

    @Test fun sharedLiveSetupContracts() = cases("liveSetups") { case ->
        assertEquals("googleGeminiLive", case.getString("provider"))
        val protocol = GeminiProtocol(ServiceConfiguration(ServiceProvider.GEMINI, emptyMap()), case.getString("target"))
        val actual = JSONObject((protocol.setup() as WireFrame.Text).value)
        assertTrue(case.getString("id"), case.getJSONObject("expected").similar(actual))
    }

    @Test fun sharedGeminiTranscriptContractsCrossActualJni() = cases("liveTranscriptSequences") { case ->
        var now = 0L
        val protocol = GeminiProtocol(ServiceConfiguration(ServiceProvider.GEMINI, emptyMap()), "ja") { now }
        val pairs = org.json.JSONArray()
        val drafts = org.json.JSONArray()
        val actual = runCatching {
            val frames = case.getJSONArray("frames")
            repeat(frames.length()) { index ->
                now += 100
                val events = protocol.text(frames.getString(index)).toMutableList()
                val settle = case.optJSONArray("settleAfterFrames")
                if (settle != null && (0 until settle.length()).any { settle.getInt(it) == index }) {
                    now += 2_000
                    events += protocol.tick()
                }
                events.filterIsInstance<ServiceEvent.FinalPair>().forEach {
                    pairs.put(JSONObject().put("source", it.source).put("translation", it.translation).put("language", it.language ?: JSONObject.NULL))
                }
                events.forEach { event ->
                    when (event) {
                        is ServiceEvent.Source -> drafts.put(JSONObject().put("kind", "source").put("text", event.text))
                        is ServiceEvent.Translation -> drafts.put(JSONObject().put("kind", "translation").put("text", event.text))
                        else -> Unit
                    }
                }
            }
        }
        if (case.isNull("expected")) assertTrue(case.getString("id"), actual.isFailure)
        else {
            assertTrue(case.getString("id"), actual.isSuccess)
            assertTrue(case.getString("id"), case.getJSONArray("expected").similar(pairs))
            if (case.has("expectedDrafts")) assertTrue(case.getString("id"), case.getJSONArray("expectedDrafts").similar(drafts))
        }
    }

    @Test fun sharedRequestContracts() = cases("requests") { case ->
        val actual = runCatching {
            val text = case.getString("text")
            val source = case.getString("source")
            val target = case.getString("target")
            when (case.getString("provider")) {
                "deepL" -> deepLRequest(text, source, target)
                "deepLX" -> deepLXRequest(text, source, target)
                "openaiCompatible" -> buildTranslationRequest(case.getString("model"), text, source, target)
                else -> error("Unknown contract provider")
            }
        }
        if (case.isNull("expected")) assertTrue(case.getString("id"), actual.isFailure)
        else assertTrue(case.getString("id"), case.getJSONObject("expected").similar(actual.getOrThrow()))
    }

    @Test fun genericRequestContractsCoverEveryConfigurableTarget() {
        val targets = mutableSetOf<String>()
        cases("requests") { case ->
            if (case.getString("provider") == "openaiCompatible" && !case.isNull("expected")) targets += case.getString("target")
        }
        assertEquals(OPENAI_COMPATIBLE_TARGET_LANGUAGE_NAMES.keys, targets)
        assertFalse(targets.contains("original"))
    }

    @Test fun sharedDetectedSourceRequestContracts() = cases("detectedSourceRequests") { case ->
        val actual = runCatching {
            val reported = if (case.isNull("reported")) null else case.getString("reported")
            val source = translationSourceLanguage(case.getString("source"), reported)
            val text = case.getString("text")
            val target = case.getString("target")
            when (case.getString("provider")) {
                "deepL" -> deepLRequest(text, source, target)
                "deepLX" -> deepLXRequest(text, source, target)
                else -> error("Unknown detected-source provider")
            }
        }
        if (case.isNull("expected")) assertTrue(case.getString("id"), actual.isFailure)
        else assertTrue(case.getString("id"), case.getJSONObject("expected").similar(actual.getOrThrow()))
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

    @Test fun sharedResponseContracts() = cases("responses") { case ->
        val actual = runCatching {
            val body = case.getString("body")
            when (case.getString("provider")) {
                "deepL" -> decodeDeepLResponse(body)
                "deepLX" -> decodeDeepLXResponse(body)
                "openaiCompatible" -> decodeTranslationResponse(body)
                else -> error("Unknown contract provider")
            }
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

    @Test fun sharedOptionalAuthorizationUsesActualClientRequests() = cases("optionalAuthorization") { case ->
        val provider = TextTranslationProvider.entries.single { it.storageId == case.getString("provider") }
        val responses = contract.getJSONArray("responses")
        val success = (0 until responses.length()).map(responses::getJSONObject).first {
            it.getString("provider") == provider.storageId && !it.isNull("expected")
        }
        val request = AtomicReference<Request>()
        val result = AtomicReference<TranslationResult>()
        val done = CountDownLatch(1)
        val http = OkHttpClient.Builder().addInterceptor { chain ->
            request.set(chain.request())
            Response.Builder().request(chain.request()).protocol(Protocol.HTTP_1_1)
                .code(200).message("Synthetic fixture")
                .body(success.getString("body").toResponseBody("application/json".toMediaType())).build()
        }.build()
        val configuration = TranslationConfiguration("https://example.test/v1", "synthetic-model",
            case.getString("apiKey"), provider = provider)
        val client = when (provider) {
            TextTranslationProvider.OPENAI_COMPATIBLE -> OpenAITranslationClient(configuration, http)
            TextTranslationProvider.DEEPLX -> DeepLXTranslationClient(configuration, http)
            else -> error("Unknown optional-authorization provider")
        }
        val call = client.check { result.set(it); done.countDown() }
        try {
            assertTrue(case.getString("id"), done.await(3, TimeUnit.SECONDS))
            assertTrue(case.getString("id"), result.get() is TranslationResult.Success)
            val expected = if (case.isNull("expected")) null else case.getString("expected")
            assertEquals(case.getString("id"), expected, request.get().header("Authorization"))
        } finally {
            call.cancel()
            http.dispatcher.executorService.shutdownNow()
            http.connectionPool.evictAll()
        }
    }
}
