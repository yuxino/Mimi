package app.yuxino.mimi.android.provider

import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test

class TencentContractTest {
    private val contract = requireNotNull(javaClass.getResourceAsStream("/translation-contracts.json"))
        .bufferedReader().use { JSONObject(it.readText()) }.getJSONObject("tencentSpeechTranslation")
    private fun protocol() = TencentProtocol(ServiceConfiguration(ServiceProvider.TENCENT,
        mapOf("appId" to "1250000000", "secretId" to "AKIDEXAMPLE", "secretKey" to "secret-key")), "en", "zh")

    @Test fun sharedSigningUsesTheDocumentedAppIdAndSecretPair() {
        val cases = contract.getJSONArray("signing")
        repeat(cases.length()) { index ->
            val case = cases.getJSONObject(index)
            val credentials = case.getJSONObject("credentials")
            val config = ServiceConfiguration(ServiceProvider.TENCENT, credentials.keys().asSequence().associateWith { credentials.getString(it) })
            val protocol = TencentProtocol(config, case.getString("source"), case.getString("target"),
                case.getLong("timestamp"), case.getLong("nonce"), case.getString("voiceId"))
            val expected = case.getJSONObject("expected")
            assertEquals(case.getString("id"), expected.getString("url"), protocol.request().url.toString().replaceFirst("https://", "wss://"))
            assertEquals(case.getLong("expired").toString(), protocol.request().url.queryParameter("expired"))
            assertEquals(expected.getInt("frameBytes"), protocol.frameBytes)
        }
    }

    @Test fun sharedEventsCommitFinalSourceAndTranslationAtomically() {
        val cases = contract.getJSONArray("events")
        repeat(cases.length()) { index ->
            val case = cases.getJSONObject(index)
            val actual = runCatching { normalize(protocol().text(case.getString("frame"))) }
            if (case.isNull("expected")) assertTrue(case.getString("id"), actual.isFailure)
            else assertTrue(case.getString("id"), case.getJSONObject("expected").similar(actual.getOrThrow()))
        }
    }

    @Test fun sharedErrorsPreserveActionableCategoriesWithoutProviderMessages() {
        val cases = contract.getJSONArray("errors")
        repeat(cases.length()) { index ->
            val case = cases.getJSONObject(index)
            val error = assertThrows(TencentProviderException::class.java) {
                protocol().text(JSONObject().put("code", case.getLong("code")).put("message", "private transcript or credential").toString())
            }
            assertEquals(case.getLong("code"), error.status)
            assertEquals(case.getString("expected"), error.category)
            assertEquals(case.getString("label"), error.errorCode)
            assertFalse(error.toString().contains("private"))
        }
    }

    @Test fun transcriptLimitsCountUtf8BytesLikeDesktop() {
        val result = JSONObject().put("source_text", "界".repeat(44_000)).put("target_text", "Synthetic translation.")
            .put("source", "zh").put("target", "en").put("sentence_end", false)
        assertThrows(IllegalArgumentException::class.java) { protocol().text(JSONObject().put("code", 0).put("result", result).toString()) }
    }

    private fun normalize(events: List<ServiceEvent>): JSONObject = when {
        events.isEmpty() -> obj("type" to "ignored")
        events == listOf(ServiceEvent.Ready) -> obj("type" to "ready")
        events == listOf(ServiceEvent.Closed) -> obj("type" to "finished")
        events.singleOrNull() is ServiceEvent.FinalPair -> (events.single() as ServiceEvent.FinalPair).let {
            obj("type" to "finalPair", "source" to it.source, "translation" to it.translation, "language" to requireNotNull(it.language))
        }
        else -> {
            assertEquals(2, events.size)
            val source = events[0] as ServiceEvent.Source
            val translation = events[1] as ServiceEvent.Translation
            assertFalse(source.final); assertFalse(translation.final)
            obj("type" to "draft", "source" to source.text, "translation" to translation.text, "language" to requireNotNull(source.language))
        }
    }
}
