package app.yuxino.mimi.android.provider

import org.json.JSONObject
import org.json.JSONArray
import org.junit.After
import org.junit.Before
import org.junit.Assert.*
import org.junit.Test

class SharedLivePairContractTest {
    @Before fun prepareBus() {
        SubtitleBus.clear()
        SubtitleBus.setHistoryLimit(6)
    }

    @After fun resetBus() {
        SubtitleBus.clear()
        SubtitleBus.setHistoryLimit(0)
    }

    @Test fun providerIdentityContractsCrossRealJniWithOpaqueStateRoundtrips() {
        val contract = requireNotNull(javaClass.getResourceAsStream("/live-pair-contracts.json"))
            .bufferedReader().use { JSONObject(it.readText()) }
        val cases = contract.getJSONArray("cases")
        var checked = 0
        repeat(cases.length()) { caseIndex ->
            val case = cases.getJSONObject(caseIndex)
            var state: String? = null
            var reducer = SharedSubtitleCore.exchange(JSONObject().put("operation",
                JSONObject().put("type", "create").put("history_limit", 6)))
            val steps = case.getJSONArray("steps")
            repeat(steps.length()) { index ->
                val step = steps.getJSONObject(index)
                val operation = JSONObject().put("type", "dashscope")
                val events: JSONArray
                if (step.optBoolean("resetStream")) {
                    state = null
                    reducer = reducerOperation(reducer, JSONObject().put("type", "reset"))
                    events = JSONArray()
                } else {
                    if (step.optBoolean("clearContent")) {
                        operation.put("action", "clear")
                        reducer = applyEvent(reducer, JSONObject().put("type", "clear"))
                    } else operation.put("action", "observe").put("event", step.getJSONObject("event"))
                        .put("identity", step.getJSONObject("identity")).put("received_at_ns", step.getLong("atNs"))
                    val response = SharedSubtitleCore.exchange(JSONObject().put("state", state).put("operation", operation))
                    state = response.getString("state")
                    events = response.getJSONArray("events")
                }
                assertTrue("${case.getString("id")} step $index",
                    step.getJSONArray("expected").similar(events))
                repeat(events.length()) { eventIndex ->
                    val event = JSONObject(events.getJSONObject(eventIndex).toString())
                    // Listener metadata is outside the strict subtitle-event schema.
                    event.remove("language")
                    event.remove("follow_latency_ms")
                    when (event.getString("type")) {
                        "item_created", "session_finished", "passthrough" -> Unit
                        else -> {
                            // The transport carries its source identity to the reducer.
                            if (event.getString("type") == "final_pair") event.put("type", "identified_final_pair")
                            reducer = applyEvent(reducer, event)
                        }
                    }
                }
                if (step.has("expectedReducer")) assertTrue("${case.getString("id")} step $index reducer",
                    step.getJSONObject("expectedReducer").similar(reducerProjection(reducer.getJSONObject("snapshot"))))
                checked++
            }
        }
        assertTrue(checked >= 64)
    }

    private fun reducerOperation(previous: JSONObject, operation: JSONObject) =
        SharedSubtitleCore.exchange(JSONObject().put("state", previous.getString("state")).put("operation", operation))

    private fun applyEvent(previous: JSONObject, event: JSONObject) =
        reducerOperation(previous, JSONObject().put("type", "apply").put("event", event))

    private fun reducerProjection(snapshot: JSONObject): JSONObject {
        val pair = snapshot.optJSONObject("displayPair")?.let {
            JSONObject().put("utteranceId", it.opt("utteranceId") ?: JSONObject.NULL)
                .put("source", it.getString("source")).put("translation", it.getString("translation"))
        }
        val history = JSONArray()
        val rows = snapshot.getJSONArray("history")
        repeat(rows.length()) { index ->
            val row = rows.getJSONObject(index)
            history.put(JSONObject().put("source", row.getString("source")).put("translation", row.getString("translation")))
        }
        return JSONObject().put("source", snapshot.getJSONObject("source"))
            .put("translation", snapshot.getJSONObject("translation"))
            .put("displayPair", pair ?: JSONObject.NULL).put("history", history)
    }

    @Test fun androidTransportDoesNotPairKnownUnlinkedResponseByArrivalOrder() {
        val pairs = mutableListOf<Pair<String, String>>()
        val identities = mutableListOf<String>()
        val listener = object : EngineListener {
            override fun onSessionReady() = Unit
            override fun onSourceDraft(text: String, language: String?) = Unit
            override fun onSourceFinal(text: String, language: String?) = Unit
            override fun onTranslationDraft(text: String) = Unit
            override fun onTranslationFinal(text: String) = fail("Identified response lost its identity")
            override fun onUtteranceText(id: String, source: Boolean, text: String, final: Boolean, language: String?) { identities += id }
            override fun onFinalPair(source: String, translation: String, language: String?) { pairs += source to translation }
            override fun onError(code: String, message: String) = fail(code)
            override fun onClosed() = Unit
            override fun onLog(message: String) = Unit
        }
        val engine = DashScopeEngine(listener)
        engine.receiveServerMessage("""{"type":"conversation.item.input_audio_transcription.completed","item_id":"A","transcript":"First sentence."}""")
        engine.receiveServerMessage("""{"type":"conversation.item.input_audio_transcription.completed","item_id":"B","transcript":"Second sentence."}""")
        engine.receiveServerMessage("""{"type":"response.text.done","item_id":"unknown","text":"不能猜到第二句"}""")
        assertTrue(pairs.isEmpty())
        engine.receiveServerMessage("""{"type":"conversation.item.created","item":{"id":"response-A"},"previous_item_id":"A"}""")
        engine.receiveServerMessage("""{"type":"response.text.done","item_id":"response-A","text":"第一句。"}""")
        assertEquals(listOf("First sentence." to "第一句。"), pairs)
        assertEquals(listOf("A", "B"), identities)
        engine.stop()
    }

    @Test fun lateLinkedFinalCommitsHistoryWithoutReplacingTheNewerDisplayedUtterance() {
        val engine = DashScopeEngine(busListener())
        try {
            sourceFinal(engine, "A", "First sentence.", "en")
            sourceFinal(engine, "B", "Second sentence.", "ko")
            assertEquals("Second sentence.", SubtitleBus.sourceFinal)
            assertEquals("ko", SubtitleBus.detectedSourceLanguage)

            linkedFinal(engine, "B", "response-B", "第二句。")
            assertEquals("Second sentence.", SubtitleBus.displaySource)
            assertEquals("第二句。", SubtitleBus.displayTranslation)
            assertEquals("ko", SubtitleBus.detectedSourceLanguage)
            assertEquals(listOf(SubtitleBus.Pair("Second sentence.", "第二句。")), SubtitleBus.historySnapshot())

            linkedFinal(engine, "A", "response-A", "第一句。")
            assertEquals("Second sentence.", SubtitleBus.sourceFinal)
            assertEquals("第二句。", SubtitleBus.translationFinal)
            assertEquals("Second sentence.", SubtitleBus.displaySource)
            assertEquals("第二句。", SubtitleBus.displayTranslation)
            assertEquals("ko", SubtitleBus.detectedSourceLanguage)
            assertEquals(listOf(
                SubtitleBus.Pair("Second sentence.", "第二句。"),
                SubtitleBus.Pair("First sentence.", "第一句。"),
            ), SubtitleBus.historySnapshot())
        } finally {
            engine.stop()
        }
    }

    @Test fun identicalFinalTextFromDistinctSourceIdsKeepsBothAndRetransmissionKeepsOnlyTwo() {
        val engine = DashScopeEngine(busListener())
        try {
            // Consecutive server finals with no draft must retain provider identity.
            sourceFinal(engine, "A", "Thank you.")
            linkedFinal(engine, "A", "response-A", "谢谢。")
            sourceFinal(engine, "B", "Thank you.")
            linkedFinal(engine, "B", "response-B", "谢谢。")
            val repeated = listOf(
                SubtitleBus.Pair("Thank you.", "谢谢。"),
                SubtitleBus.Pair("Thank you.", "谢谢。"),
            )
            assertEquals(repeated, SubtitleBus.historySnapshot())
            assertEquals("Thank you.", SubtitleBus.displaySource)
            assertEquals("谢谢。", SubtitleBus.displayTranslation)

            sourceFinal(engine, "B", "Thank you.")
            linkedFinal(engine, "B", "response-B", "谢谢。")
            assertEquals(repeated, SubtitleBus.historySnapshot())
        } finally {
            engine.stop()
        }
    }

    private fun sourceFinal(engine: DashScopeEngine, id: String, text: String, language: String? = null) {
        engine.receiveServerMessage(JSONObject().put("type", "conversation.item.input_audio_transcription.completed")
            .put("item_id", id).put("transcript", text).put("language", language ?: JSONObject.NULL).toString())
    }

    private fun linkedFinal(engine: DashScopeEngine, sourceId: String, responseId: String, text: String) {
        engine.receiveServerMessage(JSONObject().put("type", "conversation.item.created")
            .put("item", JSONObject().put("id", responseId)).put("previous_item_id", sourceId).toString())
        engine.receiveServerMessage(JSONObject().put("type", "response.text.done")
            .put("item_id", responseId).put("text", text).toString())
    }

    /** Route the same identity-bearing callbacks used by MimiService into its bus. */
    private fun busListener() = object : EngineListener {
        override fun onSessionReady() = Unit
        override fun onSourceDraft(text: String, language: String?) = SubtitleBus.onSourceDraft(text, language)
        override fun onSourceFinal(text: String, language: String?) = SubtitleBus.onSourceFinal(text, language)
        override fun onTranslationDraft(text: String) = SubtitleBus.onTranslationDraft(text)
        override fun onTranslationFinal(text: String) = fail("Linked final lost its source identity")
        override fun onUtteranceText(id: String, source: Boolean, text: String, final: Boolean, language: String?) {
            SubtitleBus.onCoreEvent(JSONObject().put("type", "utterance_text").put("utterance_id", id)
                .put("role", if (source) "source" else "translation").put("text", text).put("is_final", final), language)
        }
        override fun onFinalPair(source: String, translation: String, language: String?) =
            fail("Linked pair lost its source identity")
        override fun onIdentifiedFinalPair(id: String, source: String, translation: String, language: String?) =
            SubtitleBus.onIdentifiedFinalPair(id, source, translation, language)
        override fun onError(code: String, message: String) = fail(code)
        override fun onClosed() = Unit
        override fun onLog(message: String) = Unit
    }
}
