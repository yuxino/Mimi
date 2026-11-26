package app.yuxino.mimi.android.provider.subtitles

import app.yuxino.mimi.android.provider.SubtitleBus
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test

/** Exercises the real JNI reducer, rather than a duplicate Kotlin implementation. */
class SubtitleBusTest {
    @Test fun aDisplayReadCannotMixConcurrentNativePublications() {
        SubtitleBus.onFinalPair("s-0", "t-0")
        val writerFailure = java.util.concurrent.atomic.AtomicReference<Throwable?>()
        val writer = Thread {
            try { repeat(300) { SubtitleBus.onFinalPair("s-$it", "t-$it") } }
            catch (error: Throwable) { writerFailure.set(error) }
        }
        writer.start()
        try {
            repeat(1000) {
                val state = SubtitleBus.displaySnapshot()
                assertEquals(state.source.removePrefix("s-"), state.translation.removePrefix("t-"))
                assertTrue(state.confirmed)
                assertNotNull(state.createdAt)
            }
        } finally { writer.join() }
        writerFailure.get()?.let { throw it }
    }
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
    @Test fun mixedBilingualTranslationRetainsTranslationAcrossActualJni() {
        SubtitleBus.setHistoryLimit(2)
        SubtitleBus.onFinalPair("Synthetic English.", "合成中文。", "zh_en")
        assertEquals("Synthetic English.", SubtitleBus.displaySource)
        assertEquals("合成中文。", SubtitleBus.displayTranslation)
        assertEquals(listOf(SubtitleBus.Pair("Synthetic English.", "合成中文。")), SubtitleBus.historySnapshot())
        SubtitleBus.onFinalPair("合成中文。", "Synthetic English.", "zh_en")
        assertEquals("Synthetic English.", SubtitleBus.displayTranslation)
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
}
