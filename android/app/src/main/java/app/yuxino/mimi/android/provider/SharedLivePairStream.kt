package app.yuxino.mimi.android.provider

import org.json.JSONObject

/** Decodes transport identity; pairing and retirement run in the shared Rust core. */
internal class SharedLivePairStream(private val listener: EngineListener) {
    private var state: String? = null
    private val startedAt = System.nanoTime()

    @Synchronized fun observe(event: JSONObject, frame: JSONObject = JSONObject()) {
        val itemId = frame.opt("item_id") as? String
            ?: frame.optJSONObject("item")?.opt("id") as? String
        val identity = JSONObject().put("item_id", itemId ?: JSONObject.NULL)
            .put("previous_item_id", (frame.opt("previous_item_id") as? String) ?: JSONObject.NULL)
        val operation = JSONObject().put("type", "dashscope").put("action", "observe")
            .put("event", event).put("identity", identity)
            .put("received_at_ns", (System.nanoTime() - startedAt).coerceAtLeast(0))
        val response = SharedSubtitleCore.exchange(JSONObject().put("state", state).put("operation", operation))
        state = response.getString("state")
        val events = response.getJSONArray("events")
        repeat(events.length()) { index ->
            val output = events.getJSONObject(index)
            val language = output.optString("language").takeIf { it.isNotBlank() && it != "null" }
            when (output.getString("type")) {
                "source_draft" -> listener.onSourceDraft(output.getString("text"), language)
                "source_final" -> listener.onSourceFinal(output.getString("text"), language)
                "translation_draft" -> listener.onTranslationDraft(output.getString("text"))
                "translation_final" -> listener.onTranslationFinal(output.getString("text"))
                "utterance_text" -> listener.onUtteranceText(output.getString("utterance_id"),
                    output.getString("role") == "source", output.getString("text"), output.getBoolean("is_final"), language)
                "final_pair" -> listener.onIdentifiedFinalPair(output.getString("utterance_id"), output.getString("source"), output.getString("translation"), language)
            }
        }
    }
    @Synchronized fun reset() { state = null }
}
