package app.yuxino.mimi.android.provider

import org.json.JSONObject
import java.util.concurrent.Executor
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

/** A UI adapter to PC's bounded text diagnostic; no speech credentials or text parser here. */
internal class NativeTextProbeClient(
    private val exchange: (JSONObject) -> JSONObject = NativeRuntimeConfiguration::exchange,
    private val networkSnapshot: (JSONObject) -> JSONObject = NativeRuntimeNetwork::textSnapshot,
    private val executor: Executor = worker,
) {
    fun check(configuration: JSONObject, callback: (TranslationResult) -> Unit): TranslationCall {
        val gate = Any()
        var cancelled = false
        var handle: Long? = null
        fun cancelHandle(value: Long) {
            runCatching { exchange(JSONObject().put("operation", "text_probe_cancel").put("handle", value)) }
        }
        fun deliver(result: TranslationResult) = synchronized(gate) { if (!cancelled) callback(result) }
        executor.execute {
            val started = System.nanoTime()
            var ownedHandle: Long? = null
            try {
                if (synchronized(gate) { cancelled }) return@execute
                val network = networkSnapshot(configuration)
                if (synchronized(gate) { cancelled }) return@execute
                val created = exchange(JSONObject().put("operation", "create_text_probe")
                    .put("configuration", configuration).put("network", network)).getLong("handle")
                ownedHandle = created
                synchronized(gate) { handle = created }
                while (!synchronized(gate) { cancelled }) {
                    val state = exchange(JSONObject().put("operation", "text_probe_poll").put("handle", created))
                    if (state.getBoolean("finished")) {
                        deliver(nativeTextProbeResult(state.getJSONObject("result")))
                        break
                    }
                    Thread.sleep(100)
                }
            } catch (_: Exception) {
                deliver(TranslationResult.Failure("translation_network",
                    TimeUnit.NANOSECONDS.toMillis(System.nanoTime() - started).coerceAtLeast(0)))
            } finally {
                synchronized(gate) { handle = null }
                ownedHandle?.let(::cancelHandle)
            }
        }
        return TranslationCall {
            val current = synchronized(gate) { cancelled = true; handle }
            current?.let(::cancelHandle)
        }
    }

    private companion object {
        val worker: Executor = Executors.newSingleThreadExecutor { action ->
            Thread(action, "mimi-text-probe-ui").apply { isDaemon = true }
        }
    }
}

internal fun buildTextProbeConfiguration(
    configuration: TranslationConfiguration,
    source: String,
    target: String,
    model: String,
): JSONObject = JSONObject()
    .put("credentials", requireNotNull(configuration.runtimeCredentials()) { "translation_provider" })
    .put("sourceLanguage", source)
    .put("targetLanguage", target)
    .put("qwenMtModel", normalizeQwenMTModel(model))

/** Only the shared diagnostic's fixed labels affect display; response content stays in Rust. */
internal fun nativeTextProbeResult(result: JSONObject): TranslationResult {
    val elapsed = result.optLong("elapsedMs", 0).coerceAtLeast(0)
    if (result.optString("service") == "available") return TranslationResult.Success("", elapsed)
    val code = when (result.optString("reason")) {
        "authenticationRejected", "credentialsMissing", "credentialsAccessDenied" -> "translation_http_401"
        "serviceNotActivated", "quotaExhausted", "concurrencyLimited" -> "translation_http_429"
        "timeout" -> "translation_timeout"
        else -> "translation_network"
    }
    return TranslationResult.Failure(code, elapsed)
}
