package app.yuxino.mimi.android.provider

import org.json.JSONObject
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.runtimeFeedbackResource
import app.yuxino.mimi.android.CaptionDisplayMode
import app.yuxino.mimi.android.forReading

/** Feed real Rust reducer snapshots through the production session-to-UI boundary. */
class RuntimeSubtitleProjectionTest {
    @Before fun reset() { SubtitleBus.clear(); SubtitleBus.setHistoryLimit(2) }
    @After fun cleanup() { SubtitleBus.clear(); SubtitleBus.setHistoryLimit(0) }

    private class Snapshots(private val atomic: Boolean, private val historyLimit: Int = 2, private val audioSource: String = "system") {
        private var response = SharedSubtitleCore.exchange(JSONObject().put("operation",
            JSONObject().put("type", "create").put("history_limit", historyLimit)))
        fun apply(event: JSONObject, publish: Boolean = true) =
            operation(JSONObject().put("type", "apply").put("event", event), publish)
        fun restore(publish: Boolean = true) =
            operation(JSONObject().put("type", "history_limit").put("limit", historyLimit), publish)
        private fun operation(operation: JSONObject, publish: Boolean) {
            response = SharedSubtitleCore.exchange(JSONObject().put("state", response.getString("state"))
                .put("operation", operation))
            if (publish) this.publish()
        }
        fun publish() = SubtitleBus.onRuntimeSnapshot(JSONObject().put("atomicPreview", atomic)
            .put("detectedLanguage", "en").put("snapshot", response.getJSONObject("snapshot")), false, audioSource)
        fun sourceOwner(): Any? = response.getJSONObject("snapshot").getJSONObject("source")
            .opt("utteranceId").takeUnless { it == JSONObject.NULL }
        fun translationOwner(): Any? = response.getJSONObject("snapshot").getJSONObject("translation")
            .opt("utteranceId").takeUnless { it == JSONObject.NULL }
        fun feedback(fields: JSONObject) = SubtitleBus.onRuntimeSnapshot(fields.put("atomicPreview", atomic)
            .put("snapshot", response.getJSONObject("snapshot")), false, audioSource)
    }
    private fun pair(id: String, source: String, translation: String) = JSONObject()
        .put("type", "identified_final_pair").put("utterance_id", id).put("source", source).put("translation", translation)
    private fun draft(role: String, text: String) = JSONObject().put("type", "utterance_text")
        .put("utterance_id", "B").put("role", role).put("text", text).put("is_final", false)

    @Test fun idleWatchdogKeepsTheFirstRealtimeTranslationReadableWithoutConfirmingIt() {
        SubtitleBus.setHistoryLimit(3)
        val lane = Snapshots(false, 3)
        lane.apply(draft("source", "最初の文"))
        SubtitleBus.hideLive()
        assertTrue(SubtitleBus.displaySnapshot().hidden)
        lane.apply(draft("translation", "第一句"))
        val reading = SubtitleBus.displaySnapshots().single()
        assertEquals("最初の文", reading.source)
        assertEquals("第一句", reading.translation)
        assertFalse(reading.confirmed)
        assertTrue(reading.history.isEmpty())
        SubtitleBus.hideLive()
        assertEquals(reading, SubtitleBus.displaySnapshots().single())
        SubtitleBus.finishSnapshot()
        assertFalse(SubtitleBus.hasRetainedContent)
    }

    @Test fun emptyAtomicHandoffReadsTheLastConfirmedRowFromItsOwnSource() {
        SubtitleBus.setHistoryLimit(3)
        val system = Snapshots(true, 3)
        val microphone = Snapshots(true, 3, "microphone")
        fun source(id: Int, text: String) = JSONObject().put("type", "source_utterance_draft")
            .put("utterance_id", id).put("text", text)
        system.apply(source(2, "次の文"))
        system.apply(JSONObject().put("type", "preview_pair").put("source_utterance_id", 2)
            .put("source", "次の文").put("translation", "下一句草稿"))
        // A's final arrived late while B owned the on-screen preview.
        system.apply(JSONObject().put("type", "confirmed_pair").put("utterance_id", 1)
            .put("source_utterance_id", 1).put("source", "最初の文。").put("translation", "第一句。"))
        microphone.apply(pair("M", "Mic.", "麦克风。"))
        system.apply(JSONObject().put("type", "clear_preview"))
        system.apply(source(3, ""))
        val row = SubtitleBus.historySnapshotWithTime().single { it.pair.audioSource == "system" }
        fun systemReading(mode: CaptionDisplayMode) =
            SubtitleBus.displaySnapshots().single { it.audioSource == "system" }.forReading(mode)
        for (mode in CaptionDisplayMode.entries) {
            val reading = systemReading(mode)
            assertFalse(reading.hidden)
            assertEquals("最初の文。", reading.source)
            if (mode.translationVisible) assertEquals("第一句。", reading.translation)
            assertTrue(reading.confirmed)
            assertEquals(row.createdAt, reading.createdAt)
        }
        system.apply(source(3, "新しい文"))
        assertEquals("新しい文", systemReading(CaptionDisplayMode.ORIGINAL).source)
        assertEquals("最初の文。", systemReading(CaptionDisplayMode.BILINGUAL).source)
        system.apply(JSONObject().put("type", "clear"))
        assertEquals("", systemReading(CaptionDisplayMode.BILINGUAL).translation)
    }

    @Test fun independentSourcesPreservePairsAndBoundedHistoryAcrossStop() {
        val system = Snapshots(false, 2, "system")
        val microphone = Snapshots(false, 2, "microphone")
        system.apply(pair("A", "Synthetic system.", "系统合成。"))
        microphone.apply(pair("A", "Synthetic microphone.", "麦克风合成。"))
        system.apply(draft("source", "System unfinished draft"))
        microphone.apply(draft("source", "Microphone unfinished draft"))
        val live = SubtitleBus.displaySnapshots()
        assertEquals(listOf("system", "microphone"), live.map { it.audioSource })
        assertEquals(listOf("System unfinished draft", "Microphone unfinished draft"), live.map { it.source })
        assertEquals(listOf("系统合成。", "麦克风合成。"), live.map { it.forReading(CaptionDisplayMode.BILINGUAL).translation })
        assertEquals(setOf("system", "microphone"), SubtitleBus.historySnapshot().map { it.audioSource }.toSet())
        assertEquals(2, SubtitleBus.historySnapshot().size)
        SubtitleBus.finishSnapshot()
        val stopped = SubtitleBus.displaySnapshots()
        assertTrue(SubtitleBus.hasRetainedContent)
        assertEquals(listOf("Synthetic system.", "Synthetic microphone."), stopped.map { it.source })
        assertTrue(stopped.all { it.confirmed && it.feedback == SubtitleBus.RuntimeFeedback() })
        assertEquals(listOf("系统合成。", "麦克风合成。"), stopped.map { it.translation })
        SubtitleBus.finishSnapshot()
        assertEquals(stopped, SubtitleBus.displaySnapshots())
        SubtitleBus.setHistoryLimit(1)
        assertEquals(1, SubtitleBus.historySnapshot().size)
        assertTrue(SubtitleBus.displaySnapshots().all { it.confirmed })
        SubtitleBus.clear()
        assertFalse(SubtitleBus.hasRetainedContent)
    }

    @Test fun microphoneOnlyDraftsAndFeedbackCannotSurviveStop() {
        val microphone = Snapshots(false, 0, "microphone")
        microphone.apply(draft("source", "Synthetic microphone draft"))
        microphone.feedback(JSONObject().put("connectionStatus", "connecting").put("isTranslationTimedOut", true))
        assertEquals("microphone", SubtitleBus.displaySnapshots().single().audioSource)
        assertTrue(SubtitleBus.hasRetainedContent)
        SubtitleBus.finishSnapshot()
        assertFalse(SubtitleBus.hasRetainedContent)
        assertEquals(SubtitleBus.RuntimeFeedback(), SubtitleBus.runtimeFeedback)
        assertTrue(SubtitleBus.displaySnapshots().single().source.isEmpty())
    }

    @Test fun pendingRealtimeTranslationKeepsAWholeConfirmedReadingPairWithoutHistory() {
        SubtitleBus.setHistoryLimit(0)
        val snapshots = Snapshots(false, 0)
        snapshots.apply(pair("A", "Synthetic A.", "合成 A。"))
        val confirmed = SubtitleBus.displaySnapshot().createdAt
        snapshots.apply(draft("source", "Synthetic B."))
        val pending = SubtitleBus.displaySnapshot()
        assertEquals("Synthetic B.", pending.source)
        assertEquals("", pending.translation)
        assertTrue(pending.history.isEmpty())
        for (mode in listOf(CaptionDisplayMode.BILINGUAL, CaptionDisplayMode.TRANSLATION)) {
            val reading = pending.forReading(mode)
            assertEquals("Synthetic A.", reading.source)
            assertEquals("合成 A。", reading.translation)
            assertTrue(reading.confirmed)
            assertEquals(confirmed, reading.createdAt)
        }
        assertEquals(pending, pending.forReading(CaptionDisplayMode.ORIGINAL))
        snapshots.apply(draft("translation", "合成 B。"))
        for (mode in CaptionDisplayMode.entries) {
            val reading = SubtitleBus.displaySnapshot().forReading(mode)
            assertEquals("Synthetic B.", reading.source)
            assertEquals("合成 B。", reading.translation)
            assertFalse(reading.confirmed)
            assertNull(reading.createdAt)
        }
        SubtitleBus.clear()
        for (mode in CaptionDisplayMode.entries) assertEquals("", SubtitleBus.displaySnapshot().forReading(mode).translation)
    }

    @Test fun firstDraftAndHiddenFramesCannotInventOrReviveAConfirmedTranslation() {
        val snapshots = Snapshots(false)
        snapshots.apply(draft("source", "Synthetic first draft."))
        for (mode in CaptionDisplayMode.entries) {
            val state = SubtitleBus.displaySnapshot()
            assertEquals(state, state.forReading(mode))
        }
        snapshots.apply(pair("A", "Synthetic A.", "合成 A。"))
        snapshots.apply(draft("source", "Synthetic B."))
        val hidden = SubtitleBus.displaySnapshot().copy(hidden = true)
        for (mode in CaptionDisplayMode.entries) assertEquals(hidden, hidden.forReading(mode))
    }

    @Test fun originalReadingFollowsRecognitionWhileAtomicTranslationRetainsItsPreviousPair() {
        SubtitleBus.setHistoryLimit(0)
        val snapshots = Snapshots(true, 0)
        snapshots.apply(pair("A", "Synthetic A.", "合成 A。"))
        snapshots.apply(draft("source", "Synthetic B."))
        val pending = SubtitleBus.displaySnapshot()
        val original = pending.forReading(CaptionDisplayMode.ORIGINAL)
        assertEquals("Synthetic B.", original.source)
        assertFalse(original.confirmed)
        assertNull(original.createdAt)
        for (mode in listOf(CaptionDisplayMode.BILINGUAL, CaptionDisplayMode.TRANSLATION)) {
            assertEquals("Synthetic A.", pending.forReading(mode).source)
            assertEquals("合成 A。", pending.forReading(mode).translation)
        }
        snapshots.apply(draft("source", "Synthetic B.").put("is_final", true))
        val recognized = SubtitleBus.displaySnapshot().forReading(CaptionDisplayMode.ORIGINAL)
        assertEquals("Synthetic B.", recognized.source)
        assertFalse(recognized.confirmed)
        assertNull(recognized.createdAt)
        snapshots.apply(pair("B", "Synthetic B.", "合成 B。"))
        val complete = SubtitleBus.displaySnapshot().forReading(CaptionDisplayMode.ORIGINAL)
        assertEquals("Synthetic B.", complete.source)
        assertTrue(complete.confirmed)
        assertNotNull(complete.createdAt)
        snapshots.apply(draft("source", "Synthetic B.").put("utterance_id", "C").put("is_final", true))
        val repeated = SubtitleBus.displaySnapshot().forReading(CaptionDisplayMode.ORIGINAL)
        assertEquals("Synthetic B.", repeated.source)
        assertFalse(repeated.confirmed)
        assertNull(repeated.createdAt)
    }

    @Test fun originalReadingKeepsCompletedWordsAcrossEmptyAsrHandoffsWithoutHistory() {
        SubtitleBus.setHistoryLimit(0)
        for (atomic in listOf(false, true)) {
            SubtitleBus.clear()
            val snapshots = Snapshots(atomic, 0)
            snapshots.apply(pair("A", "Synthetic completed original.", "合成已完成译文。"))
            val confirmedTime = SubtitleBus.displaySnapshot().createdAt
            snapshots.apply(draft("source", " \n"))
            val gap = SubtitleBus.displaySnapshot()
            assertTrue(gap.currentSource.text.isBlank())
            assertTrue(gap.history.isEmpty())
            val reading = gap.forReading(CaptionDisplayMode.ORIGINAL)
            assertEquals("Synthetic completed original.", reading.source)
            assertTrue(reading.confirmed)
            assertEquals(confirmedTime, reading.createdAt)
            val hidden = gap.copy(hidden = true)
            assertEquals(hidden, hidden.forReading(CaptionDisplayMode.ORIGINAL))
            snapshots.apply(draft("source", "Synthetic next original"))
            val next = SubtitleBus.displaySnapshot().forReading(CaptionDisplayMode.ORIGINAL)
            assertEquals("Synthetic next original", next.source)
            assertFalse(next.confirmed)
            assertNull(next.createdAt)
            SubtitleBus.clear()
            assertEquals("", SubtitleBus.displaySnapshot().forReading(CaptionDisplayMode.ORIGINAL).source)
        }
    }

    @Test fun emptyOriginalHandoffCanKeepTheSharedCompletePreviewWithoutConfirmingIt() {
        SubtitleBus.setHistoryLimit(0)
        val snapshots = Snapshots(true, 0)
        snapshots.apply(draft("source", "Synthetic complete preview source."))
        snapshots.apply(JSONObject().put("type", "preview_pair").put("source_utterance_id", JSONObject.NULL)
            .put("source", "Synthetic complete preview source.").put("translation", "合成完整预览译文。"))
        snapshots.apply(draft("source", ""))
        val gap = SubtitleBus.displaySnapshot()
        assertNull(gap.retainedConfirmed)
        assertTrue(gap.currentSource.text.isBlank())
        val reading = gap.forReading(CaptionDisplayMode.ORIGINAL)
        assertEquals("Synthetic complete preview source.", reading.source)
        assertFalse(reading.confirmed)
        assertNull(reading.createdAt)
        snapshots.apply(draft("source", "Synthetic new original"))
        assertEquals("Synthetic new original", SubtitleBus.displaySnapshot().forReading(CaptionDisplayMode.ORIGINAL).source)
    }

    @Test fun recoveryFeedbackPreservesCaptionsAndDistinguishesScheduledRetriesFromNewSpeech() {
        val snapshots = Snapshots(true)
        snapshots.apply(pair("A", "Synthetic A.", "合成 A。"))
        for ((reason, retryResource, waitingResource) in listOf(
            Triple("rateLimited", R.string.runtime_rate_retry, R.string.runtime_rate_wait),
            Triple("temporarilyUnavailable", R.string.runtime_unavailable_retry, R.string.runtime_unavailable_wait),
        )) {
            for (scheduled in listOf(true, false)) {
                snapshots.feedback(JSONObject().put("connectionStatus", "listening")
                    .put("translationRecovery", JSONObject().put("reason", reason).put("retryAfterMs", 0).put("retryScheduled", scheduled)))
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
        assertThrows(IllegalArgumentException::class.java) {
            snapshots.feedback(JSONObject().put("translationRecovery", JSONObject().put("reason", "synthetic-private-error")))
        }
        assertNull(runtimeFeedbackResource(SubtitleBus.RuntimeFeedback(recoveryReason = "synthetic-private-error")))
        assertNull(runtimeFeedbackResource(SubtitleBus.runtimeFeedback))
        SubtitleBus.finishSnapshot()
        assertEquals("Synthetic A.", SubtitleBus.displaySource)
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


    private fun volcanoCase(id: String): JSONObject {
        val fixture = requireNotNull(javaClass.getResourceAsStream("/realtime-display-contracts.json"))
            .bufferedReader().use { JSONObject(it.readText()) }
        val cases = fixture.getJSONArray("cases")
        return (0 until cases.length()).map(cases::getJSONObject).single { it.getString("id") == id }
    }

    private fun assertFixtureReadingModes(case: JSONObject, stepIndex: Int) {
        val steps = case.getJSONArray("steps")
        val expected = steps.getJSONObject(stepIndex).getJSONObject("expected")
        // The shared fixtures name the two pending frames whose native reading
        // projection retains A while the actual source lane already belongs to B.
        val retainedPairIndex = when (case.getString("id")) {
            "volcano-proven-order-keeps-newer-source-through-first-late-end" -> if (stepIndex == 5) 5 else null
            "volcano-empty-boundary-keeps-completed-owner-without-history" -> if (stepIndex == 6) 4 else null
            else -> null
        }
        val retained = retainedPairIndex?.let { steps.getJSONObject(it).getJSONObject("operation").getJSONObject("event") }
        val state = SubtitleBus.displaySnapshot()
        assertEquals(expected.getInt("historyCount"), state.history.size)
        for (mode in CaptionDisplayMode.entries) {
            val reading = state.forReading(mode)
            val keepsCompleted = mode != CaptionDisplayMode.ORIGINAL && retained != null
            val label = "${case.getString("id")} step=$stepIndex mode=${mode.id}"
            assertEquals(label, if (keepsCompleted) retained!!.getString("source") else expected.getString("source"), reading.source)
            if (mode != CaptionDisplayMode.ORIGINAL) {
                assertEquals(label, if (keepsCompleted) retained!!.getString("translation") else expected.getString("translation"), reading.translation)
            }
            val confirmed = keepsCompleted || expected.getBoolean("confirmed")
            assertEquals(label, confirmed, reading.confirmed)
            if (confirmed) assertNotNull(label, reading.createdAt) else assertNull(label, reading.createdAt)
        }
    }

    @Test fun provenSourceBoundaryFixturesKeepAllReadingModesOwnedAcrossActualJniRoundTrips() {
        for (id in listOf(
            "volcano-proven-order-keeps-newer-source-through-first-late-end",
            "volcano-empty-boundary-keeps-completed-owner-without-history",
            "volcano-invalid-order-metadata-remains-restorable",
        )) {
            val case = volcanoCase(id)
            val limit = case.getInt("historyLimit")
            SubtitleBus.clear(); SubtitleBus.setHistoryLimit(limit)
            val snapshots = Snapshots(false, limit)
            val steps = case.getJSONArray("steps")
            for (index in 0 until steps.length()) {
                val event = steps.getJSONObject(index).getJSONObject("operation").getJSONObject("event")
                val before = SubtitleBus.displaySnapshot()
                val owner = snapshots.sourceOwner()
                val translationOwner = snapshots.translationOwner()
                snapshots.apply(event)
                if (event.getString("type") == "source_boundary") {
                    // Ordering metadata cannot invent a word, confirmation, or
                    // new visible identity when no source text was received.
                    assertEquals(owner, snapshots.sourceOwner())
                    assertEquals(translationOwner, snapshots.translationOwner())
                    assertEquals(before, SubtitleBus.displaySnapshot())
                }
                assertFixtureReadingModes(case, index)
                val published = SubtitleBus.displaySnapshot()
                snapshots.restore()
                assertEquals(published, SubtitleBus.displaySnapshot())
                assertFixtureReadingModes(case, index)
            }
            val finalOwner = if (id == "volcano-invalid-order-metadata-remains-restorable") "volcano:A" else "volcano:B"
            assertEquals(finalOwner, snapshots.sourceOwner())
            assertEquals(finalOwner, snapshots.translationOwner())
        }
    }

    @Test fun provenSourceOrderSurvivesCoalescedNativeSnapshotPublicationAfterLateOldFinal() {
        val case = volcanoCase("volcano-proven-order-keeps-newer-source-through-first-late-end")
        val limit = case.getInt("historyLimit")
        SubtitleBus.setHistoryLimit(limit)
        val snapshots = Snapshots(false, limit)
        val steps = case.getJSONArray("steps")
        // Native polling may observe only the latest Rust snapshot. Provider
        // queue ordering and draft republishing are tested in the Rust client;
        // this test batches publication only, without recreating that queue.
        for (index in 0..5) {
            snapshots.apply(steps.getJSONObject(index).getJSONObject("operation").getJSONObject("event"), false)
            snapshots.restore(false)
        }
        snapshots.publish()
        assertEquals("volcano:B", snapshots.sourceOwner())
        assertFixtureReadingModes(case, 5)
        for (index in 6 until steps.length()) {
            snapshots.apply(steps.getJSONObject(index).getJSONObject("operation").getJSONObject("event"), false)
            snapshots.restore(false)
        }
        snapshots.publish()
        assertEquals("volcano:B", snapshots.sourceOwner())
        assertEquals("volcano:B", snapshots.translationOwner())
        assertFixtureReadingModes(case, steps.length() - 1)
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
