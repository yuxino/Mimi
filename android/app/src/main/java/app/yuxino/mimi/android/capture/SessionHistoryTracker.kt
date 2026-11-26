package app.yuxino.mimi.android.capture

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import app.yuxino.mimi.android.provider.NativeRuntimeConfiguration
import java.io.File
import java.util.concurrent.atomic.AtomicLong
import kotlin.concurrent.thread
import org.json.JSONObject

/** Owns one session's local archive handle, per-source storage status and post-stop finalization polling. */
internal class SessionHistoryTracker(
    private val context: Context,
    private val readStatus: () -> Pair<Boolean, Boolean>,
    private val writeStatus: (saveError: Boolean, limited: Boolean, notify: Boolean) -> Unit,
    private val onAsyncFinalized: (failed: Boolean, limited: Boolean) -> Unit,
) {
    private var historyHandle: Long? = null
    private val historyStates = mutableMapOf<CaptureSource, Pair<Boolean, Boolean>>()

    fun begin(enabled: Boolean): Long? {
        historyStatusGeneration.incrementAndGet()
        writeStatus(false, false, false)
        historyStates.clear()
        historyHandle = if (enabled) {
            runCatching {
                NativeRuntimeConfiguration.exchange(
                    JSONObject().put("operation", "history_begin")
                        .put("directory", File(context.filesDir, "session-history").absolutePath),
                ).getLong("historyHandle")
            }.getOrElse {
                writeStatus(true, false, false)
                null
            }
        } else null
        return historyHandle
    }

    fun onSnapshot(source: CaptureSource, state: JSONObject) {
        val (previousError, previousLimited) = readStatus()
        historyStates[source] = state.optBoolean("historySaveError") to state.optBoolean("historyLimited")
        val nextError = previousError || historyStates.values.any { it.first }
        val nextLimited = previousLimited || historyStates.values.any { it.second }
        writeStatus(nextError, nextLimited, (previousError to previousLimited) != (nextError to nextLimited))
    }

    fun disable() {
        val handle = historyHandle ?: return
        val (currentError, currentLimited) = readStatus()
        var nextError = currentError
        var nextLimited = currentLimited
        runCatching {
            NativeRuntimeConfiguration.exchange(
                JSONObject().put("operation", "history_disable").put("historyHandle", handle),
            )
        }.onSuccess { response ->
            nextError = response.optJSONObject("state")?.optBoolean("saveError") == true
            nextLimited = false
            historyStates.clear()
        }.onFailure {
            nextError = true
        }
        writeStatus(nextError, nextLimited, true)
    }

    fun finish(retain: Boolean) {
        historyHandle?.let { handle ->
            val operation = if (retain) "history_finish" else "history_disable"
            val (currentError, currentLimited) = readStatus()
            runCatching {
                NativeRuntimeConfiguration.exchange(
                    JSONObject().put("operation", operation).put("historyHandle", handle),
                )
            }.onSuccess { response ->
                val nextError = currentError || response.optJSONObject("state")?.optBoolean("saveError") == true
                if (nextError != currentError) writeStatus(nextError, currentLimited, false)
                if (response.optBoolean("pending")) observeFinalization(handle, onAsyncFinalized)
            }.onFailure {
                writeStatus(true, currentLimited, false)
            }
        }
        historyHandle = null
        historyStates.clear()
    }

    companion object {
        private val historyStatusGeneration = AtomicLong()

        /** A terminal runtime can release its journal after its service is destroyed. */
        private fun observeFinalization(
            handle: Long,
            onResult: (failed: Boolean, limited: Boolean) -> Unit,
        ) {
            val owner = historyStatusGeneration.get()
            val timeout = NativeRuntimeConfiguration.policy.getLong("sessionFinishTimeoutMs")
            thread(name = "mimi-history-finalize", isDaemon = true) {
                var failed = false
                var limited = false
                try {
                    val deadline = SystemClock.elapsedRealtime() + timeout
                    while (historyStatusGeneration.get() == owner) {
                        val result = NativeRuntimeConfiguration.exchange(
                            JSONObject().put("operation", "history_state").put("historyHandle", handle),
                        )
                        failed = result.getJSONObject("state").getBoolean("saveError")
                        limited = result.getJSONObject("state").getBoolean("limited")
                        if (!result.getBoolean("pending")) break
                        if (SystemClock.elapsedRealtime() >= deadline) {
                            failed = true
                            break
                        }
                        Thread.sleep(100)
                    }
                } catch (_: Exception) {
                    failed = true
                }
                Handler(Looper.getMainLooper()).post {
                    if (historyStatusGeneration.get() == owner) onResult(failed, limited)
                }
            }
        }
    }
}
