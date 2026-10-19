package app.yuxino.mimi.android.provider

import org.json.JSONObject
import java.util.concurrent.ScheduledThreadPoolExecutor
import java.util.concurrent.TimeUnit

/** Serial finals; shared Rust policy owns queue, retry and original-age bounds. */
class TranslationPipeline(
    private val client: TranslationClient,
    private val sourceLanguage: String,
    private val targetLanguage: String,
    private val listener: Listener,
    private val nanoTime: () -> Long = System::nanoTime,
    private val scheduleDeadline: (Long, () -> Unit) -> TranslationCall = ::scheduleTranslationDeadline,
) {
    interface Listener {
        fun onTranslation(source: String, language: String?, translation: String, elapsedMs: Long)
        fun onTranslationForUtterance(sourceUtteranceId: Long?, source: String, language: String?, translation: String, elapsedMs: Long) {
            onTranslation(source, language, translation, elapsedMs)
        }
        fun onRetry(nextAttempt: Int, delayMs: Long) {}
        fun onError(code: String)
    }

    private class Entry(val text: String, val language: String?, val sourceUtteranceId: Long?, val enqueuedAtNanos: Long) {
        var attempts = 0
    }
    private class Attempt(val entry: Entry, val number: Int) {
        var installing = true
        var claimed = false
        var immediateResult: TranslationResult? = null
    }
    private class Retry(val entry: Entry)
    private data class Cleanup(val calls: List<TranslationCall>, val callbacks: List<() -> Unit>) {
        fun run() {
            calls.forEach { it.cancel() }
            callbacks.forEach { it() }
        }
    }

    private val lock = java.lang.Object()
    private val pending = ArrayDeque<Entry>()
    private var active: Entry? = null
    private var activeAttempt: Attempt? = null
    private var activeCall: TranslationCall? = null
    private var activeDeadline: TranslationCall? = null
    private var retry: Retry? = null
    private var retryCall: TranslationCall? = null
    private var accepting = true
    private var stopped = false
    private var finishing = false
    private var finishDeadline: TranslationCall? = null
    private val finishCallbacks = mutableListOf<() -> Unit>()
    private var publishingThread: Thread? = null

    fun submit(text: String, language: String? = null, sourceUtteranceId: Long? = null) {
        val trimmed = text.trim()
        if (trimmed.isEmpty()) return
        var error: String? = null
        var cleanup: Cleanup? = null
        val next = synchronized(lock) {
            if (!accepting || stopped) return
            when {
                trimmed.length > MAX_TRANSLATION_TEXT_CHARS -> {
                    cleanup = terminateLocked()
                    error = "translation_too_large"
                    null
                }
                !FinalTranslationPolicy.admit(pending.size, accepting) -> {
                    // Reject only the new final. Previously accepted work retains
                    // its source identity; the service can report and gracefully drain.
                    accepting = false
                    error = "translation_queue_full"
                    null
                }
                else -> {
                    pending.addLast(Entry(trimmed, language, sourceUtteranceId, nanoTime()))
                    takeNextLocked()
                }
            }
        }
        cleanup?.run()
        error?.let(listener::onError)
        next?.let(::launch)
    }

    /** Immediate abort for errors, replacement or cancellation; never drain new work. */
    fun stop() {
        val cleanup = synchronized(lock) { terminateLocked() }
        cleanup.run()
    }

    /** Reject new finals and drain only accepted work, bounded by shared finish grace. */
    fun finish(onDrained: () -> Unit) {
        var immediate: Cleanup? = null
        val installTimer = synchronized(lock) {
            accepting = false
            if (stopped || (active == null && pending.isEmpty())) {
                immediate = terminateLocked().let { it.copy(callbacks = it.callbacks + onDrained) }
                false
            } else {
                finishCallbacks.add(onDrained)
                if (finishing) false else {
                    finishing = true
                    true
                }
            }
        }
        immediate?.run()
        if (!installTimer) return
        val timer = scheduleDeadline(TimeUnit.MILLISECONDS.toNanos(FinalTranslationPolicy.finishDrainMs)) { stop() }
        val owned = synchronized(lock) {
            if (stopped || !finishing) false else {
                finishDeadline = timer
                true
            }
        }
        if (!owned) timer.cancel()
    }

    /** All detached transport/timer handles are cancelled after leaving the state lock. */
    private fun terminateLocked(): Cleanup {
        accepting = false
        stopped = true
        pending.clear()
        active = null
        activeAttempt = null
        retry = null
        val calls = listOfNotNull(activeCall, activeDeadline, retryCall, finishDeadline)
        activeCall = null
        activeDeadline = null
        retryCall = null
        finishDeadline = null
        // A stop from another thread waits for an already-admitted delivery;
        // reentrant stop from that delivery must not wait on itself.
        var interrupted = false
        while (publishingThread != null && publishingThread !== Thread.currentThread()) {
            try { lock.wait() } catch (_: InterruptedException) { interrupted = true }
        }
        if (interrupted) Thread.currentThread().interrupt()
        val callbacks = finishCallbacks.toList()
        finishCallbacks.clear()
        return Cleanup(calls, callbacks)
    }

    private fun takeNextLocked(): Entry? {
        if (stopped || active != null || pending.isEmpty()) return null
        return pending.removeFirst().also { active = it }
    }

    private fun elapsedNanos(entry: Entry): Long = (nanoTime() - entry.enqueuedAtNanos).coerceAtLeast(0)
    private fun elapsedMs(entry: Entry): Long = TimeUnit.NANOSECONDS.toMillis(elapsedNanos(entry))
    private fun remainingNanos(entry: Entry): Long {
        val elapsed = elapsedNanos(entry)
        // Preserve the native clock's fractional millisecond without duplicating
        // the Rust policy's original-age subtraction or extending its deadline.
        val milliseconds = FinalTranslationPolicy.remaining(TimeUnit.NANOSECONDS.toMillis(elapsed))
        return TimeUnit.MILLISECONDS.toNanos(milliseconds) - elapsed % TimeUnit.MILLISECONDS.toNanos(1)
    }

    private fun expire(entry: Entry) {
        val cleanup = synchronized(lock) {
            if (stopped || active !== entry || publishingThread != null) return
            terminateLocked()
        }
        cleanup.run()
        listener.onError("translation_timeout")
    }

    private fun launch(entry: Entry) {
        var installWatchdog = false
        val attempt = synchronized(lock) {
            if (stopped || active !== entry || activeAttempt != null) return
            if (!FinalTranslationPolicy.start(elapsedMs(entry)) || remainingNanos(entry) <= 0) null else {
                entry.attempts++
                Attempt(entry, entry.attempts).also {
                    activeAttempt = it
                    installWatchdog = activeDeadline == null
                }
            }
        }
        if (attempt == null) {
            expire(entry)
            return
        }
        if (installWatchdog) {
            val remaining = synchronized(lock) {
                if (stopped || active !== entry || activeAttempt !== attempt) return
                remainingNanos(entry)
            }
            if (remaining <= 0) {
                expire(entry)
                return
            }
            val timer = scheduleDeadline(remaining) { expire(entry) }
            val owned = synchronized(lock) {
                if (stopped || active !== entry || activeAttempt !== attempt) false else {
                    activeDeadline = timer
                    true
                }
            }
            if (!owned) {
                timer.cancel()
                return
            }
        }
        // Enqueue/installation is guarded against abort. Synchronous validation
        // callbacks are deferred until this outer startup lock has been released.
        val started = synchronized(lock) {
            if (stopped || active !== entry || activeAttempt !== attempt) return
            if (remainingNanos(entry) <= 0) false else {
                activeCall = client.translate(entry.text, client.resolveSourceLanguage(sourceLanguage, entry.language), targetLanguage) { result ->
                    val deferred = synchronized(lock) {
                        if (stopped || active !== entry || activeAttempt !== attempt || attempt.claimed) return@translate
                        attempt.claimed = true
                        if (attempt.installing) {
                            attempt.immediateResult = result
                            true
                        } else false
                    }
                    if (!deferred) complete(attempt, result)
                }
                attempt.installing = false
                true
            }
        }
        if (!started) expire(entry)
        else attempt.immediateResult?.let { complete(attempt, it) }
    }

    private fun complete(attempt: Attempt, result: TranslationResult) {
        val entry = attempt.entry
        var cleanup: Cleanup? = null
        var error: String? = null
        var success: TranslationResult.Success? = null
        var retryToSchedule: Pair<Retry, Long>? = null
        var deadlineToCancel: TranslationCall? = null
        synchronized(lock) {
            if (stopped || active !== entry || activeAttempt !== attempt) return
            activeAttempt = null
            if (remainingNanos(entry) <= 0) {
                // A delayed watchdog cannot permit an expired final to publish.
                cleanup = terminateLocked()
                error = "translation_timeout"
            } else {
                // This callback has completed its handle; retain it only for an
                // expired completion's explicit cancellation, always outside lock.
                activeCall = null
                when (result) {
                    is TranslationResult.Success -> {
                        deadlineToCancel = activeDeadline
                        activeDeadline = null
                        success = result
                    }
                    is TranslationResult.Failure -> {
                        val decision = FinalTranslationPolicy.retry(result.code, attempt.number, elapsedMs(entry))
                        if (decision.getBoolean("retry")) {
                            val token = Retry(entry)
                            retry = token
                            retryToSchedule = token to decision.getLong("delay_ms")
                        } else {
                            cleanup = terminateLocked()
                            error = if (decision.getBoolean("expired") && !decision.getBoolean("exhausted")) "translation_timeout" else result.code
                        }
                    }
                }
            }
        }
        deadlineToCancel?.cancel()
        cleanup?.run()
        error?.let(listener::onError)
        retryToSchedule?.let { (token, delayMs) ->
            listener.onRetry(attempt.number + 1, delayMs)
            scheduleRetry(token, delayMs)
        }
        success?.let { publish(entry, it) }
    }

    private fun scheduleRetry(token: Retry, delayMs: Long) {
        val timer = scheduleDeadline(TimeUnit.MILLISECONDS.toNanos(delayMs)) {
            val owned = synchronized(lock) {
                if (stopped || active !== token.entry || retry !== token) false else {
                    retry = null
                    retryCall = null
                    true
                }
            }
            if (owned) launch(token.entry)
        }
        val owned = synchronized(lock) {
            if (stopped || active !== token.entry || retry !== token) false else {
                retryCall = timer
                true
            }
        }
        if (!owned) timer.cancel()
    }

    private fun publish(entry: Entry, result: TranslationResult.Success) {
        val admitted = synchronized(lock) {
            if (stopped || active !== entry) false else if (remainingNanos(entry) <= 0) false else {
                publishingThread = Thread.currentThread()
                true
            }
        }
        if (!admitted) {
            expire(entry)
            return
        }
        var next: Entry? = null
        var drained: Cleanup? = null
        try {
            listener.onTranslationForUtterance(entry.sourceUtteranceId, entry.text, entry.language, result.text, result.elapsedMs)
        } finally {
            synchronized(lock) {
                publishingThread = null
                lock.notifyAll()
                if (!stopped && active === entry) {
                    active = null
                    next = takeNextLocked()
                    if (finishing && next == null) drained = terminateLocked()
                }
            }
            drained?.run()
            next?.let(::launch)
        }
    }

    companion object {
        val MAX_PENDING_TRANSLATIONS: Int get() = FinalTranslationPolicy.maxWaiting
        val MAX_FINAL_AGE_SECONDS: Long get() = FinalTranslationPolicy.finalDeadlineMs / 1_000
    }
}

/** JNI calls execute the same Rust policy as desktop; no fallback Kotlin policy. */
private object FinalTranslationPolicy {
    private val policy: JSONObject get() = SharedSubtitleCore.policy
    val maxWaiting: Int get() = policy.getInt("max_final_queue_depth")
    val finalDeadlineMs: Long get() = policy.getLong("final_deadline_ms")
    val finishDrainMs: Long get() = policy.getLong("finish_drain_timeout_ms")
    private fun decision(kind: String, fields: JSONObject): JSONObject =
        SharedSubtitleCore.exchange(JSONObject().put("operation", fields.put("type", "decision").put("kind", kind))).getJSONObject("decision")
    fun admit(depth: Int, accepting: Boolean): Boolean = decision("admit", JSONObject().put("waiting_depth", depth).put("accepting", accepting)).getBoolean("admit")
    fun start(elapsedMs: Long): Boolean = decision("start", JSONObject().put("elapsed_ms", elapsedMs)).getBoolean("start")
    fun remaining(elapsedMs: Long): Long = decision("remaining", JSONObject().put("elapsed_ms", elapsedMs)).getLong("remaining_ms")
    fun retry(code: String, attempt: Int, elapsedMs: Long): JSONObject =
        decision("retry", JSONObject().put("code", code).put("attempt", attempt).put("elapsed_ms", elapsedMs))
}

/** One daemon for active finals; cancellation removes its queued task and captured source immediately. */
private object TranslationDeadlines {
    val executor = ScheduledThreadPoolExecutor(1) { task ->
        Thread(task, "mimi-translation-deadline").apply { isDaemon = true }
    }.apply { removeOnCancelPolicy = true }
}

private fun scheduleTranslationDeadline(delayNanos: Long, action: () -> Unit): TranslationCall {
    val task = TranslationDeadlines.executor.schedule(Runnable { action() }, delayNanos, TimeUnit.NANOSECONDS)
    return TranslationCall { task.cancel(false) }
}

/** Explicit user choices are authoritative; ASR aliases only refine automatic recognition. */
internal fun translationSourceLanguage(configured: String, reported: String?, supportedSources: Collection<String>? = null): String {
    if (configured != "auto") return configured
    val normalized = reported?.takeIf { it.length <= 64 }?.trim()?.lowercase() ?: return "auto"
    val code = when (normalized) {
        "chinese", "mandarin", "zh-hans", "zh-cn" -> "zh"
        "english" -> "en"
        "japanese" -> "ja"
        "korean" -> "ko"
        "fil", "filipino", "tagalog" -> "tl"
        "zh-hant", "zh-tw" -> "zh_tw"
        "nb" -> "no"
        else -> normalized
    }
    val registry = OPENAI_COMPATIBLE_TARGET_LANGUAGE_NAMES.keys
    val detected = registry.firstOrNull { it.equals(code, ignoreCase = true) }
        ?: code.substringBefore('-').takeIf { it in registry } ?: return "auto"
    return detected.takeIf { supportedSources == null || it in supportedSources } ?: "auto"
}
