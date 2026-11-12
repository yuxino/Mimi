package app.yuxino.mimi.android.provider

import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test

class SharedTranscriptContractTest {
    @Test fun continuousLanguageAndSentenceBoundariesFollowSharedFixturesAcrossActualJni() {
        val cases = org.json.JSONObject(requireNotNull(javaClass.getResourceAsStream("/transcript-continuity-contracts.json"))
            .bufferedReader().use { it.readText() }).getJSONArray("cases")
        for (index in 0 until cases.length()) {
            val case = cases.getJSONObject(index)
            val stream = Stream(case.optString("mode", "openai"))
            val steps = case.getJSONArray("steps")
            for (stepIndex in 0 until steps.length()) {
                val step = steps.getJSONObject(stepIndex)
                when (step.getString("action")) {
                    "source", "translation" -> stream.append(step.getString("action") == "source", step.getString("delta"),
                        if (step.has("elapsed_ms")) step.getLong("elapsed_ms") else null)
                    "finish" -> stream.finish()
                    "settle" -> stream.settle(step.getLong("elapsed_ms"))
                    "reset" -> stream.reset()
                }
                val expected = step.getJSONArray("pairs")
                assertEquals("${case.getString("id")} step $stepIndex", (0 until expected.length()).map {
                    expected.getJSONArray(it).getString(0) to expected.getJSONArray(it).getString(1)
                }, stream.pairs)
                if (step.optBoolean("clear_previews")) {
                    assertEquals(case.getString("id"), 1, stream.sourceClears)
                    assertEquals(case.getString("id"), 1, stream.translationClears)
                }
            }
        }
    }
    /** Exercises the JSON JNI boundary directly, without a second transport engine. */
    private class Stream(private val mode: String = "openai") {
        private var state: String? = null
        val pairs = mutableListOf<Pair<String, String>>()
        var sourceClears = 0
        var translationClears = 0
        fun append(source: Boolean, delta: String, elapsedMs: Long?) = exchange(
            JSONObject().put("action", if (source) "source" else "translation")
                .put("delta", delta).put("elapsed_ms", elapsedMs ?: JSONObject.NULL))
        fun finish() = exchange(JSONObject().put("action", "finish"))
        fun settle(now: Long) = exchange(JSONObject().put("action", "settle").put("elapsed_ms", now))
        fun reset() { state = null }
        private fun exchange(operation: JSONObject) {
            val response = SharedSubtitleCore.exchange(JSONObject().put("state", state)
                .put("operation", operation.put("type", mode)))
            state = response.getString("state")
            val events = response.getJSONArray("events")
            repeat(events.length()) { index ->
                val event = events.getJSONObject(index)
                when (event.getString("type")) {
                    "source_draft" -> if (event.getJSONObject("payload").getString("text").isEmpty()) sourceClears++
                    "translation_draft" -> if (event.getString("payload").isEmpty()) translationClears++
                    "subtitle_final_pair" -> event.getJSONObject("payload").let {
                        pairs += it.getString("source") to it.getString("translation")
                    }
                    "error" -> fail(event.getJSONObject("payload").getString("code"))
                }
            }
        }
    }
    @Test fun differentSentencePunctuationDoesNotPairIndependentFinalsByArrivalOrder() {
        val stream = Stream()
        stream.append(true, "First sentence. Second sentence.", null)
        stream.append(false, "第一句、第二句。", null)
        assertTrue(stream.pairs.isEmpty())
        stream.finish()
        assertEquals(listOf("First sentence. Second sentence." to "第一句、第二句。"), stream.pairs)
    }
    @Test fun timingAlignedFinalKeepsBothCompleteTextsAndResetDropsUnmatchedOldTail() {
        val stream = Stream()
        stream.append(true, "Hello, world.", 1000)
        stream.append(false, "你好，世界。", 1000)
        assertEquals(listOf("Hello, world." to "你好，世界。"), stream.pairs)
        stream.append(true, "Old incomplete", null)
        stream.reset()
        stream.append(false, "新内容", null)
        stream.finish()
        assertEquals(1, stream.pairs.size)
    }
}
