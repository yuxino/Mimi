package app.yuxino.mimi.android.provider

import okhttp3.*
import okio.ByteString.Companion.toByteString
import java.io.ByteArrayOutputStream
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

internal sealed interface ServiceEvent {
    data object Ready : ServiceEvent
    data object Closed : ServiceEvent
    data class Source(val text: String, val final: Boolean = false, val language: String? = null) : ServiceEvent
    data class Translation(val text: String, val final: Boolean = false) : ServiceEvent
}
internal sealed interface WireFrame {
    data class Text(val value: String) : WireFrame
    data class Binary(val value: ByteArray) : WireFrame
}
internal interface ServiceProtocol {
    val frameBytes: Int
    fun request(): Request
    fun setup(): WireFrame?
    fun audio(data: ByteArray): WireFrame
    fun text(value: String): List<ServiceEvent> = emptyList()
    fun binary(value: ByteArray): List<ServiceEvent> = emptyList()
    fun finish(): WireFrame?
}

/** One adapter instance per session. All callbacks and audio writes share a bounded state lock. */
class StreamingServiceEngine(private val config: ServiceConfiguration, private val listener: EngineListener) : ProviderEngine {
    override val sampleRateHz = config.provider.sampleRate
    private val lock = Any()
    private var stopped = false
    private var finishing = false
    private val finishGate = ProviderFinishGate()
    private var ready = false
    private var socket: WebSocket? = null
    private var protocol: ServiceProtocol? = null
    private val pending = ByteArrayOutputStream()
    private val timer = Executors.newSingleThreadScheduledExecutor()

    override fun start(apiKey: String, sourceLang: String, targetLang: String, customBaseUrl: String, customModel: String) {
        synchronized(lock) {
            if (stopped || protocol != null) return
            try {
                require(config.provider.configured(config.credentials))
                require(sourceLang in config.provider.sources && targetLang in config.provider.targets && sourceLang != targetLang)
                val adapter = createProtocol(config, sourceLang, targetLang)
                protocol = adapter
                socket = CLIENT.newWebSocket(adapter.request(), object : WebSocketListener() {
                    override fun onOpen(webSocket: WebSocket, response: Response) = synchronized(lock) {
                        if (stopped) { webSocket.cancel(); return@synchronized }
                        socket = webSocket
                        adapter.setup()?.let { if (!send(it)) fail("setup_send_failed") }
                        Unit
                    }
                    override fun onMessage(webSocket: WebSocket, text: String) = receive {
                        require(text.length <= MAX_MESSAGE)
                        adapter.text(text)
                    }
                    override fun onMessage(webSocket: WebSocket, bytes: okio.ByteString) = receive {
                        require(bytes.size <= MAX_MESSAGE)
                        adapter.binary(bytes.toByteArray())
                    }
                    override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) = synchronized(lock) {
                        if (!finishGate.complete()) fail("transport_error")
                    }
                    override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                        webSocket.close(code, null)
                        synchronized(lock) { if (!stopped && !finishGate.complete()) { shutdown(); listener.onClosed() } }
                    }
                    override fun onClosed(webSocket: WebSocket, code: Int, reason: String) = synchronized(lock) {
                        if (!stopped && !finishGate.complete()) { shutdown(); listener.onClosed() }
                    }
                })
                timer.schedule({ synchronized(lock) { if (!stopped && !ready) fail("setup_timeout") } }, 20, TimeUnit.SECONDS)
            } catch (_: Exception) { fail("invalid_configuration") }
        }
    }
    private fun receive(decode: () -> List<ServiceEvent>) = synchronized(lock) {
        if (stopped) return@synchronized
        try {
            for (event in decode()) {
                if (stopped) break
                when (event) {
                    ServiceEvent.Ready -> if (!ready) { ready = true; sendAudio(ByteArray(0)); listener.onSessionReady() }
                    ServiceEvent.Closed -> { if (!finishGate.complete()) { shutdown(); listener.onClosed() } }
                    is ServiceEvent.Source -> if (ready) {
                        if (event.final) listener.onSourceFinal(event.text, event.language)
                        else listener.onSourceDraft(event.text, event.language)
                    }
                    is ServiceEvent.Translation -> if (ready) {
                        if (event.final) listener.onTranslationFinal(event.text)
                        else listener.onTranslationDraft(event.text)
                    }
                }
            }
        } catch (_: Exception) { fail("invalid_server_event") }
    }
    override fun sendAudio(pcm16Mono: ByteArray) = synchronized(lock) {
        val adapter = protocol ?: return@synchronized
        if (stopped || finishing) return@synchronized
        val limit = sampleRateHz.toLong() * 2 * SharedSubtitleCore.policy.getLong("startup_audio_limit_ms") / 1000
        if (pending.size().toLong() + pcm16Mono.size > limit) { fail("audio_buffer_limit"); return@synchronized }
        if (pcm16Mono.size > sampleRateHz * 2) { fail("audio_buffer_limit"); return@synchronized }
        pending.write(pcm16Mono)
        if (!ready) return@synchronized
        val bytes = pending.toByteArray()
        pending.reset()
        var offset = 0
        while (offset + adapter.frameBytes <= bytes.size) {
            if (!send(adapter.audio(bytes.copyOfRange(offset, offset + adapter.frameBytes)))) {
                fail("audio_send_failed"); return@synchronized
            }
            offset += adapter.frameBytes
        }
        pending.write(bytes, offset, bytes.size - offset)
    }
    private fun send(frame: WireFrame): Boolean {
        val ws = socket ?: return false
        if (ws.queueSize() > 512 * 1024) return false
        return when (frame) {
            is WireFrame.Text -> ws.send(frame.value)
            is WireFrame.Binary -> ws.send(frame.value.toByteString())
        }
    }
    override fun finish(onFinished: () -> Unit) = synchronized(lock) {
        if (stopped || !ready) { stop(); onFinished(); return@synchronized }
        if (finishing) return@synchronized
        finishing = true
        finishGate.begin { stop(); onFinished() }
        val adapter = protocol ?: run { finishGate.complete(); return@synchronized }
        val tail = pending.toByteArray()
        pending.reset()
        if (tail.isNotEmpty() && !send(adapter.audio(tail))) { finishGate.complete(); return@synchronized }
        adapter.finish()?.let { if (!send(it)) finishGate.complete() }
        Unit
    }
    override fun stop() = synchronized(lock) {
        if (stopped) return@synchronized
        // Stop is immediate: no retained audio, and callbacks cannot resurrect the overlay.
        try { protocol?.finish()?.let { send(it) }; socket?.close(1000, "bye") } catch (_: Exception) { }
        shutdown()
    }
    private fun fail(code: String) {
        if (stopped) return
        if (finishGate.complete()) return
        shutdown()
        listener.onError(code, "服务连接失败，请检查网络和配置。")
    }
    private fun shutdown() {
        stopped = true; ready = false; pending.reset()
        finishGate.cancel()
        socket?.cancel(); socket = null; protocol = null
        timer.shutdownNow()
    }
    companion object {
        private const val MAX_MESSAGE = 1024 * 1024
        private val CLIENT = OkHttpClient.Builder().followRedirects(false).followSslRedirects(false).connectTimeout(15, TimeUnit.SECONDS)
            .readTimeout(0, TimeUnit.MILLISECONDS).pingInterval(20, TimeUnit.SECONDS).build()
    }
}

internal fun createProtocol(config: ServiceConfiguration, source: String, target: String): ServiceProtocol =
    when (config.provider) {
        ServiceProvider.GEMINI -> GeminiProtocol(config, target)
        ServiceProvider.AZURE -> AzureProtocol(config, target)
        ServiceProvider.VOLCANO -> VolcanoProtocol(config, source, target)
        ServiceProvider.TENCENT -> TencentProtocol(config, source, target)
        ServiceProvider.BAIDU -> BaiduProtocol(config, source, target)
        ServiceProvider.XAI -> GrokProtocol(config, target)
        else -> error("unsupported_adapter")
    }
