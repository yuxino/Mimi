package app.yuxino.mimi.android.provider

import org.json.JSONObject
import org.json.JSONArray
import org.junit.Assert.*
import org.junit.Test

class SharedLivePairContractTest {
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
                if (step.optBoolean("legacySourceOrder")) {
                    state = JSONObject(requireNotNull(state)).apply { remove("source_order") }.toString()
                    events = JSONArray()
                } else if (step.optBoolean("resetStream")) {
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

}
