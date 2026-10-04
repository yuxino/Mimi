package app.yuxino.mimi.android.provider

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test

/** The exact desktop contract also crosses the real JNI string/state boundary. */
class SharedSubtitleContractTest {
    private fun pair(value: JSONObject?): Any = value?.let {
        JSONObject().put("source", it.getString("source")).put("translation", it.getString("translation"))
    } ?: JSONObject.NULL

    private fun projection(snapshot: JSONObject): JSONObject {
        fun line(name: String): JSONObject = snapshot.getJSONObject(name).let {
            JSONObject().put("text", it.getString("text")).put("isFinal", it.getBoolean("isFinal"))
        }
        val history = JSONArray()
        val entries = snapshot.getJSONArray("history")
        for (index in 0 until entries.length()) {
            val entry = entries.getJSONObject(index)
            history.put(JSONObject().put("audioSource", entry.getString("audioSource"))
                .put("source", entry.getString("source")).put("translation", entry.getString("translation")))
        }
        return JSONObject().put("source", line("source")).put("translation", line("translation"))
            .put("previewPair", pair(snapshot.optJSONObject("previewPair")))
            .put("displayPair", pair(snapshot.optJSONObject("displayPair"))).put("history", history)
    }

    @Test fun sharedSyntheticCasesExecuteTheSameRustReducerThroughJni() {
        val contract = requireNotNull(javaClass.getResourceAsStream("/subtitle-contracts.json")) {
            "Missing shared subtitle contract resource"
        }.bufferedReader().use { JSONObject(it.readText()) }
        assertEquals(1, contract.getInt("schemaVersion"))
        val cases = contract.getJSONArray("cases")
        assertTrue(cases.length() >= 10)
        var checked = 0
        for (index in 0 until cases.length()) {
            val case = cases.getJSONObject(index)
            val source = case.getString("audioSource")
            // Android captures system playback only. The microphone lane
            // fixture is exercised directly by the shared Rust core tests.
            if (source == "microphone") continue
            assertEquals("Unexpected unsupported audio source fixture", "system", source)
            var response = SharedSubtitleCore.exchange(JSONObject().put("operation",
                JSONObject().put("type", "create").put("history_limit", case.getInt("maxHistoryCount"))))
            val steps = case.getJSONArray("steps")
            for (stepIndex in 0 until steps.length()) {
                val step = steps.getJSONObject(stepIndex)
                val operation = when {
                    step.has("event") -> JSONObject().put("type", "apply").put("event", step.getJSONObject("event"))
                    step.optBoolean("resetTransient") -> JSONObject().put("type", "reset")
                    step.optBoolean("beginSession") -> JSONObject().put("type", "begin_session")
                    step.has("historyLimit") -> JSONObject().put("type", "history_limit").put("limit", step.getInt("historyLimit"))
                    else -> error("Unknown shared subtitle contract operation")
                }
                response = SharedSubtitleCore.exchange(JSONObject().put("state", response.getString("state")).put("operation", operation))
                assertTrue("${case.getString("id")} step $stepIndex",
                    step.getJSONObject("expected").similar(projection(response.getJSONObject("snapshot"))))
                if (step.has("displayPairFinal")) {
                    assertEquals(step.getBoolean("displayPairFinal"), response.getJSONObject("snapshot").getBoolean("displayPairFinal"))
                }
                checked += 1
            }
        }
        assertTrue("No meaningful shared JNI subtitle cases were executed", checked >= 45)
    }
}
