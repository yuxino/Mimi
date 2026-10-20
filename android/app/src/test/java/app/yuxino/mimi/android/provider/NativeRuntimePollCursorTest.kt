package app.yuxino.mimi.android.provider

import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test

class NativeRuntimePollCursorTest {
    private fun full(version: Long, finished: Boolean = false, error: String? = null): JSONObject =
        JSONObject().put("version", version).put("snapshot", JSONObject().put("history", "synthetic-state"))
            .put("finished", finished).put("errorCode", error ?: JSONObject.NULL)

    private fun light(version: Long, finished: Boolean = false, error: String? = null): JSONObject =
        JSONObject().put("unchanged", true).put("version", version)
            .put("finished", finished).put("errorCode", error ?: JSONObject.NULL)

    @Test fun firstPollOmitsSinceVersionAndSubsequentRequestsUseTheDeliveredFullRevision() {
        val cursor = NativeRuntimePollCursor()
        val first = runtimePollRequest(7, cursor.lastFullRevision)
        assertEquals("poll", first.getString("operation"))
        assertEquals(7L, first.getLong("handle"))
        assertFalse(first.has("sinceVersion"))
        cursor.consume(full(3), {}, {})
        assertEquals(3L, runtimePollRequest(7, cursor.lastFullRevision).getLong("sinceVersion"))
        assertEquals(0L, runtimePollRequest(7, 0).getLong("sinceVersion"))
        assertEquals("native_runtime_invalid_request", assertThrows(IllegalArgumentException::class.java) {
            runtimePollRequest(7, -1)
        }.message)
    }

    @Test fun legacyFullRepliesAndLightRepliesOnlyNotifyOnSnapshotChanges() {
        val cursor = NativeRuntimePollCursor()
        val delivered = mutableListOf<JSONObject>()
        val first = full(3)
        assertFalse(cursor.consume(first, delivered::add) { fail("Unexpected terminal") })
        assertSame(first, delivered.single())
        assertFalse(cursor.consume(full(3), delivered::add) { fail("Unexpected terminal") })
        assertFalse(cursor.consume(light(3), delivered::add) { fail("Unexpected terminal") })
        val changed = full(4)
        assertFalse(cursor.consume(changed, delivered::add) { fail("Unexpected terminal") })
        assertEquals(listOf(first, changed), delivered)
        assertEquals(4L, cursor.lastFullRevision)
    }

    @Test fun sameVersionLightRepliesDeliverLateFinishAndErrorWithoutAnotherSnapshot() {
        for (error in listOf(null, "credential_authentication_failed")) {
            val cursor = NativeRuntimePollCursor()
            val events = mutableListOf<String>()
            cursor.consume(full(8), { events += "snapshot" }, { fail("Unexpected terminal") })
            assertTrue(cursor.consume(light(8, finished = true, error = error),
                { fail("Light reply reached snapshot listener") }, { code ->
                    assertEquals(error, code)
                    events += "terminal"
                }))
            assertEquals(listOf("snapshot", "terminal"), events)
            assertEquals(8L, cursor.lastFullRevision)
        }
    }

    @Test fun sameVersionLegacyTerminalFullDeliversTheFinalSnapshotBeforeItsCallback() {
        val cursor = NativeRuntimePollCursor()
        val events = mutableListOf<String>()
        cursor.consume(full(8), {}, {})
        val terminal = full(8, finished = true)
        assertTrue(cursor.consume(terminal, {
            assertSame(terminal, it)
            events += "final-snapshot"
        }, { events += "terminal" }))
        assertEquals(listOf("final-snapshot", "terminal"), events)
    }

    @Test fun failedSnapshotDeliveryDoesNotAdvanceTheNextPollRevision() {
        val cursor = NativeRuntimePollCursor()
        cursor.consume(full(2), {}, {})
        val failure = IllegalStateException("synthetic-listener-failure")
        assertSame(failure, assertThrows(IllegalStateException::class.java) {
            cursor.consume(full(3), { throw failure }, {})
        })
        assertEquals(2L, cursor.lastFullRevision)
        assertEquals(2L, runtimePollRequest(7, cursor.lastFullRevision).getLong("sinceVersion"))
    }

    @Test fun invalidLightRepliesAreRejectedWithoutAdvancingOrCallingListeners() {
        val cursor = NativeRuntimePollCursor()
        fun reject(state: JSONObject) {
            assertEquals("native_runtime_invalid_response", assertThrows(IllegalArgumentException::class.java) {
                cursor.consume(state, { fail("Invalid reply reached snapshot listener") }, { fail("Invalid reply reached terminal listener") })
            }.message)
        }
        reject(light(3))
        assertNull(cursor.lastFullRevision)
        cursor.consume(full(3), {}, {})
        reject(light(4))
        reject(light(4, finished = true, error = "transport_error"))
        reject(light(3).put("snapshot", JSONObject()))
        reject(light(3).apply { remove("finished") })
        reject(light(3).apply { remove("errorCode") })
        assertEquals(3L, cursor.lastFullRevision)
    }
}
