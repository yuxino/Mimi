package app.yuxino.mimi.android.provider

import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import java.util.concurrent.Executor

class NativeTextProbeClientTest {
    private val text = TranslationConfiguration(provider = TextTranslationProvider.CHAT_MOCK,
        endpoint = "http://10.0.2.2:8000/v1", model = "synthetic-model", apiKey = "synthetic-text-key", allowLocalHttp = true)
    private fun configuration() = buildTextProbeConfiguration(text, "auto", "zh", "flash")
    private fun diagnostic(service: String = "available", reason: String? = null) = JSONObject()
        .put("credential", "present").put("service", service).put("reason", reason ?: JSONObject.NULL).put("elapsedMs", 321)
    private class QueueExecutor : Executor {
        lateinit var task: Runnable
        override fun execute(command: Runnable) { task = command }
    }

    @Test fun textProbeConfigurationContainsOnlyIndependentTextCredentials() {
        val configuration = configuration()
        assertEquals(setOf("credentials", "sourceLanguage", "targetLanguage", "qwenMtModel"), configuration.keys().asSequence().toSet())
        assertEquals("chatMock", configuration.getJSONObject("credentials").getString("kind"))
        assertFalse(configuration.toString().contains("asrApiKey"))
        assertFalse(configuration.toString().contains("speech"))
        assertThrows(IllegalArgumentException::class.java) { buildTextProbeConfiguration(text.copy(allowLocalHttp = false), "auto", "zh", "flash") }
    }

    @Test fun diagnosticUsesSharedDurationAndFixedContentFreeReasons() {
        assertEquals(TranslationResult.Success("", 321), nativeTextProbeResult(diagnostic()))
        for ((reason, code) in listOf("authenticationRejected" to "translation_http_401", "quotaExhausted" to "translation_http_429",
            "concurrencyLimited" to "translation_http_429", "timeout" to "translation_timeout", "serviceRejected" to "translation_network",
            "synthetic-secret-response" to "translation_network")) {
            assertEquals(TranslationResult.Failure(code, 321), nativeTextProbeResult(diagnostic("unavailable", reason)))
        }
    }

    @Test fun completedProbePollsAndReleasesOnlyItsNativeHandle() {
        val operations = mutableListOf<String>()
        val network = JSONObject().put("synthetic", true)
        val client = NativeTextProbeClient(exchange = { request ->
            val operation = request.getString("operation"); operations += operation
            when (operation) {
                "create_text_probe" -> { assertSame(network, request.getJSONObject("network")); JSONObject().put("handle", 42) }
                "text_probe_poll" -> { assertEquals(42, request.getLong("handle")); JSONObject().put("finished", true).put("result", diagnostic()) }
                "text_probe_cancel" -> { assertEquals(42, request.getLong("handle")); JSONObject() }
                else -> error("unexpected operation")
            }
        }, networkSnapshot = { network }, executor = Executor { it.run() })
        val results = mutableListOf<TranslationResult>()
        client.check(configuration(), results::add)
        assertEquals(listOf(TranslationResult.Success("", 321)), results)
        assertEquals(listOf("create_text_probe", "text_probe_poll", "text_probe_cancel"), operations)
    }

    @Test fun cancelledQueuedDraftDoesNotCreateAnyNativeProbe() {
        val executor = QueueExecutor()
        val client = NativeTextProbeClient(exchange = { fail("Cancelled draft crossed JNI"); JSONObject() },
            networkSnapshot = { fail("Cancelled draft read routing"); JSONObject() }, executor = executor)
        val call = client.check(configuration()) { fail("Cancelled draft delivered a result") }
        call.cancel(); executor.task.run()
    }

    @Test fun cancellationDuringCreateReleasesTheCreatedHandleWithoutPolling() {
        val executor = QueueExecutor()
        val operations = mutableListOf<String>()
        lateinit var call: TranslationCall
        val client = NativeTextProbeClient(exchange = { request ->
            val operation = request.getString("operation"); operations += operation
            when (operation) {
                "create_text_probe" -> { call.cancel(); JSONObject().put("handle", 42) }
                "text_probe_cancel" -> JSONObject()
                else -> { fail("Cancelled create was polled"); JSONObject() }
            }
        }, networkSnapshot = { JSONObject() }, executor = executor)
        call = client.check(configuration()) { fail("Cancelled create delivered a result") }
        executor.task.run()
        assertEquals(listOf("create_text_probe", "text_probe_cancel"), operations)
    }

    @Test fun adapterFailuresNeverRevealExceptionsOrCredentials() {
        val results = mutableListOf<TranslationResult>()
        val client = NativeTextProbeClient(exchange = { throw IllegalArgumentException("synthetic-private-key") },
            networkSnapshot = { JSONObject() }, executor = Executor { it.run() })
        client.check(configuration(), results::add)
        val result = results.single() as TranslationResult.Failure
        assertEquals("translation_network", result.code)
        assertFalse(result.toString().contains("synthetic"))
    }

    @Test fun lateCompletedResultAfterCancellationCannotUpdateTheDraft() {
        val executor = QueueExecutor()
        lateinit var call: TranslationCall
        var cancellations = 0
        val client = NativeTextProbeClient(exchange = { request ->
            when (request.getString("operation")) {
                "create_text_probe" -> JSONObject().put("handle", 42)
                "text_probe_poll" -> { call.cancel(); JSONObject().put("finished", true).put("result", diagnostic()) }
                "text_probe_cancel" -> { cancellations++; JSONObject() }
                else -> error("unexpected operation")
            }
        }, networkSnapshot = { JSONObject() }, executor = executor)
        call = client.check(configuration()) { fail("Cancelled probe delivered a late result") }
        executor.task.run()
        assertTrue(cancellations >= 1)
    }
}
