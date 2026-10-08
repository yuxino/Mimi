package app.yuxino.mimi.android.provider

import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test

/** Exercises the real JNI reducer, rather than a duplicate Kotlin implementation. */
class SubtitleBusTest {
    @Before fun reset() { SubtitleBus.clear(); SubtitleBus.setHistoryLimit(0) }
    @After fun cleanup() = reset()

    @Test fun disabledHistoryDoesNotRetainPastSources() {
        SubtitleBus.onFinalPair("Earlier sentence.", "Earlier translation.")
        assertTrue(SubtitleBus.historySnapshot().isEmpty())
        SubtitleBus.setHistoryLimit(6)
        SubtitleBus.onFinalPair("Current sentence.", "Current translation.")
        assertEquals(listOf(SubtitleBus.Pair("Current sentence.", "Current translation.")), SubtitleBus.historySnapshot())
    }
    @Test fun disablingHistoryClearsRowsButRetainsOnlyCurrentCaption() {
        SubtitleBus.setHistoryLimit(6)
        SubtitleBus.onFinalPair("First.", "第一句。")
        SubtitleBus.onFinalPair("Second.", "第二句。")
        SubtitleBus.setHistoryLimit(0)
        assertTrue(SubtitleBus.historySnapshot().isEmpty())
        assertEquals("Second.", SubtitleBus.displaySource)
        assertEquals("第二句。", SubtitleBus.displayTranslation)
        SubtitleBus.setHistoryLimit(6)
        assertTrue(SubtitleBus.historySnapshot().isEmpty())
    }
    @Test fun historyLimitMatchesSettingAndSnapshotsStayStable() {
        SubtitleBus.setHistoryLimit(6)
        repeat(7) { SubtitleBus.onFinalPair("source $it", "translation $it") }
        val snapshot = SubtitleBus.historySnapshot()
        assertEquals(6, snapshot.size)
        assertEquals("source 1", snapshot.first().source)
        SubtitleBus.setHistoryLimit(2)
        assertEquals(2, SubtitleBus.historySnapshot().size)
        assertEquals(6, snapshot.size)
    }
    @Test fun newSessionResetsLanguageAndVisibility() {
        SubtitleBus.onSourceDraft("Hello", "en-US")
        SubtitleBus.hideLive()
        SubtitleBus.clear()
        assertNull(SubtitleBus.detectedSourceLanguage)
        assertFalse(SubtitleBus.liveHidden)
        assertEquals("", SubtitleBus.sourceDraft)
        assertEquals("", SubtitleBus.displayTranslation)
    }
    @Test fun completeFinalPreservesPunctuationAndEverySentence() {
        val source = "今天下雨了，我们明天再出发。还有后续。"
        val translation = "First sentence. Last sentence."
        SubtitleBus.setHistoryLimit(1)
        SubtitleBus.onFinalPair(source, "  $translation  \n")
        assertEquals(source, SubtitleBus.sourceFinal)
        assertEquals(translation, SubtitleBus.translationFinal)
        assertEquals(listOf(SubtitleBus.Pair(source, translation)), SubtitleBus.historySnapshot())
    }
    @Test fun ordinaryLongDraftIsNeverCroppedToItsLastTwoHundredCharacters() {
        val text = "a".repeat(220) + "RECENT"
        SubtitleBus.onTranslationDraft(text)
        assertEquals(text, SubtitleBus.translationDraft)
        SubtitleBus.onTranslationDraft("🙂".repeat(16_384) + "x")
        assertEquals(text, SubtitleBus.translationDraft)
    }
    @Test fun newRawRecognitionRetainsCompletePairAndLateFinalDoesNotEraseRawDraft() {
        SubtitleBus.setHistoryLimit(2)
        val first = SubtitleBus.onUntranslatedSource("First sentence.", "en", true)
        SubtitleBus.onTranslatedSource("First sentence.", "en", "最初の文。", first)
        val second = SubtitleBus.onUntranslatedSource("Second sentence.", "en", true)
        SubtitleBus.onUntranslatedSource("Next draft", "en", false)
        assertEquals("最初の文。", SubtitleBus.displayTranslation)
        SubtitleBus.onTranslatedSource("Second sentence.", "en", "二番目の文。", second)
        assertEquals("Next draft", SubtitleBus.sourceDraft)
        assertEquals("二番目の文。", SubtitleBus.displayTranslation)
        assertEquals(2, SubtitleBus.historySnapshot().size)
    }
    @Test fun anOlderCompletionCannotOverwriteANewerCompletePair() {
        SubtitleBus.setHistoryLimit(3)
        val first = SubtitleBus.onUntranslatedSource("First.", "en", true)
        val second = SubtitleBus.onUntranslatedSource("Second.", "en", true)
        SubtitleBus.onTranslatedSource("Second.", "en", "第二句。", second)
        SubtitleBus.onTranslatedSource("First.", "en", "第一句。", first)
        assertEquals("Second.", SubtitleBus.displaySource)
        assertEquals("第二句。", SubtitleBus.displayTranslation)
        assertEquals(2, SubtitleBus.historySnapshot().size)
    }
    @Test fun repeatedIdenticalUtterancesKeepDistinctSourceIdentities() {
        SubtitleBus.setHistoryLimit(2)
        val first = SubtitleBus.onUntranslatedSource("Again.", "en", true)
        SubtitleBus.onTranslatedSource("Again.", "en", "再一次。", first)
        val second = SubtitleBus.onUntranslatedSource("Again.", "en", true)
        SubtitleBus.onTranslatedSource("Again.", "en", "再一次。", second)
        assertNotEquals(first, second)
        assertEquals(2, SubtitleBus.historySnapshot().size)
    }
    @Test fun mixedBilingualTranslationRetainsTranslationAcrossActualJni() {
        SubtitleBus.setHistoryLimit(2)
        SubtitleBus.onFinalPair("Synthetic English.", "合成中文。", "zh_en")
        assertEquals("Synthetic English.", SubtitleBus.displaySource)
        assertEquals("合成中文。", SubtitleBus.displayTranslation)
        assertEquals(listOf(SubtitleBus.Pair("Synthetic English.", "合成中文。")), SubtitleBus.historySnapshot())
        SubtitleBus.onFinalPair("合成中文。", "Synthetic English.", "zh_en")
        assertEquals("Synthetic English.", SubtitleBus.displayTranslation)
    }
    @Test fun originalOnlyUsesCompleteConfirmationWithoutDuplicatingItsDisplay() {
        val text = "新しい字幕。まだ続いています。"
        SubtitleBus.onOriginalSource(text, "ja")
        assertEquals(text, SubtitleBus.sourceFinal)
        assertEquals("", SubtitleBus.displayTranslation)
        assertEquals("ja", SubtitleBus.detectedSourceLanguage)
        assertTrue(SubtitleBus.historySnapshot().isEmpty())
        SubtitleBus.setHistoryLimit(2)
        repeat(3) { SubtitleBus.onOriginalSource("source $it", "en") }
        assertEquals(listOf(SubtitleBus.Pair("source 1", ""), SubtitleBus.Pair("source 2", "")), SubtitleBus.historySnapshot())
    }
    @Test fun oversizedOriginalDoesNotEraseCompletePreviousCaption() {
        SubtitleBus.onOriginalSource("Complete previous.", "en")
        SubtitleBus.onOriginalSource("x".repeat(65_537), "en")
        assertEquals("Complete previous.", SubtitleBus.displaySource)
    }
    @Test fun idleHideCannotEraseACompleteCurrentCaptionWithHistoryDisabled() {
        SubtitleBus.setHistoryLimit(0)
        SubtitleBus.onFinalPair("Complete sentence.", "完整的一句话。")
        SubtitleBus.hideLive()
        assertFalse(SubtitleBus.liveHidden)
        assertEquals("完整的一句话。", SubtitleBus.displayTranslation)
        SubtitleBus.onSourceDraft("Next unfinished sentence.")
        SubtitleBus.hideLive()
        assertFalse(SubtitleBus.liveHidden)
        assertEquals("Next unfinished sentence.", SubtitleBus.displaySource)
        assertEquals("", SubtitleBus.displayTranslation)
        assertFalse(SubtitleBus.displayPairFinal)
        assertTrue(SubtitleBus.historySnapshot().isEmpty())
        SubtitleBus.clear()
        assertEquals("", SubtitleBus.displayTranslation)
    }
    @Test fun identifiedRealtimeDraftsAdvanceBeyondThePriorFinalWithoutMixingOwners() {
        fun draft(role: String, text: String) = org.json.JSONObject().put("type", "utterance_text")
            .put("utterance_id", "B").put("role", role).put("text", text).put("is_final", false)
        SubtitleBus.onIdentifiedFinalPair("A", "Synthetic A.", "合成 A。", "en")
        SubtitleBus.onCoreEvent(draft("source", "Synthetic B."), "en")
        assertEquals("Synthetic B.", SubtitleBus.displaySource)
        assertEquals("", SubtitleBus.displayTranslation)
        assertFalse(SubtitleBus.displayPairFinal)
        SubtitleBus.onCoreEvent(draft("translation", "合成 B。"))
        assertEquals("Synthetic B.", SubtitleBus.displaySource)
        assertEquals("合成 B。", SubtitleBus.displayTranslation)
        assertTrue(SubtitleBus.historySnapshot().isEmpty())
        SubtitleBus.onIdentifiedFinalPair("B", "Synthetic B.", "合成 B。", "en")
        assertTrue(SubtitleBus.displayPairFinal)
        assertEquals("合成 B。", SubtitleBus.displayTranslation)
    }
}
