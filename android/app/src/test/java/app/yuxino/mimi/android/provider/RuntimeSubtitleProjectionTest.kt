package app.yuxino.mimi.android.provider

import org.json.JSONObject
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.runtimeFeedbackResource

/** Feed real Rust reducer snapshots through the production session-to-UI boundary. */
class RuntimeSubtitleProjectionTest {
    @Before fun reset() { SubtitleBus.clear(); SubtitleBus.setHistoryLimit(2) }
    @After fun cleanup() { SubtitleBus.clear(); SubtitleBus.setHistoryLimit(0) }

    private class Snapshots(private val atomic: Boolean) {
        private var response = SharedSubtitleCore.exchange(JSONObject().put("operation",
            JSONObject().put("type", "create").put("history_limit", 2)))
        fun apply(event: JSONObject) {
            response = SharedSubtitleCore.exchange(JSONObject().put("state", response.getString("state"))
                .put("operation", JSONObject().put("type", "apply").put("event", event)))
            SubtitleBus.onRuntimeSnapshot(JSONObject().put("atomicPreview", atomic).put("detectedLanguage", "en")
                .put("snapshot", response.getJSONObject("snapshot")), false)
        }
        fun feedback(fields: JSONObject) = SubtitleBus.onRuntimeSnapshot(fields.put("atomicPreview", atomic)
            .put("snapshot", response.getJSONObject("snapshot")), false)
    }
    private fun pair(id: String, source: String, translation: String) = JSONObject()
        .put("type", "identified_final_pair").put("utterance_id", id).put("source", source).put("translation", translation)
    private fun draft(role: String, text: String) = JSONObject().put("type", "utterance_text")
        .put("utterance_id", "B").put("role", role).put("text", text).put("is_final", false)

    @Test fun recoveryFeedbackPreservesCaptionsAndDistinguishesScheduledRetriesFromNewSpeech() {
        val snapshots = Snapshots(true)
        snapshots.apply(pair("A", "Synthetic A.", "合成 A。"))
        for ((reason, retryResource, waitingResource) in listOf(
            Triple("rateLimited", R.string.runtime_rate_retry, R.string.runtime_rate_wait),
            Triple("temporarilyUnavailable", R.string.runtime_unavailable_retry, R.string.runtime_unavailable_wait),
        )) {
            for (scheduled in listOf(true, false)) {
                snapshots.feedback(JSONObject().put("connectionStatus", "listening")
                    .put("translationRecovery", JSONObject().put("reason", reason).put("retryScheduled", scheduled)))
                assertEquals(if (scheduled) retryResource else waitingResource, runtimeFeedbackResource(SubtitleBus.runtimeFeedback))
                assertEquals("Synthetic A.", SubtitleBus.displaySource)
                assertEquals("合成 A。", SubtitleBus.displayTranslation)
            }
        }
        snapshots.feedback(JSONObject().put("connectionStatus", "connecting"))
        assertEquals(R.string.runtime_connecting, runtimeFeedbackResource(SubtitleBus.runtimeFeedback))
        snapshots.feedback(JSONObject().put("connectionStatus", "listening").put("isTranslationTimedOut", true))
        assertEquals(R.string.runtime_translation_delayed, runtimeFeedbackResource(SubtitleBus.runtimeFeedback))
        snapshots.feedback(JSONObject().put("connectionStatus", "listening"))
        assertNull(runtimeFeedbackResource(SubtitleBus.runtimeFeedback))
        snapshots.feedback(JSONObject().put("translationRecovery", JSONObject().put("reason", "synthetic-private-error")))
        assertNull(runtimeFeedbackResource(SubtitleBus.runtimeFeedback))
        SubtitleBus.clear()
        assertNull(runtimeFeedbackResource(SubtitleBus.runtimeFeedback))
    }

    @Test fun atomicRuntimeRetainsTheCompletePairUntilTheNextCompletePreviewArrives() {
        val snapshots = Snapshots(true)
        snapshots.apply(pair("A", "Synthetic A.", "合成 A。"))
        snapshots.apply(draft("source", "Synthetic B."))
        snapshots.apply(draft("translation", "合成 B。"))
        assertEquals("Synthetic A.", SubtitleBus.displaySource)
        assertEquals("合成 A。", SubtitleBus.displayTranslation)
        assertTrue(SubtitleBus.displayPairFinal)
        assertEquals("Synthetic B.", SubtitleBus.sourceDraft)
        snapshots.apply(JSONObject().put("type", "preview_pair").put("source_utterance_id", JSONObject.NULL)
            .put("source", "Synthetic B.").put("translation", "合成 B。"))
        assertEquals("Synthetic B.", SubtitleBus.displaySource)
        assertEquals("合成 B。", SubtitleBus.displayTranslation)
        assertFalse(SubtitleBus.displayPairFinal)
        assertEquals(1, SubtitleBus.historySnapshot().size)
        SubtitleBus.setHistoryLimit(1)
        assertEquals("合成 B。", SubtitleBus.displayTranslation)
        snapshots.apply(pair("B", "Synthetic B.", "合成 B。"))
        assertTrue(SubtitleBus.displayPairFinal)
        assertEquals("合成 B。", SubtitleBus.historySnapshot().single().translation)
    }

    @Test fun independentLaneRuntimeAdvancesDraftsAndCannotCombineDifferentSentenceOwners() {
        val snapshots = Snapshots(false)
        snapshots.apply(pair("A", "Synthetic A.", "合成 A。"))
        snapshots.apply(draft("source", "Synthetic B."))
        assertEquals("Synthetic B.", SubtitleBus.displaySource)
        assertEquals("", SubtitleBus.displayTranslation)
        assertFalse(SubtitleBus.displayPairFinal)
        snapshots.apply(draft("translation", "合成 B。"))
        assertEquals("Synthetic B.", SubtitleBus.displaySource)
        assertEquals("合成 B。", SubtitleBus.displayTranslation)
        assertEquals(1, SubtitleBus.historySnapshot().size)
        SubtitleBus.setHistoryLimit(1)
        assertEquals("Synthetic B.", SubtitleBus.displaySource)
        assertFalse(SubtitleBus.displayPairFinal)
        snapshots.apply(pair("B", "Synthetic B.", "合成 B。"))
        assertTrue(SubtitleBus.displayPairFinal)
        assertEquals("合成 B。", SubtitleBus.historySnapshot().single().translation)
    }
}
