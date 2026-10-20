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
        assertEquals("deepLX", config.getString("provider"))
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

    private fun createLiveTranslate(fixture: RealtimeLoopback): Long = NativeRuntimeConfiguration.exchange(
        JSONObject().put("operation", "create").put("configuration", configuration()
            .put("alibabaRealtimeMode", "liveTranslate").put("alibabaWorkspaceId", "synthetic-workspace")
            .put("sourceLanguage", "auto")).put("network", fixture.network)).getLong("handle")

    private fun awaitRuntimeState(handle: Long, fixture: RealtimeLoopback, predicate: (JSONObject) -> Boolean): JSONObject {
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(5)
        while (System.nanoTime() < deadline) {
            fixture.checkFailure()
            val state = NativeRuntimeConfiguration.command("poll", handle)
            assertTrue("Loopback runtime failed: ${state.optString("errorCode")}", state.isNull("errorCode"))
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
                    val state = awaitRuntimeState(handle, fixture, predicate)
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
                    val state = awaitRuntimeState(handle, fixture, predicate)
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
        private val translationStartReplay: Boolean = false) : AutoCloseable {
        private val tencent = provider == ServiceProvider.TENCENT
        private val qwen = provider == ServiceProvider.DASHSCOPE && !liveTranslate
        private val host = if (liveTranslate) "synthetic-workspace.cn-beijing.maas.aliyuncs.com"
            else if (qwen) "dashscope.aliyuncs.com" else if (tencent) "asr.cloud.tencent.com" else "aip.baidubce.com"
        private val phases = JSONObject(requireNotNull(javaClass.getResourceAsStream("/translation-contracts.json"))
            .bufferedReader().use { it.readText() }).let { contract ->
            if (translationStartReplay) JSONObject(requireNotNull(javaClass.getResourceAsStream("/live-pair-contracts.json"))
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
                    if (liveTranslate) {
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
            check(request.startsWith(if (liveTranslate) "GET /api-ws/v1/realtime?model=qwen3.8-livetranslate-flash-realtime HTTP/1.1\r\n"
                else if (qwen) "GET /api-ws/v1/inference HTTP/1.1\r\n" else if (tencent) "GET /asr/speech_translate/123456789?" else "GET /ws/realtime_speech_trans HTTP/1.1\r\n"))
            val key = request.lineSequence().first { it.startsWith("sec-websocket-key:", true) }.substringAfter(':').trim()
            val accept = Base64.getEncoder().encodeToString(MessageDigest.getInstance("SHA-1").digest((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").toByteArray(Charsets.US_ASCII)))
            secure.outputStream.write("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: $accept\r\n\r\n".toByteArray(Charsets.US_ASCII))
            return secure
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
                    for (event in listOf(
                        """{"type":"conversation.item.input_audio_transcription.delta","item_id":"source","delta":"Synthetic finish tail."}""",
                        """{"type":"conversation.item.created","previous_item_id":"source","item":{"id":"out","role":"assistant"}}""",
                        """{"type":"response.text.done","item_id":"out","response_id":"response","text":"合成收尾字幕。"}""",
                        """{"type":"conversation.item.input_audio_transcription.completed","item_id":"source","transcript":"Synthetic finish tail."}""",
                        """{"type":"response.done","response":{"id":"response","status":"completed","output":[]}}""",
                        """{"type":"session.finished"}""",
                    )) send(output, event)
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
        private fun frame(output: OutputStream, opcode: Int, bytes: ByteArray) {
            require(bytes.size < 65_536)
            output.write(0x80 or opcode)
            if (bytes.size < 126) output.write(bytes.size) else {
                output.write(126); output.write(bytes.size shr 8); output.write(bytes.size and 255)
            }
            output.write(bytes)
            output.flush()
        }
        private fun readText(input: DataInputStream, output: OutputStream, connection: Int = 1): String {
            try {
                while (true) {
                    val opcode = input.readUnsignedByte() and 15
                    val flags = input.readUnsignedByte()
                    check(flags and 128 != 0)
                    val size = when (val short = flags and 127) { 126 -> input.readUnsignedShort(); 127 -> error("Oversized loopback frame"); else -> short }
                    check(size <= if (liveTranslate) 8_192 else 4_096)
                    val mask = ByteArray(4).also(input::readFully)
                    val payload = ByteArray(size).also(input::readFully)
                    for (index in payload.indices) payload[index] = (payload[index].toInt() xor mask[index % 4].toInt()).toByte()
                    if ((qwen || liveTranslate) && opcode == 8) {
                        if (liveTranslate) observeTransportClosed(connection)
                        return ""
                    }
                    if ((qwen || liveTranslate) && opcode == 2) continue
                    if (opcode == 9) frame(output, 10, payload) else {
                        check(opcode == 1) { "Unexpected loopback client frame" }
                        return payload.toString(Charsets.UTF_8)
                    }
                }
            } catch (error: java.io.EOFException) {
                if (!liveTranslate) throw error
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
