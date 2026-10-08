package app.yuxino.mimi.android.provider

import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test

class SharedRealtimeDisplayContractTest {
    @Test fun sharedRealtimeDisplayCasesCrossActualJni() {
        val fixture = requireNotNull(javaClass.getResourceAsStream("/realtime-display-contracts.json"))
            .bufferedReader().use { JSONObject(it.readText()) }
        val cases = fixture.getJSONArray("cases")
        for (index in 0 until cases.length()) {
            val case = cases.getJSONObject(index)
            var response = SharedSubtitleCore.exchange(JSONObject().put("operation",
                JSONObject().put("type", "create").put("history_limit", case.getInt("historyLimit"))))
            val steps = case.getJSONArray("steps")
            for (stepIndex in 0 until steps.length()) {
                val step = steps.getJSONObject(stepIndex)
                response = SharedSubtitleCore.exchange(JSONObject().put("state", response.getString("state"))
                    .put("operation", step.getJSONObject("operation")))
                val snapshot = response.getJSONObject("snapshot")
                val preview = snapshot.optJSONObject("realtimePreview")
                val pair = snapshot.optJSONObject("displayPair")
                fun line(role: String): String = preview?.getJSONObject(role)?.getString("text")
                    ?: pair?.getString(role) ?: snapshot.getJSONObject(role).getString("text")
                val expected = step.getJSONObject("expected")
                val label = "${case.getString("id")} step $stepIndex"
                assertEquals(label, expected.getString("source"), line("source"))
                assertEquals(label, expected.getString("translation"), line("translation"))
                assertEquals(label, expected.getBoolean("confirmed"), preview == null && snapshot.getBoolean("displayPairFinal"))
                assertEquals(label, expected.getInt("historyCount"), snapshot.getJSONArray("history").length())
            }
        }
    }
}
