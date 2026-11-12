package app.yuxino.mimi.android.provider

/** Native capture/lifecycle boundary; the configured shared Rust runtime owns providers. */
interface ProviderEngine {
    val sampleRateHz: Int

    /** Starts the runtime using the configuration supplied to its constructor. */
    fun start()

    /** Pushes one chunk of mono PCM16LE audio at [sampleRateHz]. */
    fun sendAudio(pcm16Mono: ByteArray)

    fun stop()

    /** Seal audio, admit provider tail events, then acknowledge bounded graceful finish. */
    fun finish(onFinished: () -> Unit) { stop(); onFinished() }
}

/** Complete shared-runtime snapshots and content-free terminal events. */
interface EngineListener {
    fun onRuntimeSnapshot(state: org.json.JSONObject, originalOnly: Boolean)
    fun onError(code: String)
    fun onClosed()
}

internal const val CREDENTIAL_AUTHENTICATION_FAILED = "credential_authentication_failed"

/** Keep provider-controlled diagnostics bounded and free of arbitrary messages. */
internal fun sanitizeErrorCode(code: String?): String =
    code?.takeIf { it.length in 1..64 && it.all { c -> c.isLetterOrDigit() || c == '_' || c == '-' } }
        ?: "provider_error"
