package app.yuxino.mimi.android

import org.junit.Assert.assertEquals
import org.junit.Test

class CaptionLaneBudgetTest {
    @Test fun compactViewportFitsOneCompleteRowPerLaneBeforeExpandingTranslation() {
        assertEquals(1 to 1, captionLaneBudget(70, listOf(30, 60), listOf(40, 80)))
        assertEquals(1 to 2, captionLaneBudget(110, listOf(30, 60), listOf(40, 80)))
    }
    @Test fun aSmallerReferenceFontCannotTakeTheSecondTranslationLine() {
        assertEquals(1 to 2, captionLaneBudget(120, listOf(20, 40, 60, 80, 100, 120), listOf(50, 100, 150)))
        assertEquals(1 to 2, captionLaneBudget(160, listOf(36, 64, 92, 120), listOf(60, 120, 180)))
    }
    @Test fun longOriginalCannotConsumeTheTranslationLane() {
        assertEquals(2 to 2, captionLaneBudget(100, listOf(20, 40, 60, 80, 100, 120), listOf(24, 48, 72, 96)))
    }
    @Test fun shortLanesGiveTheirSpareSpaceToTheOtherLanguage() {
        assertEquals(1 to 3, captionLaneBudget(100, listOf(20), listOf(24, 48, 72, 96)))
        assertEquals(3 to 1, captionLaneBudget(100, listOf(20, 40, 60, 80, 100), listOf(30)))
    }
    @Test fun singleLanguageUsesAllAvailableReadingSpace() {
        assertEquals(0 to 4, captionLaneBudget(100, emptyList(), listOf(24, 48, 72, 96, 120)))
        assertEquals(4 to 0, captionLaneBudget(80, listOf(20, 40, 60, 80, 100), emptyList()))
        assertEquals(0 to 0, captionLaneBudget(80, emptyList(), emptyList()))
    }
}
