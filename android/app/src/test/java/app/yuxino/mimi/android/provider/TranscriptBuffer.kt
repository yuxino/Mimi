package app.yuxino.mimi.android.provider

/** A bounded visible window over an append-only provider transcript. */
internal class TranscriptBuffer {
    data class Update(val final: String?, val draft: String)

    private var pending = ""

    @Synchronized
    fun append(delta: String): Update {
        // Trim each input before concatenation as a single event may be large.
        val text = (pending + delta.takeLast(MAX_CHARS)).takeLast(MAX_CHARS)
        val boundary = text.indexOfLast { it in ".!?。！？\n" }
        val final = if (boundary >= 0) text.substring(0, boundary + 1).trim().ifEmpty { null } else null
        pending = if (boundary >= 0) text.substring(boundary + 1) else text
        return Update(final, pending)
    }

    @Synchronized
    fun drain(): String = pending.trim().also { pending = "" }

    @Synchronized
    fun clear() {
        pending = ""
    }

    companion object {
        const val MAX_CHARS = 2048
    }
}
