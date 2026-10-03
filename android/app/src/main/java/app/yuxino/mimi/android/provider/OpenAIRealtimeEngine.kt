package app.yuxino.mimi.android.provider

import android.util.Base64
import android.util.Log
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
 * OpenAI Realtime Translation endpoint (gpt-realtime-translate).
 *
 * Wire protocol mirrors mimi's src-tauri/src/core/protocols/openai_realtime.rs:
 *   up   session.update { session.audio.input.transcription.model = gpt-realtime-whisper,
 *                         session.audio.output.language = zh|en|ja }
 *   up   session.input_audio_buffer.append { audio: base64(pcm16le mono 24k, 200 ms frames) }
 *   up   session.close
 *   down session.created/updated/closed,
 *        session.input_transcript.delta, session.output_transcript.delta,
 *        session.output_audio.delta (ignored), error
 *
 * The shared Rust core aligns append-only streams using the same timing and
 * boundary rules as desktop; this adapter only maps protocol events.
 */
class OpenAIRealtimeEngine(private val listener: EngineListener) : ProviderEngine {
    override val sampleRateHz: Int = 24_000

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
    private val finishGate = ProviderFinishGate()
    private val audioBuffer = java.io.ByteArrayOutputStream()

    private var sourceLang: String = "auto"
    private var targetLang: String = "zh"
    private var model: String = MODEL

    private val transcript = SharedTranscriptStream(listener)

    override fun start(
        apiKey: String,
        sourceLang: String,
        targetLang: String,
        customBaseUrl: String,
        customModel: String,
    ) {
        this.sourceLang = sourceLang
        this.targetLang = targetLang
        model = customModel.trim().ifEmpty { MODEL }
        val request = Request.Builder()
            .url(resolveEndpoint(customBaseUrl).also { require(it.startsWith("wss://")) { "speech_https_required" } })
            .header("Authorization", "Bearer $apiKey")
            .build()
        webSocket = client.newWebSocket(request, object : WebSocketListener() {
            override fun onOpen(ws: WebSocket, response: Response) {
                listener.onLog("已连接 OpenAI，正在配置会话…")
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
            val frameBytes = 9_600 // 200 ms of 24 kHz mono PCM16
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
        if (webSocket?.send(JSONObject().put("type", "session.close").toString()) != true) finishGate.complete()
    }

    override fun stop() {
        stopped.set(true)
        finishGate.cancel()
        synchronized(audioBuffer) { audioBuffer.reset() }
        transcript.reset()
        try {
            webSocket?.send(JSONObject().put("type", "session.close").toString())
            webSocket?.close(1000, "bye")
        } catch (_: Exception) {
        }
        sessionReady.set(false)
    }

    private fun resolveEndpoint(customBaseUrl: String): String {
        val custom = normalizeWebSocketUrl(customBaseUrl)
        val base = if (customBaseUrl.isBlank()) ENDPOINT else custom
        return if (base.contains("?")) {
            if (base.contains("model=")) base else "$base&model=$model"
        } else {
            "$base?model=$model"
        }
    }

    private fun buildSessionUpdate(): JSONObject {
        val input = JSONObject()
            .put("transcription", JSONObject().put("model", SOURCE_TRANSCRIPTION_MODEL))
        val output = JSONObject().put("language", targetLang)
        val audio = JSONObject().put("input", input).put("output", output)
        val session = JSONObject().put("audio", audio)
        return JSONObject()
            .put("type", "session.update")
            .put("session", session)
    }

    private fun encodeAudioAppend(pcm: ByteArray): JSONObject {
        val audio = Base64.encodeToString(pcm, Base64.NO_WRAP)
        return JSONObject()
            .put("type", "session.input_audio_buffer.append")
            .put("audio", audio)
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
            "session.closed" -> {
                transcript.finish()
                sessionReady.set(false)
                if (!finishGate.complete()) listener.onClosed()
            }
            "session.input_transcript.delta" -> {
                transcript.append(true, json.optString("delta"), elapsedMillis(json))
            }
            "session.output_transcript.delta" -> {
                transcript.append(false, json.optString("delta"), elapsedMillis(json))
            }
            "session.output_audio.delta" -> Unit
            "error" -> {
                val error = json.optJSONObject("error")
                listener.onError(
                    sanitizeErrorCode(error?.optString("code")),
                    "服务请求失败，请检查 API Key 和服务配置。",
                )
            }
        }
    }

    /** Match Rust's unsigned integer metadata; null/strings/floats are absent. */
    private fun elapsedMillis(json: JSONObject): Long? = when (val value = json.opt("elapsed_ms")) {
        is Int -> value.toLong()
        is Long -> value
        else -> null
    }?.takeIf { it >= 0 }

    companion object {
        private const val TAG = "OpenAIRealtimeEngine"
        const val ENDPOINT = "wss://api.openai.com/v1/realtime/translations"
        const val MODEL = "gpt-realtime-translate"
        const val SOURCE_TRANSCRIPTION_MODEL = "gpt-realtime-whisper"
    }
}
