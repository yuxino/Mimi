package app.yuxino.mimi.android.provider

import org.json.JSONObject

/** Thin transport adapter; alignment and complete text retention execute in shared Rust. */
internal class SharedTranscriptStream(private val listener: EngineListener) {
    private var state: String? = null
    @Synchronized fun append(source: Boolean, delta: String, elapsedMs: Long?) {
        exchange(JSONObject().put("type", "openai").put("action", if (source) "source" else "translation")
            .put("delta", delta).put("elapsed_ms", elapsedMs ?: JSONObject.NULL))
    }
    @Synchronized fun finish() { exchange(JSONObject().put("type", "openai").put("action", "finish")) }
    @Synchronized fun reset() { state = null }
    private fun exchange(operation: JSONObject) {
        val response = SharedSubtitleCore.exchange(JSONObject().put("state", state).put("operation", operation))
        state = response.getString("state")
        val events = response.getJSONArray("events")
        repeat(events.length()) { index ->
            val event = events.getJSONObject(index)
            when (event.getString("type")) {
                "source_draft" -> listener.onSourceDraft(event.getJSONObject("payload").getString("text"))
                "translation_draft" -> listener.onTranslationDraft(event.getString("payload"))
                "subtitle_final_pair" -> event.getJSONObject("payload").let {
                    listener.onFinalPair(it.getString("source"), it.getString("translation"))
                }
                "error" -> listener.onError(event.getJSONObject("payload").getString("code"), "服务字幕处理失败。")
            }
        }
    }
}
