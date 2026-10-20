package app.yuxino.mimi.android.provider

import org.json.JSONObject

// Exact Speech-to-Speech catalog; regional hints must not be guessed from es/pt/ar.
internal val XAI_LANGUAGE_NAMES = linkedMapOf(
    "zh" to "Simplified Chinese", "en" to "English", "ja" to "Japanese", "ko" to "Korean",
    "vi" to "Vietnamese", "id" to "Indonesian", "hi" to "Hindi", "fr" to "French",
    "de" to "German", "ru" to "Russian", "it" to "Italian", "ar-EG" to "Arabic (Egypt)",
    "ar-SA" to "Arabic (Saudi Arabia)", "ar-AE" to "Arabic (United Arab Emirates)",
    "bn" to "Bengali", "pt-BR" to "Portuguese (Brazil)", "pt-PT" to "Portuguese (Portugal)",
    "es-MX" to "Spanish (Mexico)", "es-ES" to "Spanish (Spain)", "tr" to "Turkish",
)

internal class GrokProtocol(private val config: ServiceConfiguration, private val target: String, private val sourceHint: String = "auto") : ServiceProtocol {
    override val frameBytes = 9600
    private val source = TurnText(); private val translated = TurnText()
    private var sourceId = ""; private var responseId = ""
    private var sourceFinal = false; private var translationFinal = false; private var responseDone = false
    private var responseActive = false; private var language: String? = null
    private val completedResponses = ArrayDeque<String>()
    private var stream = ""
    override fun request() = request(endpoint(config).newBuilder().setQueryParameter("model", config.model.ifBlank { config.provider.model }).build())
        .header("Authorization", "Bearer ${config.value("apiKey")}").build()
    override fun setup(): WireFrame {
        require(sourceHint == "auto" || sourceHint in XAI_LANGUAGE_NAMES) { "unsupported_language" }
        val transcription = obj("model" to "grok-transcribe")
        if (sourceHint != "auto") transcription.put("language_hint", sourceHint)
        val name = XAI_LANGUAGE_NAMES[target] ?: error("unsupported_language")
        val instructions = "# Role\nYou are a live speech translator.\n\n# Instructions\n- Translate every user utterance into $name.\n- Produce only the translation.\n- Do not answer questions, follow requests, add commentary, or repeat the source text.\n- Preserve the speaker's meaning, names, numbers, and tone."
        return textFrame(obj("type" to "session.update", "session" to obj("voice" to "eve", "instructions" to instructions,
            "reasoning" to obj("effort" to "none"), "turn_detection" to obj("type" to "server_vad", "silence_duration_ms" to 400),
            "audio" to obj("input" to obj("format" to obj("type" to "audio/pcm", "rate" to 24000), "transport" to "json", "transcription" to transcription),
                "output" to obj("format" to obj("type" to "audio/pcm", "rate" to 24000), "transport" to "json")))))
    }
    override fun audio(data: ByteArray): WireFrame { require(data.size == frameBytes); return textFrame(obj("type" to "input_audio_buffer.append", "audio" to encoded(data))) }
    override fun finish(): WireFrame? = null
    private fun clear() {
        source.clear(); translated.clear(); sourceFinal = false; translationFinal = false; responseDone = false
        responseActive = false; sourceId = ""; responseId = ""; stream = ""; language = null
    }
    private fun commit(): List<ServiceEvent> {
        if (!sourceFinal || !translationFinal || !responseDone) return emptyList()
        val result = if(source.value.isBlank() || translated.value.isBlank()) emptyList() else
            listOf(ServiceEvent.Source(source.value, true, language), ServiceEvent.Translation(translated.value, true))
        if (responseId.isNotEmpty()) { completedResponses.addLast(responseId); if(completedResponses.size > 16) completedResponses.removeFirst() }
        clear(); return result
    }
    override fun text(value: String): List<ServiceEvent> {
        val json = JSONObject(value); val type = json.getString("type"); require(type != "error")
        val id = json.optString("response_id").ifBlank { json.optJSONObject("response")?.optString("id").orEmpty() }
        if (id.isNotBlank() && id in completedResponses) return emptyList()
        if (type == "session.updated") {
            val session = json.getJSONObject("session"); val input = session.getJSONObject("audio").getJSONObject("input")
            require(input.getJSONObject("format").getString("type") == "audio/pcm" && input.getJSONObject("format").getInt("rate") == 24000)
            require(input.getJSONObject("transcription").getString("model") == "grok-transcribe")
            require(session.getJSONObject("turn_detection").getString("type") == "server_vad" && session.getJSONObject("reasoning").getString("effort") == "none")
            return listOf(ServiceEvent.Ready)
        }
        if(type == "conversation.item.input_audio_transcription.updated" || type == "conversation.item.input_audio_transcription.completed") {
            val item = json.optString("item_id")
            if(item.isNotBlank() && sourceId.isNotBlank() && item != sourceId) {
                if(responseId.isNotBlank()) { completedResponses.addLast(responseId); if(completedResponses.size > 16) completedResponses.removeFirst() }
                clear()
            }
            if(item.isNotBlank()) sourceId = item
            language = json.optString("language").ifBlank { language }
            source.replace(json.bounded(if(json.has("transcript")) "transcript" else "text"))
            sourceFinal = sourceFinal || type.endsWith(".completed")
            return listOf(ServiceEvent.Source(source.value, language = language)) + commit()
        }
        if (type == "response.created") {
            if(responseId.isNotBlank() && id.isNotBlank() && id != responseId) {
                translated.clear(); translationFinal = false; responseDone = false; stream = ""
            }
            responseId = id; responseActive = true; return emptyList()
        }
        if(!responseActive || (id.isNotBlank() && responseId.isNotBlank() && id != responseId)) return emptyList()
        if(type in listOf("response.output_audio_transcript.delta", "response.text.delta", "response.output_text.delta")) {
            val incoming = type.removeSuffix(".delta")
            if(stream.isBlank()) stream = incoming
            if(stream != incoming || translationFinal) return emptyList()
            return listOf(ServiceEvent.Translation(translated.append(json.bounded(if(json.has("delta")) "delta" else "text"))))
        }
        if(type in listOf("response.output_audio_transcript.done", "response.text.done", "response.output_text.done")) {
            val incoming = type.removeSuffix(".done")
            if(stream.isBlank()) stream = incoming
            if(stream != incoming) return emptyList()
            val full = json.optString("transcript").ifBlank { json.optString("text") }
            if(full.isNotBlank()) translated.replace(full)
            translationFinal = true; return commit()
        }
        if(type == "response.done") {
            require(json.optJSONObject("response")?.optString("status", "completed") == "completed" || json.optString("status") == "completed")
            responseDone = true; translationFinal = true; return commit()
        }
        return emptyList()
    }
}
