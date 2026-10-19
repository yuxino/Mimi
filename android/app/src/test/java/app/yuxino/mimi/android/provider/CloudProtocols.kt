package app.yuxino.mimi.android.provider

import okhttp3.HttpUrl.Companion.toHttpUrl
import okhttp3.Request
import org.json.JSONObject
import org.json.JSONArray
import java.util.Base64
import java.util.UUID
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

internal fun obj(vararg pairs: Pair<String, Any>): JSONObject = JSONObject().apply { pairs.forEach { put(it.first, it.second) } }
internal fun textFrame(value: JSONObject) = WireFrame.Text(value.toString())
internal fun encoded(data: ByteArray) = Base64.getEncoder().encodeToString(data)
internal fun JSONObject.bounded(key: String): String = getString(key).also { require(it.length <= 128 * 1024) }
internal fun JSONObject.transcriptText(): String {
    if (!has("text")) return ""
    require(get("text") is String)
    return bounded("text")
}
internal fun endpoint(config: ServiceConfiguration): okhttp3.HttpUrl {
    val url = normalizeWebSocketUrl(config.endpoint.ifBlank { config.provider.endpoint })
    require(url.startsWith("wss://"))
    return url.replaceFirst("wss://", "https://").toHttpUrl().also {
        require(it.username.isEmpty() && it.password.isEmpty() && it.fragment == null)
    }
}
internal fun request(url: okhttp3.HttpUrl) = Request.Builder().url(url.toString().replaceFirst("https://", "wss://"))

internal class TencentProviderException(val status: Long) : IllegalArgumentException("tencent_provider_rejected") {
    val category: String get() = when (status) {
        6001L -> "configuration"
        6002L -> "authentication"
        6003L -> "activation"
        6004L, 6005L -> "quota"
        6006L -> "capacity"
        else -> "rejected"
    }
    val errorCode: String get() = when (category) {
        "configuration" -> "tencent_configuration_rejected"
        "authentication" -> CREDENTIAL_AUTHENTICATION_FAILED
        "activation" -> "tencent_service_activation_required"
        "quota" -> "tencent_quota_exhausted"
        "capacity" -> "tencent_capacity_exceeded"
        else -> "tencent_provider_rejected"
    }
}

internal class TencentProtocol(config: ServiceConfiguration, private val source: String, private val target: String,
    timestamp: Long = System.currentTimeMillis() / 1000, nonce: Long = (UUID.randomUUID().mostSignificantBits and Long.MAX_VALUE) % 9999999999L + 1,
    voiceId: String = UUID.randomUUID().toString()) : ServiceProtocol {
    override val frameBytes = 6400
    private val signedRequest: Request
    init {
        val appId = config.value("appId").trim(); val secretId = config.value("secretId").trim()
        val secretKey = config.value("secretKey").trim()
        require(appId.matches(Regex("[0-9]+")) && secretId.matches(Regex("[a-zA-Z0-9_.~-]+")))
        require(voiceId.matches(Regex("[a-zA-Z0-9_.~-]{1,128}")) && nonce in 1..9999999999L)
        require(secretKey.isNotEmpty() && timestamp in 0..(Long.MAX_VALUE - 3600))
        val wireSource = if (source == "auto") "zh_en" else source
        require(target in TENCENT_LANGUAGE_PAIRS[wireSource].orEmpty()) { "unsupported_language" }
        val values = sortedMapOf("expired" to "${timestamp + 3600}", "nonce" to "$nonce", "secretid" to secretId,
            "source" to wireSource, "target" to target, "timestamp" to "$timestamp", "trans_model" to "hunyuan-translation-lite",
            "voice_format" to "1", "voice_id" to voiceId)
        val path = "asr.cloud.tencent.com/asr/speech_translate/$appId"
        val canonical = path + "?" + values.entries.joinToString("&") { "${it.key}=${it.value}" }
        val mac = Mac.getInstance("HmacSHA1").apply { init(SecretKeySpec(secretKey.toByteArray(), "HmacSHA1")) }
        val url = "https://$path".toHttpUrl().newBuilder()
        values.forEach { (k,v) -> url.addQueryParameter(k,v) }
        url.addQueryParameter("signature", encoded(mac.doFinal(canonical.toByteArray())))
        signedRequest = request(url.build()).build()
    }
    override fun request() = signedRequest
    override fun setup(): WireFrame? = null
    override fun audio(data: ByteArray): WireFrame { require(data.size == frameBytes); return WireFrame.Binary(data) }
    override fun finish() = textFrame(obj("type" to "end"))
    override fun text(value: String): List<ServiceEvent> {
        require(value.toByteArray(Charsets.UTF_8).size <= 256 * 1024)
        val json = JSONObject(value)
        val code = when (val status = json.get("code")) {
            is Int -> status.toLong()
            is Long -> status
            else -> error("invalid_tencent_status")
        }
        if (code != 0L) throw TencentProviderException(code)
        val finalValue = when (val value = json.opt("final")) {
            is Int -> value.toLong().takeIf { it >= 0 }
            is Long -> value.takeIf { it >= 0 }
            else -> null
        }
        if (finalValue == 1L) return listOf(ServiceEvent.Closed)
        // Tencent's authenticated startup acknowledgement carries final=0.
        // Result frames also carry zero and must continue through transcript parsing.
        if (!json.has("result")) return if (finalValue == null || finalValue == 0L) listOf(ServiceEvent.Ready) else emptyList()
        val result = json.getJSONObject("result")
        val sourceText = transcript(result, "source_text")
        val targetText = transcript(result, "target_text")
        val sourceLanguage = language(result, "source")
        language(result, "target")
        val final = result.get("sentence_end").also { require(it is Boolean) } as Boolean
        return if (final) {
            if (sourceText.isBlank() || targetText.isBlank()) emptyList()
            else listOf(ServiceEvent.FinalPair(sourceText.trim(), targetText.trim(), sourceLanguage))
        } else listOf(ServiceEvent.Source(sourceText, language = sourceLanguage), ServiceEvent.Translation(targetText))
    }
    private fun transcript(result: JSONObject, field: String): String = (result.get(field).also { require(it is String) } as String)
        .also { require(it.toByteArray(Charsets.UTF_8).size <= 128 * 1024) }
    private fun language(result: JSONObject, field: String): String = (result.get(field).also { require(it is String) } as String)
        .also { require(it.matches(Regex("[a-zA-Z0-9_-]{1,16}"))) }
}

internal class BaiduProtocol(private val config: ServiceConfiguration, private val source: String, private val target: String) : ServiceProtocol {
    override val frameBytes = 1280
    override fun request() = Request.Builder().url("wss://aip.baidubce.com/ws/realtime_speech_trans").build()
    private fun language(code: String) = requireNotNull(BAIDU_LANGUAGE_CODES[code]) { "unsupported_language" }
    override fun setup() = textFrame(obj("type" to "START", "from" to language(source), "to" to language(target),
        "app_id" to config.value("appId"), "app_key" to config.value("appKey"), "sampling_rate" to 16000))
    override fun audio(data: ByteArray): WireFrame { require(data.size == frameBytes); return WireFrame.Binary(data) }
    override fun finish() = textFrame(obj("type" to "FINISH"))
    override fun text(value: String): List<ServiceEvent> {
        val json = JSONObject(value); require(json.getInt("code") == 0)
        val data = json.getJSONObject("data")
        return when(data.getString("status")) {
            "STA" -> listOf(ServiceEvent.Ready)
            "END" -> listOf(ServiceEvent.Closed)
            "TRN" -> {
                val result = data.getJSONObject("result"); val type = result.getString("type")
                if (type !in listOf("MID", "FIN")) emptyList() else {
                    val final = type == "FIN"
                    listOf(ServiceEvent.Source(result.bounded(if(final) "sentence" else "asr"), final, source),
                        ServiceEvent.Translation(result.bounded(if(final) "sentence_trans" else "asr_trans"), final))
                }
            }
            else -> emptyList()
        }
    }
}

/** Append fragments until the provider's explicit turn boundary, with a fixed memory ceiling. */
internal class TurnText {
    var value: String = ""; private set
    fun append(text: String): String { require(value.length + text.length <= 64 * 1024); value += text; return value }
    fun replace(text: String): String { require(text.length <= 64 * 1024); value = text; return value }
    fun clear() { value = "" }
}

internal class GeminiProtocol(private val config: ServiceConfiguration, private val target: String, private val nowMs: () -> Long = { System.nanoTime() / 1_000_000 }) : ServiceProtocol {
    override val frameBytes = 3200
    private var stream: String? = null
    private var sourceLanguage: String? = null
    override fun request() = request(endpoint(config).newBuilder().setQueryParameter("key", config.value("apiKey")).build()).build()
    override fun setup() = textFrame(obj("setup" to obj("model" to "models/${config.model.ifBlank { config.provider.model }}",
        "inputAudioTranscription" to obj(), "outputAudioTranscription" to obj(),
        "generationConfig" to obj("responseModalities" to JSONArray(listOf("AUDIO")),
            "translationConfig" to obj("targetLanguageCode" to geminiTargetCode(target), "echoTargetLanguage" to true)))))
    override fun audio(data: ByteArray): WireFrame { require(data.size == frameBytes); return textFrame(obj("realtimeInput" to obj("audio" to obj("data" to encoded(data), "mimeType" to "audio/pcm;rate=16000")))) }
    override fun finish() = textFrame(obj("realtimeInput" to obj("audioStreamEnd" to true)))
    override fun text(value: String): List<ServiceEvent> {
        val json = JSONObject(value); require(!json.has("error") && !json.has("goAway"))
        if (json.has("setupComplete")) return listOf(ServiceEvent.Ready)
        val content = json.optJSONObject("serverContent") ?: return emptyList()
        if (content.optBoolean("interrupted")) { stream = null; sourceLanguage = null; return emptyList() }
        val events = mutableListOf<ServiceEvent>()
        content.optJSONObject("inputTranscription")?.let {
            sourceLanguage = it.optString("languageCode").ifBlank { it.optString("language") }.substringBefore('-').ifBlank { sourceLanguage }
            val text = it.transcriptText()
            if (text.isNotEmpty()) events += exchange("source", text)
        }
        content.optJSONObject("outputTranscription")?.let {
            val text = it.transcriptText()
            if (text.isNotEmpty()) events += exchange("translation", text)
        }
        if (content.optBoolean("turnComplete")) events += exchange("turn_complete")
        return events
    }
    private fun exchange(action: String, delta: String = ""): List<ServiceEvent> {
        val response = SharedSubtitleCore.exchange(obj("state" to (stream ?: JSONObject.NULL),
            "operation" to obj("type" to "gemini", "action" to action, "delta" to delta, "elapsed_ms" to nowMs())))
        stream = response.getString("state")
        val events = response.getJSONArray("events")
        return (0 until events.length()).map { index ->
            val event = events.getJSONObject(index)
            when (event.getString("type")) {
                "source_draft" -> ServiceEvent.Source(event.getJSONObject("payload").getString("text"), language = sourceLanguage)
                "translation_draft" -> ServiceEvent.Translation(event.getString("payload"))
                "subtitle_final_pair" -> event.getJSONObject("payload").let {
                    ServiceEvent.FinalPair(it.getString("source"), it.getString("translation"), sourceLanguage)
                }
                else -> error("gemini_transcript_safety_limit")
            }
        }
    }
    override fun tick() = exchange("settle")
    override fun binary(value: ByteArray) = text(value.toString(Charsets.UTF_8))
}

internal class AzureProtocol(private val config: ServiceConfiguration, private val target: String) : ServiceProtocol {
    override val frameBytes = 9600
    private val source = TranscriptBuffer(); private val translated = TranscriptBuffer()
    private var stream: String? = null
    private val translationSeen = TurnText()
    override fun request(): Request {
        val raw = config.value("endpoint").replaceFirst("wss://", "https://")
        val root = raw.toHttpUrl()
        require(root.scheme == "https" && root.port == 443 && root.encodedPath == "/" && root.query == null && root.fragment == null && root.username.isEmpty() && root.password.isEmpty())
        require(listOf(".openai.azure.com", ".openai.azure.cn", ".openai.azure.us").any { root.host.endsWith(it) && root.host.length > it.length })
        val url = root.newBuilder().encodedPath("/openai/v1/realtime/translations").addQueryParameter("model", config.value("deployment")).build()
        return request(url).header("api-key", config.value("apiKey")).build()
    }
    override fun setup(): WireFrame {
        require(target in ServiceProvider.OPENAI.targets) { "unsupported_language" }
        val transcription = config.value("transcriptionDeployment").trim()
        require(transcription.isNotEmpty()) { "missing_deployment" }
        return textFrame(obj("type" to "session.update", "session" to obj("audio" to obj(
            "input" to obj("transcription" to obj("model" to transcription)), "output" to obj("language" to target)))))
    }
    override fun audio(data: ByteArray): WireFrame { require(data.size == frameBytes); return textFrame(obj("type" to "session.input_audio_buffer.append", "audio" to encoded(data))) }
    override fun finish() = textFrame(obj("type" to "session.close"))
    override fun text(value: String): List<ServiceEvent> {
        val json = JSONObject(value); val type = json.getString("type")
        require(type != "error")
        if (type == "session.updated") {
            require(json.getJSONObject("session").getJSONObject("audio").getJSONObject("output").getString("language") == target)
            return listOf(ServiceEvent.Ready)
        }
        if (type == "session.closed") return listOf(ServiceEvent.Closed)
        if (type == "session.input_transcript.delta") {
            val update = source.append(json.bounded("delta"))
            return listOfNotNull(update.final?.let { ServiceEvent.Source(it, true) }, update.draft.takeIf { it.isNotEmpty() }?.let { ServiceEvent.Source(it) })
        }
        val deltaTypes = listOf("session.output_transcript.delta", "response.text.delta", "response.output_text.delta")
        val doneTypes = listOf("session.output_transcript.done", "response.text.done", "response.output_text.done")
        if (type !in deltaTypes && type !in doneTypes) return emptyList()
        val incoming = if (type.startsWith("session.")) "session" else "response"
        if (stream == null) stream = incoming
        if (stream != incoming) return emptyList()
        val done = type in doneTypes
        val delta = if (!done) json.bounded(if(json.has("delta")) "delta" else "text") else {
            val full = json.optString("transcript").ifBlank { json.optString("text") }
            require(full.length <= 128 * 1024)
            if (full.startsWith(translationSeen.value)) full.removePrefix(translationSeen.value) else ""
        }
        translationSeen.append(delta)
        val update = translated.append(delta)
        val events = mutableListOf<ServiceEvent>()
        update.final?.let { events += ServiceEvent.Translation(it,true) }
        if(done) {
            translated.drain().takeIf { it.isNotEmpty() }?.let { events += ServiceEvent.Translation(it,true) }
            translationSeen.clear()
        } else if(update.draft.isNotEmpty()) events += ServiceEvent.Translation(update.draft)
        return events
    }
}

// Exact gemini-3.5-live-translate-preview language list, using Mimi's stable codes.
internal val GEMINI_TRANSLATION_LANGUAGE_CODES = listOf(
    "zh", "en", "ja", "zh_tw", "ko", "ru", "es", "fr", "de", "it", "th", "vi", "id", "ms", "ar",
    "hi", "he", "ur", "bn", "pl", "nl", "tr", "km", "cs", "sv", "hu", "da", "fi", "tl", "fa",
    "af", "ak", "sq", "am", "hy", "az", "eu", "be", "bg", "my", "ca", "hr", "et", "gl", "ka",
    "el", "gu", "ha", "is", "jv", "kn", "kk", "rw", "lo", "lv", "lt", "mk", "ml", "mr", "mn",
    "ne", "no", "pt-BR", "pt-PT", "pa", "ro", "sr", "sd", "si", "sk", "sl", "su", "sw", "ta",
    "te", "uk", "uz", "zu",
)

internal fun geminiTargetCode(target: String): String {
    require(target in GEMINI_TRANSLATION_LANGUAGE_CODES) { "unsupported_language" }
    return when(target) { "zh" -> "zh-Hans"; "zh_tw" -> "zh-Hant"; "tl" -> "fil"; else -> target }
}
