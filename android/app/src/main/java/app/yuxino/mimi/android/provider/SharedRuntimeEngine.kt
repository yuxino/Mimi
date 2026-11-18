package app.yuxino.mimi.android.provider

import org.json.JSONObject
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.concurrent.thread

/** Capture/lifecycle adapter only. Every provider and subtitle decision is Rust. */
internal class SharedRuntimeEngine(
    private val configuration: JSONObject,
    private val listener: EngineListener,
    private val audioSource: String = "system",
    private val historyHandle: Long? = null,
) : ProviderEngine {
    override val sampleRateHz: Int = NativeRuntimeConfiguration.capabilities(
        configuration.getString("provider"), configurationRoute(configuration), configuration.getString("targetLanguage"),
        configuration.optString("alibabaRealtimeMode", "disabled"),
        configuration.optString("xaiSpeechMode", "voice"),
        configuration.optString("speechTranslationMode", "simultaneous"),
    ).getInt("sampleRateHz")

    private val isActive = AtomicBoolean(false)
    private val runtimeLock = Any()
    private var runtimeHandle = 0L
    private var finishCallback: (() -> Unit)? = null
    private var pollWorker: Thread? = null

    override fun start() {
        synchronized(runtimeLock) {
            check(runtimeHandle == 0L) { "native_runtime_already_started" }
            val network = NativeRuntimeNetwork.snapshot(configuration)
            runtimeHandle = NativeRuntimeConfiguration.exchange(
                JSONObject().put("operation", "create")
                    .put("configuration", configuration)
                    .put("network", network)
                    .put("audioSource", audioSource)
                    .apply { historyHandle?.let { put("historyHandle", it) } },
            ).getLong("handle")
            try {
                NativeRuntimeConfiguration.command("start", runtimeHandle)
            } catch (error: Exception) {
                NativeRuntimeConfiguration.command("stop", runtimeHandle)
                runtimeHandle = 0L
                throw error
            }
            isActive.set(true)
            val ownedHandle = runtimeHandle
            pollWorker = thread(name = "mimi-native-state") {
                val cursor = NativeRuntimePollCursor()
                try {
                    while (isActive.get()) {
                        val state = synchronized(runtimeLock) {
                            if (runtimeHandle != ownedHandle || !isActive.get()) return@thread
                            NativeRuntimeConfiguration.poll(ownedHandle, cursor.lastFullRevision)
                        }
                        val finished = cursor.consume(state, onSnapshot = { snapshot ->
                            listener.onRuntimeSnapshot(snapshot, configuration.getString("targetLanguage") == "original")
                        }, onTerminal = { code ->
                            val callback = synchronized(runtimeLock) {
                                finishCallback.also { finishCallback = null }
                            }
                            if (code != null) listener.onError(sanitizeErrorCode(code))
                            else if (callback != null) callback() else listener.onClosed()
                        })
                        if (finished) return@thread
                        Thread.sleep(NativeRuntimeConfiguration.policy.getLong("snapshotPublishIntervalMs"))
                    }
                } catch (_: InterruptedException) {
                } catch (_: Exception) {
                    if (isActive.get()) listener.onError("native_runtime_failed")
                }
            }
        }
    }

    override fun sendAudio(pcm16Mono: ByteArray) {
        synchronized(runtimeLock) {
            if (isActive.get() && runtimeHandle != 0L) {
                NativeRuntimeConfiguration.pcmRaw(runtimeHandle, pcm16Mono)
            }
        }
    }

    override fun finish(onFinished: () -> Unit) {
        synchronized(runtimeLock) {
            if (!isActive.get() || runtimeHandle == 0L) {
                onFinished()
                return
            }
            finishCallback = onFinished
            NativeRuntimeConfiguration.command("finish", runtimeHandle)
        }
    }

    override fun stop() {
        synchronized(runtimeLock) {
            isActive.set(false)
            finishCallback = null
            if (runtimeHandle != 0L) NativeRuntimeConfiguration.command("stop", runtimeHandle)
            runtimeHandle = 0L
            pollWorker?.interrupt()
            pollWorker = null
        }
    }
}

/** Tracks delivered JNI snapshot revisions; terminal metadata also comes with light replies. */
internal class NativeRuntimePollCursor {
    var lastFullRevision: Long? = null
        private set

    fun consume(state: JSONObject, onSnapshot: (JSONObject) -> Unit, onTerminal: (String?) -> Unit): Boolean {
        val snapshotRevision = state.getLong("version")
        require(snapshotRevision >= 0) { "native_runtime_invalid_response" }
        val unchanged = state.optBoolean("unchanged", false)
        val finished = state.optBoolean("finished")
        if (unchanged) {
            require(lastFullRevision == snapshotRevision && !state.has("snapshot") &&
                state.opt("finished") is Boolean && state.has("errorCode")) { "native_runtime_invalid_response" }
        } else if (snapshotRevision != lastFullRevision || finished) {
            state.getJSONObject("snapshot")
            onSnapshot(state)
            lastFullRevision = snapshotRevision
        }
        if (finished) {
            onTerminal(state.optString("errorCode").takeUnless { it.isBlank() || it == "null" })
        }
        return finished
    }
}

internal fun configurationRoute(configuration: JSONObject): String =
    configuration.optJSONObject("textCredentials")?.optString("kind")?.takeIf { it.isNotEmpty() }
        ?: configuration.getJSONObject("credentials").optString("kind")
            .takeIf { it in setOf("deepL", "deepLX", "openAICompatible", "chatMock") }
        ?: "followService"
