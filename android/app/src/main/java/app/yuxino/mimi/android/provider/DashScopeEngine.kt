package app.yuxino.mimi.android.provider

import android.util.Base64
import android.util.Log
import okhttp3.HttpUrl.Companion.toHttpUrl
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import org.json.JSONObject
import org.json.JSONException
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

/**
 * DashScope unified realtime endpoint (qwen3.5-livetranslate-flash-realtime).
 *
 * Wire protocol mirrors mimi's src-tauri/src/core/protocols/live_translate.rs:
 *   up   session.update { modalities:["text"], sample_rate:16000,
 *                         input_audio_format:"pcm", input_audio_transcription, translation }
 *   up   input_audio_buffer.append { audio: base64(pcm16le mono 16k) }
 *   up   session.finish
 *   down session.created/updated/finished,
 *        conversation.item.input_audio_transcription.text/.completed,
 *        response.text.text/.done, response.audio_transcript.text/.done, error
 */
class DashScopeEngine(
    private val listener: EngineListener,
    private val transcriptionOnly: Boolean = false,
) : ProviderEngine {
    override val sampleRateHz: Int = 16_000

    private val client = OkHttpClient.Builder()
        .followRedirects(false)
        .followSslRedirects(false)
        .connectTimeout(15, TimeUnit.SECONDS)
        .readTimeout(0, TimeUnit.MILLISECONDS)
        .pingInterval(20, TimeUnit.SECONDS)
        .build()

    private var webSocket: WebSocket? = null
    private val sessionReady = AtomicBoolean(false)
    private val stopped = AtomicBoolean(false)
    private val finishing = AtomicBoolean(false)
    private val finishGate = ProviderFinishGate(SharedSubtitleCore.policy.getLong("recognition_finish_timeout_ms"))
    private val audioBuffer = java.io.ByteArrayOutputStream()
    private val pairedStream = SharedLivePairStream(listener)
    private var sourceLang: String = "auto"
    private var targetLang: String = "zh"
    private var model: String = if (transcriptionOnly) ASR_MODEL else MODEL
    private var hotwords: Map<String, String> = emptyMap()

    override fun setHotwords(words: Map<String, String>) {
        hotwords = words
    }

    override fun start(
        apiKey: String,
        sourceLang: String,
        targetLang: String,
        customBaseUrl: String,
        customModel: String,
    ) {
        this.sourceLang = sourceLang
        this.targetLang = targetLang
        model = customModel.trim().ifEmpty { if (transcriptionOnly) ASR_MODEL else MODEL }
        val url = resolveEndpoint(customBaseUrl).also { require(it.startsWith("wss://")) { "speech_https_required" } }
        val request = Request.Builder()
            .url(url)
            .header("Authorization", "Bearer $apiKey")
            .build()
        webSocket = client.newWebSocket(request, object : WebSocketListener() {
            override fun onOpen(ws: WebSocket, response: Response) {
                listener.onLog("已连接 DashScope，正在配置会话…")
                ws.send(buildSessionUpdate().toString())
            }

            override fun onMessage(ws: WebSocket, text: String) {
                receiveServerMessage(text)
            }

            override fun onFailure(ws: WebSocket, t: Throwable, response: Response?) {
                if (stopped.get()) return
                if (finishGate.complete()) return
                Log.w(TAG, "WebSocket transport failure (HTTP ${response?.code ?: 0})")
                listener.onError(
                    "transport_error",
                    "连接失败，请检查网络和服务配置。",
                )
                sessionReady.set(false)
            }

            override fun onClosed(ws: WebSocket, code: Int, reason: String) {
                sessionReady.set(false)
                if (!stopped.get() && !finishGate.complete()) listener.onClosed()
            }
        })
    }

    override fun sendAudio(pcm16Mono: ByteArray) {
        if (stopped.get() || finishing.get()) return
        synchronized(audioBuffer) {
            if (stopped.get() || finishing.get()) return
            val limit = sampleRateHz.toLong() * 2 * SharedSubtitleCore.policy.getLong("startup_audio_limit_ms") / 1000
            if (audioBuffer.size().toLong() + pcm16Mono.size > limit) {
                listener.onError("audio_buffer_limit", "连接尚未就绪，音频缓冲已满。")
                return
            }
            audioBuffer.write(pcm16Mono)
            if (!sessionReady.get()) return
            val data = audioBuffer.toByteArray()
            val frameBytes = 3200 // 100 ms of 16 kHz mono PCM16
            var offset = 0
            while (offset + frameBytes <= data.size) {
                val frame = data.copyOfRange(offset, offset + frameBytes)
                if (webSocket?.send(encodeAudioAppend(frame).toString()) != true) {
                    audioBuffer.reset()
                    sessionReady.set(false)
                    listener.onError("audio_send_failed", "音频发送失败，请重新连接。")
                    return
                }
                offset += frameBytes
            }
            if (offset > 0) {
                audioBuffer.reset()
                audioBuffer.write(data, offset, data.size - offset)
            }
        }
    }

    override fun finish(onFinished: () -> Unit) {
        if (stopped.get() || !sessionReady.get()) { stop(); onFinished(); return }
        if (!finishing.compareAndSet(false, true)) return
        finishGate.begin { stop(); onFinished() }
        synchronized(audioBuffer) {
            val tail = audioBuffer.toByteArray()
            audioBuffer.reset()
            if (tail.isNotEmpty() && webSocket?.send(encodeAudioAppend(tail).toString()) != true) {
                finishGate.complete(); return
            }
        }
        if (webSocket?.send(buildFinish().toString()) != true) finishGate.complete()
    }

    override fun stop() {
        stopped.set(true)
        finishGate.cancel()
        synchronized(audioBuffer) { audioBuffer.reset() }
        try {
            webSocket?.send(buildFinish().toString())
            webSocket?.close(1000, "bye")
        } catch (_: Exception) {
        }
        sessionReady.set(false)
        pairedStream.reset()
    }

    internal fun resolveEndpoint(customBaseUrl: String): String {
        val custom = normalizeWebSocketUrl(customBaseUrl)
        val base = if (customBaseUrl.isBlank()) DASHSCOPE_REALTIME_WS else custom
        if (transcriptionOnly) {
            // A pasted live-translate URL must not override the ASR-only session contract.
            return base.replaceFirst("wss://", "https://").replaceFirst("ws://", "http://")
                .toHttpUrl().newBuilder().setQueryParameter("model", ASR_MODEL).build().toString()
                .replaceFirst("https://", "wss://").replaceFirst("http://", "ws://")
        }
        return if (base.contains("?")) {
            if (base.contains("model=")) base else "$base&model=$model"
        } else {
            "$base?model=$model"
        }
    }

    internal fun buildSessionUpdate(): JSONObject {
        val transcription = JSONObject()
        if (!transcriptionOnly) transcription.put("model", ASR_MODEL)
        if (sourceLang != "auto") {
            transcription.put("language", sourceLang)
        }
        val translation = JSONObject().put("language", targetLang)
        if (hotwords.isNotEmpty()) {
            val phrases = JSONObject()
            for ((term, translation) in hotwords) {
                phrases.put(term, translation)
            }
            translation.put("corpus", JSONObject().put("phrases", phrases))
        }
        val session = JSONObject()
            .put("sample_rate", 16_000)
            .put("input_audio_format", "pcm")
            .put("input_audio_transcription", transcription)
        if (transcriptionOnly) {
            session.put("turn_detection", JSONObject()
                .put("type", "server_vad")
                .put("threshold", 0.2)
                .put("silence_duration_ms", 800))
        } else {
            session.put("modalities", org.json.JSONArray(listOf("text")))
            session.put("translation", translation)
        }
        return JSONObject()
            .put("event_id", "setup_" + System.nanoTime())
            .put("type", "session.update")
            .put("session", session)
    }

    private fun encodeAudioAppend(pcm: ByteArray): JSONObject {
        val audio = Base64.encodeToString(pcm, Base64.NO_WRAP)
        return JSONObject()
            .put("event_id", "audio_" + System.nanoTime())
            .put("type", "input_audio_buffer.append")
            .put("audio", audio)
    }

    private fun buildFinish(): JSONObject {
        return JSONObject()
            .put("event_id", "finish_" + System.nanoTime())
            .put("type", "session.finish")
    }

    internal fun receiveServerMessage(text: String) {
        if (stopped.get()) return
        try {
            handleServerEvent(JSONObject(text))
        } catch (_: JSONException) {
            sessionReady.set(false)
            listener.onError("invalid_server_event", "服务返回了无效数据，请重新连接。")
        } catch (_: IllegalArgumentException) {
            sessionReady.set(false)
            listener.onError("invalid_server_event", "服务返回了无效数据，请重新连接。")
        }
    }

    private fun handleServerEvent(json: JSONObject) {
        when (json.optString("type")) {
            "session.created" -> Unit
            "session.updated" -> {
                sessionReady.set(true)
                sendAudio(ByteArray(0))
                listener.onSessionReady()
            }
            "session.finished" -> {
                if (!transcriptionOnly) pairedStream.observe(JSONObject().put("type", "session_finished"), json)
                sessionReady.set(false)
                if (!finishGate.complete()) listener.onClosed()
            }
            "conversation.item.input_audio_transcription.text" -> {
                if (transcriptionOnly) listener.onSourceDraft(combinedText(json), json.optString("language").takeIf { it.isNotBlank() })
                else pairedStream.observe(JSONObject().put("type", "source_draft").put("text", combinedText(json))
                    .put("language", json.optString("language").takeIf { it.isNotBlank() } ?: JSONObject.NULL), json)
            }
            "conversation.item.input_audio_transcription.completed" -> {
                if (transcriptionOnly) listener.onSourceFinal(
                    json.optString("transcript").trim(),
                    json.optString("language").takeIf { it.isNotBlank() },
                )
                else pairedStream.observe(JSONObject().put("type", "source_final").put("text", json.optString("transcript").trim())
                    .put("language", json.optString("language").takeIf { it.isNotBlank() } ?: JSONObject.NULL), json)
            }
            "response.text.text", "response.audio_transcript.text" -> {
                if (!transcriptionOnly) pairedStream.observe(JSONObject().put("type", "translation_draft").put("text", combinedText(json)), json)
            }
            "response.text.done" -> {
                if (!transcriptionOnly) pairedStream.observe(JSONObject().put("type", "translation_final").put("text", json.optString("text").trim()), json)
            }
            "response.audio_transcript.done" -> {
                if (!transcriptionOnly) pairedStream.observe(JSONObject().put("type", "translation_final").put("text", json.optString("transcript").trim()), json)
            }
            "conversation.item.created" -> if (!transcriptionOnly) pairedStream.observe(JSONObject().put("type", "item_created"), json)
            "error" -> {
                val error = json.optJSONObject("error")
                listener.onError(
                    sanitizeErrorCode(error?.optString("code")),
                    "服务请求失败，请检查 API Key 和服务配置。",
                )
            }
        }
    }

    /** The server's combined preview representation: confirmed text + tentative stash. */
    private fun combinedText(json: JSONObject): String {
        val confirmed = json.optString("text")
        val tentative = json.optString("stash")
        return (confirmed + tentative).trim()
    }

    companion object {
        private const val TAG = "DashScopeEngine"
        const val DASHSCOPE_REALTIME_WS = "wss://dashscope.aliyuncs.com/api-ws/v1/realtime"
        const val MODEL = "qwen3.5-livetranslate-flash-realtime"
        const val ASR_MODEL = "qwen3-asr-flash-realtime"
    }
}
