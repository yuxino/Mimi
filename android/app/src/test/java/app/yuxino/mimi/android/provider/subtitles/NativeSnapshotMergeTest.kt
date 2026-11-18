package app.yuxino.mimi.android.provider.subtitles

import app.yuxino.mimi.android.provider.NativeRuntimeConfiguration
import app.yuxino.mimi.android.provider.SharedSubtitleCore
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test

/** Actual JNI projection of two independently reduced source streams. */
class NativeSnapshotMergeTest {
    private fun lane(source: String, text: String): JSONObject {
        val created = SharedSubtitleCore.exchange(JSONObject().put("operation",
            JSONObject().put("type", "create").put("history_limit", 3)))
        val applied = SharedSubtitleCore.exchange(JSONObject().put("state", created.getString("state"))
            .put("operation", JSONObject().put("type", "apply").put("event", JSONObject()
                .put("type", "identified_final_pair").put("utterance_id", "shared-id")
                .put("source", text).put("translation", "$text translated"))))
        return JSONObject().put("audioSource", source).put("state", JSONObject()
            .put("snapshot", applied.getJSONObject("snapshot")).put("connectionStatus", "listening")
            .put("isTranslationPending", true))
    }
    private fun merge(rows: JSONArray, limit: Int = 3, stopped: Boolean = false): JSONObject =
        NativeRuntimeConfiguration.exchange(JSONObject().put("operation", "merge_snapshots")
            .put("snapshots", rows).put("historyLimit", limit).put("stopped", stopped))

    @Test fun bothSourcesPreserveIdentityAndBoundTheSharedDisplay() {
        val rows = JSONArray().put(lane("microphone", "Synthetic microphone"))
            .put(lane("system", "Synthetic system"))
        val merged = merge(rows, 1)
        val snapshot = merged.getJSONObject("snapshot")
        val tracks = snapshot.getJSONArray("tracks")
        assertEquals(2, tracks.length())
        assertEquals("system", tracks.getJSONObject(0).getString("audioSource"))
        assertEquals("Synthetic system", tracks.getJSONObject(0).getJSONObject("source").getString("text"))
        assertEquals("microphone", tracks.getJSONObject(1).getString("audioSource"))
        assertEquals("Synthetic microphone translated", tracks.getJSONObject(1).getJSONObject("translation").getString("text"))
        assertEquals(1, snapshot.getJSONArray("history").length())
        assertEquals("microphone", tracks.getJSONObject(1).getJSONArray("history").getJSONObject(0).getString("audioSource"))
        val stopped = merge(rows, 0, true)
        assertEquals("idle", stopped.getString("connectionStatus"))
        assertFalse(stopped.getBoolean("isTranslationPending"))
        assertEquals(0, stopped.getJSONObject("snapshot").getJSONArray("history").length())
        assertTrue(stopped.getJSONObject("snapshot").getBoolean("displayPairFinal"))
        assertEquals("Synthetic system", stopped.getJSONObject("snapshot").getJSONObject("source").getString("text"))
    }

    @Test fun duplicateSourcesAndUnboundedRequestsAreRejected() {
        for (rows in listOf(JSONArray().put(lane("system", "one")).put(lane("system", "two")),
            JSONArray().put(lane("system", "one")).put(lane("microphone", "two")).put(lane("system", "three")))) {
            try { merge(rows); fail("Invalid lanes accepted") } catch (_: IllegalArgumentException) { }
        }
        try { merge(JSONArray(), 7); fail("Unbounded history accepted") } catch (_: IllegalArgumentException) { }
    }

    @Test fun fullRuntimeHistoryIsReducedToTheAndroidDisplayLimit() {
        val input = lane("system", "Synthetic caption")
        val history = input.getJSONObject("state").getJSONObject("snapshot").getJSONArray("history")
        val first = history.getJSONObject(0)
        repeat(19) { index -> history.put(JSONObject(first.toString()).put("createdAt", index + 1L)) }
        val snapshot = merge(JSONArray().put(input), 3).getJSONObject("snapshot")
        assertEquals(3, snapshot.getJSONArray("history").length())
        assertEquals(3, snapshot.getJSONArray("tracks").getJSONObject(0).getJSONArray("history").length())
    }

    @Test fun stopWithZeroHistoryRestoresAcceptedTextBeforeNewerPreview() {
        val input = lane("system", "Accepted synthetic caption")
        input.getJSONObject("state").getJSONObject("snapshot").apply {
            put("displayPairFinal", false)
            put("displayPair", JSONObject().put("source", "New preview").put("translation", "Preview translation"))
        }
        val stopped = merge(JSONArray().put(input), 0, true).getJSONObject("snapshot")
        assertTrue(stopped.getBoolean("displayPairFinal"))
        assertEquals("Accepted synthetic caption", stopped.getJSONObject("source").getString("text"))
        assertEquals(0, stopped.getJSONArray("history").length())
    }
}
