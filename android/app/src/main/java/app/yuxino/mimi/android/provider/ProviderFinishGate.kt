package app.yuxino.mimi.android.provider

import java.util.concurrent.ScheduledThreadPoolExecutor
import java.util.concurrent.TimeUnit

/** Transport-only acknowledgement gate; all grace windows come from shared Rust. */
internal class ProviderFinishGate(private val timeoutMs: Long = SharedSubtitleCore.policy.getLong("realtime_finish_timeout_ms")) {
    private val lock = Any()
    private var callback: (() -> Unit)? = null
    private var deadline: java.util.concurrent.ScheduledFuture<*>? = null
    fun begin(onFinished: () -> Unit) {
        synchronized(lock) {
            if (callback != null) return
            callback = onFinished
            deadline = timer.schedule({ complete() }, timeoutMs, TimeUnit.MILLISECONDS)
        }
    }
    fun complete(): Boolean {
        val action = synchronized(lock) {
            val next = callback ?: return false
            callback = null
            deadline?.cancel(false); deadline = null
            next
        }
        action()
        return true
    }
    fun cancel() = synchronized(lock) { callback = null; deadline?.cancel(false); deadline = null }
    companion object {
        private val timer = ScheduledThreadPoolExecutor(1) { task -> Thread(task, "mimi-provider-finish").apply { isDaemon = true } }
            .apply { removeOnCancelPolicy = true }
    }
}
