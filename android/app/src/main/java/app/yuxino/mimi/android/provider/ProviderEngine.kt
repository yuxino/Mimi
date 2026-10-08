package app.yuxino.mimi.android.provider

/**
 * A provider session: WebSocket streaming transcription + translation.
 *
 * The capture service pushes mono PCM16 bytes at [sampleRateHz]; the engine
 * frames, base64-encodes, and streams them. Server events surface on the bus.
 */
interface ProviderEngine {
    val sampleRateHz: Int

    /**
     * Opens the WebSocket and sends the session configuration.
     * [customBaseUrl] is an optional complete endpoint override; blank means
     * the provider's official endpoint. [customModel] overrides the model
     * query parameter; blank means the engine's default.
     */
    fun start(
        apiKey: String,
        sourceLang: String,
        targetLang: String,
        customBaseUrl: String = "",
        customModel: String = "",
    )

    /**
     * Optional vocabulary biasing, applied before [start]. Providers that do
     * not support hotwords ignore it.
     */
    fun setHotwords(words: Map<String, String>) = Unit

    /** Pushes one chunk of mono PCM16LE audio at [sampleRateHz]. */
    fun sendAudio(pcm16Mono: ByteArray)

    /** Closes the session gracefully. */
    fun stop()

    /** Seal audio, admit provider tail events, then acknowledge bounded graceful finish. */
    fun finish(onFinished: () -> Unit) { stop(); onFinished() }
}

/** Events every engine maps its wire protocol onto. */
interface EngineListener {
    fun onRuntimeSnapshot(state: org.json.JSONObject, originalOnly: Boolean) = Unit
    fun onSessionReady()

    /** [language] is the provider's detected source language, if reported. */
    fun onSourceDraft(text: String, language: String? = null)
    fun onSourceFinal(text: String, language: String? = null)
    fun onTranslationDraft(text: String)
    fun onTranslationFinal(text: String)
    fun onUtteranceText(id: String, source: Boolean, text: String, final: Boolean, language: String? = null) {
        if (source) { if (final) onSourceFinal(text, language) else onSourceDraft(text, language) }
        else { if (final) onTranslationFinal(text) else onTranslationDraft(text) }
    }
    fun onFinalPair(source: String, translation: String, language: String? = null) {
        onSourceFinal(source, language)
        onTranslationFinal(translation)
    }
    fun onIdentifiedFinalPair(id: String, source: String, translation: String, language: String? = null) {
        onFinalPair(source, translation, language)
    }
    fun onError(code: String, message: String)
    fun onClosed()

    /** Connection-level progress, surfaced on the overlay status line. */
    fun onLog(message: String)
}

/** Normalizes a user-pasted base URL into a WebSocket endpoint. */
fun normalizeWebSocketUrl(url: String): String {
    val trimmed = url.trim()
    return when {
        trimmed.startsWith("wss://") || trimmed.startsWith("ws://") -> trimmed
        trimmed.startsWith("https://") -> "wss://" + trimmed.removePrefix("https://")
        trimmed.startsWith("http://") -> "ws://" + trimmed.removePrefix("http://")
        else -> "wss://$trimmed"
    }
}

internal const val CREDENTIAL_AUTHENTICATION_FAILED = "credential_authentication_failed"

/** Only an explicit HTTP authentication rejection changes the transport failure category. */
internal fun websocketFailureCode(status: Int?): String = when (status) {
    401, 403 -> CREDENTIAL_AUTHENTICATION_FAILED
    else -> "transport_error"
}

/** Keep provider-controlled diagnostics bounded and free of arbitrary messages. */
internal fun sanitizeErrorCode(code: String?): String =
    code?.takeIf { it.length in 1..64 && it.all { c -> c.isLetterOrDigit() || c == '_' || c == '-' } }
        ?: "provider_error"
