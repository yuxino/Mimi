package app.yuxino.mimi.android.provider

import org.junit.Assert.*
import org.junit.Test
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger

class ProviderFinishGateTest {
    @Test fun repeatedBeginAndConcurrentAcknowledgementsCompleteTheOriginalCallbackOnce() {
        val gate = ProviderFinishGate()
        val original = AtomicInteger()
        val replacement = AtomicInteger()
        val successfulCompletions = AtomicInteger()
        val pool = Executors.newFixedThreadPool(4)
        val start = CountDownLatch(1)
        try {
            gate.begin { original.incrementAndGet() }
            gate.begin { replacement.incrementAndGet() }
            val completions = (1..4).map {
                pool.submit {
                    assertTrue(start.await(2, TimeUnit.SECONDS))
                    if (gate.complete()) successfulCompletions.incrementAndGet()
                }
            }
            start.countDown()
            completions.forEach { it.get(2, TimeUnit.SECONDS) }
            assertEquals(1, successfulCompletions.get())
            assertEquals(1, original.get())
            assertEquals(0, replacement.get())
            assertFalse(gate.complete())
        } finally {
            gate.cancel()
            pool.shutdownNow()
        }
    }

    @Test fun cancellationSuppressesAcknowledgementAndTheScheduledTimeout() {
        val gate = ProviderFinishGate()
        val completed = CountDownLatch(1)
        val timeoutMs = SharedSubtitleCore.policy.getLong("realtime_finish_timeout_ms")
        gate.begin { completed.countDown() }
        gate.cancel()
        assertFalse(gate.complete())
        assertFalse(completed.await(timeoutMs + 200, TimeUnit.MILLISECONDS))
        assertFalse(gate.complete())
    }

    @Test fun missingAcknowledgementFinishesWithinTheNativePolicyWindowOnce() {
        val gate = ProviderFinishGate()
        val completed = CountDownLatch(1)
        val count = AtomicInteger()
        val policy = SharedSubtitleCore.policy // Real host JNI; no Kotlin timeout fallback.
        val timeoutMs = policy.getLong("realtime_finish_timeout_ms")
        assertTrue(timeoutMs > 0)
        assertTrue(timeoutMs < policy.getLong("provider_finish_timeout_ms"))
        try {
            gate.begin {
                count.incrementAndGet()
                completed.countDown()
            }
            assertFalse(completed.await((timeoutMs / 4).coerceAtLeast(1), TimeUnit.MILLISECONDS))
            // Allow host scheduling slack while bounding a missing transport acknowledgement.
            assertTrue(completed.await(timeoutMs * 3 + 1_000, TimeUnit.MILLISECONDS))
            assertEquals(1, count.get())
            assertFalse(gate.complete())
            assertEquals(1, count.get())
        } finally {
            gate.cancel()
        }
    }

    @Test fun callbackRunsOutsideTheGateLock() {
        val gate = ProviderFinishGate()
        val pool = Executors.newSingleThreadExecutor()
        try {
            gate.begin {
                // A callback may close its provider from another thread without deadlocking.
                pool.submit { gate.cancel() }.get(2, TimeUnit.SECONDS)
            }
            assertTrue(gate.complete())
            assertFalse(gate.complete())
        } finally {
            gate.cancel()
            pool.shutdownNow()
        }
    }
}
