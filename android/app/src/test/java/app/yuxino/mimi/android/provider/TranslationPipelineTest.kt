package app.yuxino.mimi.android.provider

import org.junit.Assert.*
import org.junit.Test
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.atomic.AtomicReference

class TranslationPipelineTest {
    private class ManualDeadlines {
        data class Task(val dueNanos: Long, val action: () -> Unit, var cancelled: Boolean = false, var fired: Boolean = false)
        var nowNanos = 0L
        val tasks = mutableListOf<Task>()
        val pendingCount: Int get() = tasks.count { !it.cancelled && !it.fired }
        fun schedule(delayNanos: Long, action: () -> Unit): TranslationCall {
            check(delayNanos > 0)
            val task = Task(nowNanos + delayNanos, action)
            tasks.add(task)
            return TranslationCall { task.cancelled = true }
        }
        fun advanceTo(milliseconds: Long, fireTimers: Boolean = true) {
            nowNanos = TimeUnit.MILLISECONDS.toNanos(milliseconds)
            if (!fireTimers) return
            while (true) {
                val task = tasks.firstOrNull { !it.cancelled && !it.fired && it.dueNanos <= nowNanos } ?: break
                task.fired = true
                task.action()
            }
        }
    }
    private class FakeClient : TranslationClient {
        data class Pending(val source: String, val language: String, val callback: (TranslationResult) -> Unit, var cancelled: Boolean = false)
        val calls = mutableListOf<Pending>()
        override fun translate(text: String, sourceLanguage: String, targetLanguage: String, callback: (TranslationResult) -> Unit): TranslationCall {
            val call = Pending(text, sourceLanguage, callback)
            calls.add(call)
            return TranslationCall { call.cancelled = true }
        }
    }
    private val deadlines = ManualDeadlines()
    private val client = FakeClient()
    private val results = mutableListOf<Pair<String, String>>()
    private val errors = mutableListOf<String>()
    private val pipeline = TranslationPipeline(client, "auto", "zh", object : TranslationPipeline.Listener {
        override fun onTranslation(source: String, language: String?, translation: String, elapsedMs: Long) {
            assertEquals(20L, elapsedMs)
            results.add(source to translation)
        }
        override fun onError(code: String) { errors.add(code) }
    }, { deadlines.nowNanos }, deadlines::schedule)

    @Test fun finalRequestsAreSerialAndPairedWithTheirOwnSource() {
        pipeline.submit("first", "en")
        pipeline.submit("second", "ja")
        assertEquals(1, client.calls.size)
        assertEquals("en", client.calls[0].language)
        client.calls[0].callback(TranslationResult.Success("一", 20))
        assertEquals(listOf("first" to "一"), results)
        assertEquals(2, client.calls.size)
        assertEquals("ja", client.calls[1].language)
        // A duplicated/out-of-order old response must never consume the next source.
        client.calls[0].callback(TranslationResult.Success("wrong", 20))
        client.calls[1].callback(TranslationResult.Success("二", 20))
        assertEquals(listOf("first" to "一", "second" to "二"), results)
    }

    @Test fun stopCancelsActiveWorkAndRejectsLateCompletions() {
        pipeline.submit("first")
        pipeline.submit("second")
        pipeline.stop()
        assertTrue(client.calls[0].cancelled)
        client.calls[0].callback(TranslationResult.Success("late", 20))
        pipeline.submit("third")
        assertTrue(results.isEmpty())
        assertEquals(1, client.calls.size)
        assertEquals(0, deadlines.pendingCount)
        deadlines.tasks.single().action() // Already-dispatched cancelled timers must also be harmless.
        assertTrue(errors.isEmpty())
    }

    @Test fun authenticationFailureStopsWithoutRetryOrMispairing() {
        pipeline.submit("first")
        pipeline.submit("second")
        client.calls[0].callback(TranslationResult.Failure("translation_http_401", 20))
        client.calls[0].callback(TranslationResult.Success("late", 20))
        assertEquals(listOf("translation_http_401"), errors)
        assertEquals(1, client.calls.size)
        assertTrue(results.isEmpty())
        assertEquals(0, deadlines.pendingCount)
    }

    @Test fun overflowRejectsOnlyTheNewSourceAndPreservesAcceptedFinals() {
        assertEquals(3, TranslationPipeline.MAX_PENDING_TRANSLATIONS)
        pipeline.submit("active")
        repeat(TranslationPipeline.MAX_PENDING_TRANSLATIONS) { pipeline.submit("pending $it") }
        pipeline.submit("overflow")
        assertEquals(listOf("translation_queue_full"), errors)
        assertFalse(client.calls[0].cancelled)
        var drained = 0
        pipeline.finish { drained++ }
        repeat(4) { index -> client.calls[index].callback(TranslationResult.Success("result $index", 20)) }
        assertEquals(listOf("active", "pending 0", "pending 1", "pending 2"), results.map { it.first })
        assertEquals(4, client.calls.size)
        assertEquals(1, drained)
        assertEquals(0, deadlines.pendingCount)
    }

    @Test fun blankFinalsAreIgnoredAndOversizedFinalsFailWithoutANetworkRequest() {
        pipeline.submit("  ")
        pipeline.submit("x".repeat(MAX_TRANSLATION_TEXT_CHARS + 1))
        assertTrue(client.calls.isEmpty())
        assertEquals(listOf("translation_too_large"), errors)
    }

    @Test fun synchronousValidationFailureDoesNotLeaveAnActiveSlotOrStartMoreWork() {
        var calls = 0
        val immediate = object : TranslationClient {
            override fun translate(text: String, sourceLanguage: String, targetLanguage: String, callback: (TranslationResult) -> Unit): TranslationCall {
                calls++
                callback(TranslationResult.Failure("translation_endpoint", 0))
                return TranslationCall { }
            }
        }
        val queue = TranslationPipeline(immediate, "en", "zh", object : TranslationPipeline.Listener {
            override fun onTranslation(source: String, language: String?, translation: String, elapsedMs: Long) { fail() }
            override fun onError(code: String) { errors.add(code) }
        }, { deadlines.nowNanos }, deadlines::schedule)
        queue.submit("first")
        queue.submit("second")
        assertEquals(1, calls)
        assertEquals(listOf("translation_endpoint"), errors)
        assertEquals(0, deadlines.pendingCount)
    }
    @Test fun explicitSourceWinsAndAutomaticSourceNormalizesAsrAliases() {
        assertEquals("ja", translationSourceLanguage("ja", "English"))
        assertEquals("en", translationSourceLanguage("auto", "en-US"))
        assertEquals("ja", translationSourceLanguage("auto", "ja-JP"))
        assertEquals("zh", translationSourceLanguage("auto", "Chinese"))
        assertEquals("zh", translationSourceLanguage("auto", "Mandarin"))
        assertEquals("en", translationSourceLanguage("auto", "English"))
        assertEquals("ko", translationSourceLanguage("auto", "Korean"))
        for (label in listOf(null, "", "unknown provider language", "fr-FR", "x".repeat(100))) {
            assertEquals("auto", translationSourceLanguage("auto", label))
        }
        pipeline.submit("synthetic", "ja-JP")
        assertEquals("ja", client.calls.single().language)
    }

    @Test fun queueWaitingConsumesTheSameFinalDeadlineAndCancelsStalledHttp() {
        pipeline.submit("first", "en")
        deadlines.advanceTo(1_000)
        pipeline.submit("second", "ja")
        pipeline.submit("third", "en")
        deadlines.advanceTo(30_000)
        client.calls[0].callback(TranslationResult.Success("一", 20))
        assertEquals(listOf("first" to "一"), results)
        assertEquals(2, client.calls.size)
        assertTrue(deadlines.tasks[0].cancelled)
        assertEquals(TimeUnit.SECONDS.toNanos(46), deadlines.tasks[1].dueNanos)
        assertEquals(1, deadlines.pendingCount)
        deadlines.tasks[0].action() // A timer racing with cancellation cannot expire its successor.
        assertTrue(errors.isEmpty())
        deadlines.advanceTo(45_999)
        assertFalse(client.calls[1].cancelled)
        deadlines.advanceTo(46_000)
        assertTrue(client.calls[1].cancelled)
        assertEquals(listOf("translation_timeout"), errors)
        assertEquals(0, deadlines.pendingCount)
        client.calls[1].callback(TranslationResult.Success("late", 20))
        pipeline.submit("after expiry")
        assertEquals(2, client.calls.size)
        assertEquals(listOf("first" to "一"), results)
    }

    @Test fun anExpiredResponseCannotPublishEvenWhenTheDeadlineThreadHasNotRun() {
        pipeline.submit("first")
        pipeline.submit("second")
        deadlines.advanceTo(45_000, fireTimers = false)
        client.calls[0].callback(TranslationResult.Success("expired", 20))
        assertTrue(results.isEmpty())
        assertEquals(listOf("translation_timeout"), errors)
        assertTrue(client.calls[0].cancelled)
        assertEquals(0, deadlines.pendingCount)
        deadlines.tasks.single().action()
        assertEquals(listOf("translation_timeout"), errors)
        assertEquals(1, client.calls.size)
    }

    @Test fun aQueuedFinalThatExpiresDuringDeliveryNeverStartsAnotherHttpRequest() {
        val queue = TranslationPipeline(client, "en", "zh", object : TranslationPipeline.Listener {
            override fun onTranslation(source: String, language: String?, translation: String, elapsedMs: Long) {
                results.add(source to translation)
                // Simulate descheduling after publishing the first result but before starting its successor.
                deadlines.advanceTo(46_000, fireTimers = false)
            }
            override fun onError(code: String) { errors.add(code) }
        }, { deadlines.nowNanos }, deadlines::schedule)
        queue.submit("first")
        queue.submit("second")
        deadlines.advanceTo(44_000)
        client.calls[0].callback(TranslationResult.Success("一", 20))
        assertEquals(listOf("first" to "一"), results)
        assertEquals(listOf("translation_timeout"), errors)
        assertEquals(1, client.calls.size)
        assertEquals(0, deadlines.pendingCount)
    }

    @Test fun expiryWhileInstallingTheWatchdogDoesNotStartHttp() {
        val queue = TranslationPipeline(client, "en", "zh", object : TranslationPipeline.Listener {
            override fun onTranslation(source: String, language: String?, translation: String, elapsedMs: Long) { fail() }
            override fun onError(code: String) { errors.add(code) }
        }, { deadlines.nowNanos }, { delay, action ->
            deadlines.schedule(delay, action).also { deadlines.advanceTo(45_000, fireTimers = false) }
        })
        queue.submit("expired before start")
        assertTrue(client.calls.isEmpty())
        assertEquals(listOf("translation_timeout"), errors)
        assertEquals(0, deadlines.pendingCount)
    }

    @Test fun completingAllFinalsRemovesEveryDeadlineTask() {
        pipeline.submit("first")
        pipeline.submit("second")
        client.calls[0].callback(TranslationResult.Success("一", 20))
        client.calls[1].callback(TranslationResult.Success("二", 20))
        assertEquals(listOf("first" to "一", "second" to "二"), results)
        assertEquals(0, deadlines.pendingCount)
        deadlines.advanceTo(100_000)
        assertTrue(errors.isEmpty())
    }

    @Test fun stopWaitsForRequestStartupAndCancelsBeforeReturning() = assertStartupCancellation(expire = false)

    @Test fun deadlineWaitsForRequestStartupAndCancelsBeforeReturning() = assertStartupCancellation(expire = true)

    private fun assertStartupCancellation(expire: Boolean) {
        val startupEntered = CountDownLatch(1)
        val allowEnqueue = CountDownLatch(1)
        val cancellationEntered = CountDownLatch(1)
        val cancellationReturned = CountDownLatch(1)
        val startedRequests = AtomicInteger()
        val startsAfterCancellation = AtomicInteger()
        val cancelled = AtomicBoolean()
        val clock = AtomicLong()
        val deadline = AtomicReference<() -> Unit>()
        val callbackRef = AtomicReference<(TranslationResult) -> Unit>()
        val error = AtomicReference<String>()
        val delivered = AtomicInteger()
        val blockedClient = object : TranslationClient {
            override fun translate(text: String, sourceLanguage: String, targetLanguage: String, callback: (TranslationResult) -> Unit): TranslationCall {
                callbackRef.set(callback)
                // Simulate synchronous request construction before OkHttp's enqueue.
                startupEntered.countDown()
                check(allowEnqueue.await(3, TimeUnit.SECONDS))
                if (cancellationReturned.count == 0L) startsAfterCancellation.incrementAndGet()
                startedRequests.incrementAndGet()
                return TranslationCall { cancelled.set(true) }
            }
        }
        val queue = TranslationPipeline(blockedClient, "en", "zh", object : TranslationPipeline.Listener {
            override fun onTranslation(source: String, language: String?, translation: String, elapsedMs: Long) { delivered.incrementAndGet() }
            override fun onError(code: String) { error.set(code) }
        }, clock::get, { _, action ->
            deadline.set(action)
            TranslationCall { }
        })
        val executor = Executors.newFixedThreadPool(2)
        try {
            val submit = executor.submit { queue.submit("synthetic source") }
            assertTrue(startupEntered.await(3, TimeUnit.SECONDS))
            val stop = executor.submit {
                cancellationEntered.countDown()
                if (expire) {
                    clock.set(TimeUnit.SECONDS.toNanos(TranslationPipeline.MAX_FINAL_AGE_SECONDS))
                    deadline.get().invoke()
                } else queue.stop()
                cancellationReturned.countDown()
            }
            assertTrue(cancellationEntered.await(3, TimeUnit.SECONDS))
            // The old implementation returned here with no registered call to cancel.
            assertFalse(cancellationReturned.await(100, TimeUnit.MILLISECONDS))
            allowEnqueue.countDown()
            submit.get(3, TimeUnit.SECONDS)
            stop.get(3, TimeUnit.SECONDS)
            assertEquals(1, startedRequests.get())
            assertEquals(0, startsAfterCancellation.get())
            assertTrue(cancelled.get())
            callbackRef.get().invoke(TranslationResult.Success("late", 20))
            queue.submit("after cancellation")
            assertEquals(1, startedRequests.get())
            assertEquals(0, delivered.get())
            assertEquals(if (expire) "translation_timeout" else null, error.get())
        } finally {
            allowEnqueue.countDown()
            queue.stop()
            executor.shutdownNow()
            assertTrue(executor.awaitTermination(3, TimeUnit.SECONDS))
        }
    }

    @Test fun synchronousFailureCleansUpWithoutHoldingTheLifecycleLock() {
        val executor = Executors.newSingleThreadExecutor()
        lateinit var queue: TranslationPipeline
        val immediateFailure = object : TranslationClient {
            override fun translate(text: String, sourceLanguage: String, targetLanguage: String, callback: (TranslationResult) -> Unit): TranslationCall {
                callback(TranslationResult.Failure("translation_endpoint", 0))
                return TranslationCall { }
            }
        }
        var deadlineCancelled = false
        queue = TranslationPipeline(immediateFailure, "en", "zh", object : TranslationPipeline.Listener {
            override fun onTranslation(source: String, language: String?, translation: String, elapsedMs: Long) { fail() }
            override fun onError(code: String) { errors.add(code) }
        }, { 0L }, { _, _ -> TranslationCall {
            // Cancellation must not keep an outer startup lock from a synchronous callback.
            executor.submit { queue.stop() }.get(3, TimeUnit.SECONDS)
            deadlineCancelled = true
        } })
        try {
            queue.submit("synthetic source")
            assertTrue(deadlineCancelled)
            assertEquals(listOf("translation_endpoint"), errors)
        } finally {
            executor.shutdownNow()
            assertTrue(executor.awaitTermination(3, TimeUnit.SECONDS))
        }
    }

    @Test fun recoverableRetriesKeepSourceAndRejectOldAttemptCallbacks() {
        pipeline.submit("first", "ja", 42L)
        pipeline.submit("second", "en", 43L)
        client.calls[0].callback(TranslationResult.Failure("translation_network", 20))
        assertTrue(errors.isEmpty())
        deadlines.advanceTo(599)
        assertEquals(1, client.calls.size)
        deadlines.advanceTo(600)
        assertEquals(2, client.calls.size)
        assertEquals("first", client.calls[1].source)
        assertEquals("ja", client.calls[1].language)
        client.calls[0].callback(TranslationResult.Success("stale attempt", 20))
        assertTrue(results.isEmpty())
        client.calls[1].callback(TranslationResult.Failure("translation_http_503", 20))
        deadlines.advanceTo(1_799)
        assertEquals(2, client.calls.size)
        deadlines.advanceTo(1_800)
        assertEquals(3, client.calls.size)
        client.calls[2].callback(TranslationResult.Success("一", 20))
        assertEquals(listOf("first" to "一"), results)
        assertEquals("second", client.calls[3].source)
        client.calls[3].callback(TranslationResult.Success("二", 20))
        assertEquals(listOf("first" to "一", "second" to "二"), results)
        assertEquals(2, deadlines.tasks.count { it.dueNanos == TimeUnit.SECONDS.toNanos(45) })
        assertEquals(0, deadlines.pendingCount)
    }

    @Test fun threeAttemptsExhaustRecoveryAndDoNotStartTheQueuedSource() {
        pipeline.submit("first")
        pipeline.submit("second")
        client.calls[0].callback(TranslationResult.Failure("translation_timeout", 20))
        deadlines.advanceTo(600)
        client.calls[1].callback(TranslationResult.Failure("translation_network", 20))
        deadlines.advanceTo(1_800)
        client.calls[2].callback(TranslationResult.Failure("translation_http_503", 20))
        assertEquals(listOf("translation_http_503"), errors)
        assertEquals(3, client.calls.size)
        assertTrue(results.isEmpty())
        assertEquals(0, deadlines.pendingCount)
        deadlines.tasks.forEach { it.action() }
        assertEquals(3, client.calls.size)
    }

    @Test fun rateLimitsUseSharedFourThenEightSecondBackoff() {
        pipeline.submit("source")
        client.calls[0].callback(TranslationResult.Failure("translation_http_429", 20))
        deadlines.advanceTo(3_999)
        assertEquals(1, client.calls.size)
        deadlines.advanceTo(4_000)
        client.calls[1].callback(TranslationResult.Failure("translation_rejected_429", 20))
        deadlines.advanceTo(11_999)
        assertEquals(2, client.calls.size)
        deadlines.advanceTo(12_000)
        client.calls[2].callback(TranslationResult.Success("complete", 20))
        assertEquals(listOf("source" to "complete"), results)
        assertTrue(errors.isEmpty())
    }

    @Test fun queueTimeAndPreviousAttemptsConsumeTheOriginalRetryBudget() {
        pipeline.submit("first")
        deadlines.advanceTo(1_000)
        pipeline.submit("second")
        deadlines.advanceTo(30_000)
        client.calls[0].callback(TranslationResult.Success("一", 20))
        deadlines.advanceTo(45_500, fireTimers = false)
        client.calls[1].callback(TranslationResult.Failure("translation_network", 20))
        assertEquals(listOf("translation_timeout"), errors)
        assertEquals(2, client.calls.size)
        assertEquals(listOf("first" to "一"), results)
        assertEquals(0, deadlines.pendingCount)
    }

    @Test fun abortDuringBackoffCancelsTimersAndRejectsStaleRetry() {
        pipeline.submit("source")
        client.calls[0].callback(TranslationResult.Failure("translation_network", 20))
        val retry = deadlines.tasks.last()
        pipeline.stop()
        assertEquals(0, deadlines.pendingCount)
        retry.action()
        client.calls[0].callback(TranslationResult.Success("late", 20))
        deadlines.advanceTo(100_000)
        assertEquals(1, client.calls.size)
        assertTrue(results.isEmpty())
        assertTrue(errors.isEmpty())
    }

    @Test fun finishDrainsAcceptedWorkRejectsNewSubmissionsAndCallsEachCompletionOnce() {
        pipeline.submit("first")
        pipeline.submit("second")
        var drainedA = 0
        var drainedB = 0
        pipeline.finish { drainedA++ }
        deadlines.advanceTo(1_000)
        pipeline.finish { drainedB++ }
        pipeline.submit("not accepted")
        client.calls[0].callback(TranslationResult.Success("一", 20))
        assertEquals(0, drainedA)
        client.calls[1].callback(TranslationResult.Success("二", 20))
        assertEquals(listOf("first" to "一", "second" to "二"), results)
        assertEquals(1, drainedA)
        assertEquals(1, drainedB)
        assertEquals(2, client.calls.size)
        assertEquals(0, deadlines.pendingCount)
        deadlines.tasks.forEach { it.action() }
        assertEquals(1, drainedA)
        assertEquals(1, drainedB)
    }

    @Test fun finishGraceAbortsStalledAcceptedWorkWithoutResettingItsAgeBudget() {
        pipeline.submit("first")
        pipeline.submit("second")
        var drained = 0
        pipeline.finish { drained++ }
        deadlines.advanceTo(2_999)
        assertFalse(client.calls[0].cancelled)
        assertEquals(0, drained)
        deadlines.advanceTo(3_000)
        assertTrue(client.calls[0].cancelled)
        assertEquals(1, drained)
        assertEquals(0, deadlines.pendingCount)
        client.calls[0].callback(TranslationResult.Success("late", 20))
        assertEquals(1, client.calls.size)
        assertTrue(results.isEmpty())
    }

    @Test fun finishCanDrainARecoverableRetryWithinTheSameGraceAndOriginalBudget() {
        pipeline.submit("source")
        client.calls[0].callback(TranslationResult.Failure("translation_network", 20))
        var drained = 0
        pipeline.finish { drained++ }
        deadlines.advanceTo(600)
        client.calls[1].callback(TranslationResult.Success("complete", 20))
        assertEquals(listOf("source" to "complete"), results)
        assertEquals(1, drained)
        assertEquals(0, deadlines.pendingCount)
    }

    @Test fun identicalTextRetainsDistinctSourceUtteranceIdentities() {
        val identities = mutableListOf<Long?>()
        val queue = TranslationPipeline(client, "en", "zh", object : TranslationPipeline.Listener {
            override fun onTranslation(source: String, language: String?, translation: String, elapsedMs: Long) { fail() }
            override fun onTranslationForUtterance(sourceUtteranceId: Long?, source: String, language: String?, translation: String, elapsedMs: Long) {
                identities.add(sourceUtteranceId)
                results.add(source to translation)
            }
            override fun onError(code: String) { errors.add(code) }
        }, { deadlines.nowNanos }, deadlines::schedule)
        queue.submit("same words", "en", 11L)
        queue.submit("same words", "en", 12L)
        client.calls[0].callback(TranslationResult.Success("first", 20))
        client.calls[1].callback(TranslationResult.Success("second", 20))
        assertEquals(listOf(11L, 12L), identities)
        assertEquals(listOf("same words" to "first", "same words" to "second"), results)
    }

    @Test fun successCleanupCancelsWatchdogOutsideTheStateLock() {
        val executor = Executors.newSingleThreadExecutor()
        lateinit var queue: TranslationPipeline
        queue = TranslationPipeline(client, "en", "zh", object : TranslationPipeline.Listener {
            override fun onTranslation(source: String, language: String?, translation: String, elapsedMs: Long) { results.add(source to translation) }
            override fun onError(code: String) { errors.add(code) }
        }, { 0L }, { _, _ -> TranslationCall {
            // Abort while completing: if cancellation retains the state lock,
            // this independent stopper deadlocks rather than rejecting delivery.
            executor.submit { queue.stop() }.get(3, TimeUnit.SECONDS)
        } })
        try {
            queue.submit("source")
            client.calls[0].callback(TranslationResult.Success("complete", 20))
            assertTrue(results.isEmpty())
        } finally {
            queue.stop()
            executor.shutdownNow()
            assertTrue(executor.awaitTermination(3, TimeUnit.SECONDS))
        }
    }

    @Test fun stopWaitsForAnAdmittedDeliveryWithoutHoldingTheStateLock() {
        val entered = CountDownLatch(1)
        val release = CountDownLatch(1)
        val executor = Executors.newFixedThreadPool(2)
        val queue = TranslationPipeline(client, "en", "zh", object : TranslationPipeline.Listener {
            override fun onTranslation(source: String, language: String?, translation: String, elapsedMs: Long) {
                entered.countDown()
                check(release.await(3, TimeUnit.SECONDS))
                results.add(source to translation)
            }
            override fun onError(code: String) { errors.add(code) }
        }, { deadlines.nowNanos }, deadlines::schedule)
        try {
            queue.submit("source")
            queue.submit("queued")
            val complete = executor.submit { client.calls[0].callback(TranslationResult.Success("complete", 20)) }
            assertTrue(entered.await(3, TimeUnit.SECONDS))
            val stopped = CountDownLatch(1)
            val stop = executor.submit { queue.stop(); stopped.countDown() }
            assertFalse(stopped.await(100, TimeUnit.MILLISECONDS))
            release.countDown()
            complete.get(3, TimeUnit.SECONDS)
            stop.get(3, TimeUnit.SECONDS)
            assertEquals(listOf("source" to "complete"), results)
            assertEquals(1, client.calls.size)
        } finally {
            release.countDown()
            queue.stop()
            executor.shutdownNow()
            assertTrue(executor.awaitTermination(3, TimeUnit.SECONDS))
        }
    }

}
