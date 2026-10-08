package app.yuxino.mimi.android.provider

import org.json.JSONObject
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.concurrent.thread

/** Capture/lifecycle adapter only. Every provider and subtitle decision is Rust. */
internal class SharedRuntimeEngine(
    private val configuration: JSONObject,
    private val listener: EngineListener,
) : ProviderEngine {
    override val sampleRateHz: Int = NativeRuntimeConfiguration.capabilities(
        configuration.getString("provider"), configurationRoute(configuration), configuration.getString("targetLanguage"),
    ).getInt("sampleRateHz")
    private val active = AtomicBoolean(false)
    private val lock = Any()
    private var handle = 0L
    private var finishCallback: (() -> Unit)? = null
    private var worker: Thread? = null

    override fun start(apiKey: String, sourceLang: String, targetLang: String, customBaseUrl: String, customModel: String) {
        synchronized(lock) {
            check(handle == 0L) { "native_runtime_already_started" }
            val network = NativeRuntimeNetwork.snapshot(configuration)
            handle = NativeRuntimeConfiguration.exchange(JSONObject().put("operation", "create")
                .put("configuration", configuration).put("network", network)).getLong("handle")
            try { NativeRuntimeConfiguration.command("start", handle) }
            catch (error: Exception) {NativeRuntimeConfiguration.command("stop", handle);handle=0;throw error}
            active.set(true)
            val owner = handle
            worker = thread(name = "mimi-native-state") {
                var revision = -1L
                try {
                    while (active.get()) {
                        val state = synchronized(lock) {
                            if (handle != owner || !active.get()) return@thread
                            NativeRuntimeConfiguration.command("poll", owner)
                        }
                        val current = state.getLong("version")
                        if (current != revision) {
                            revision = current
                            listener.onRuntimeSnapshot(state, configuration.getString("targetLanguage") == "original")
                        }
                        if (state.optBoolean("finished")) {
                            val callback = synchronized(lock) { finishCallback.also {finishCallback=null} }
                            val code = state.optString("errorCode").takeUnless { it.isBlank() || it == "null" }
                            if (code != null) listener.onError(sanitizeErrorCode(code), "")
                            else if (callback != null) callback() else listener.onClosed()
                            return@thread
                        }
                        Thread.sleep(NativeRuntimeConfiguration.policy.getLong("snapshotPublishIntervalMs"))
                    }
                } catch (_: InterruptedException) { }
                catch (_: Exception) { if (active.get()) listener.onError("native_runtime_failed", "") }
            }
        }
    }
    override fun sendAudio(pcm16Mono: ByteArray) {
        synchronized(lock) { if (active.get() && handle != 0L) NativeRuntimeConfiguration.pcmRaw(handle, pcm16Mono) }
    }
    override fun finish(onFinished: () -> Unit) {
        synchronized(lock) {
            if (!active.get() || handle == 0L) {onFinished();return}
            finishCallback = onFinished
            NativeRuntimeConfiguration.command("finish",handle)
        }
    }
    override fun stop() {
        synchronized(lock) {
            active.set(false)
            finishCallback=null
            if(handle!=0L) NativeRuntimeConfiguration.command("stop",handle)
            handle=0
            worker?.interrupt();worker=null
        }
    }
}
internal fun configurationRoute(configuration: JSONObject): String =
    configuration.optJSONObject("textCredentials")?.optString("kind")?.takeIf {it.isNotEmpty()}
        ?: configuration.getJSONObject("credentials").optString("kind").takeIf {it in setOf("deepL","deepLX","openAICompatible","chatMock")}
        ?: "followService"
