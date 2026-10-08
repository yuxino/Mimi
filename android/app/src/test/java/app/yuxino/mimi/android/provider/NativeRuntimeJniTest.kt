package app.yuxino.mimi.android.provider

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import java.security.KeyStore
import java.util.Base64
import javax.net.ssl.TrustManagerFactory
import javax.net.ssl.X509TrustManager

/** Actual JNI configuration/lifecycle checks. Never start, connect, capture or submit audio. */
class NativeRuntimeJniTest {
    @Test fun stopWatchdogIncludesNativeDrainDisconnectAndSnapshotPublication() {
        val policy = NativeRuntimeConfiguration.policy
        assertEquals(60L, policy.getLong("snapshotPublishIntervalMs"))
        val providerFinish = SharedSubtitleCore.policy.getLong("provider_finish_timeout_ms")
        assertTrue(policy.getLong("sessionFinishTimeoutMs") > providerFinish + 1_000L + 2_000L)
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

    @Test fun allSavedProviderCredentialShapesCrossTheActualNativeBoundaryWithoutConnecting() {
        for (provider in ServiceProvider.entries) {
            val config = configuration(provider)
            val handle = create(config)
            try {
                val state = NativeRuntimeConfiguration.command("poll", handle)
                assertTrue(state.has("snapshot"))
                assertFalse(state.getBoolean("finished"))
                assertFalse(state.toString().contains("synthetic-apiKey"))
                assertFalse(state.toString().contains("synthetic-secretKey"))
                assertFalse(state.toString().contains("synthetic-appKey"))
                assertFalse(state.has("configuration"))
            } finally { NativeRuntimeConfiguration.command("stop", handle) }
        }
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
        val text = TranslationConfiguration(provider = TextTranslationProvider.OPENAI_COMPATIBLE,
            endpoint = "https://synthetic.example.invalid/v1", model = "synthetic-model", apiKey = "synthetic-text-key")
        val speech = ServiceConfiguration(ServiceProvider.DASHSCOPE, mapOf("apiKey" to "synthetic-speech-key"))
        val independent = buildRuntimeConfiguration(speech, text, "auto", "zh", "plus")
        val endpoints = NativeRuntimeConfiguration.endpoints(independent)
        assertEquals(builtIn.getString("speechEndpoint"), endpoints.getString("speechEndpoint"))
        assertEquals("https://synthetic.example.invalid/v1/chat/completions", endpoints.getString("textEndpoint"))
        val handle = create(independent)
        try { assertFalse(NativeRuntimeConfiguration.command("poll", handle).toString().contains("synthetic-text-key")) }
        finally { NativeRuntimeConfiguration.command("stop", handle) }
        val original = buildRuntimeConfiguration(speech, TranslationConfiguration(provider = TextTranslationProvider.NONE), "nl", "zh", "lite")
        assertTrue(NativeRuntimeConfiguration.endpoints(original).isNull("textEndpoint"))
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
}
