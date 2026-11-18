package app.yuxino.mimi.android

import app.yuxino.mimi.android.captions.CaptionSourceViewport
import org.junit.Assert.assertEquals
import org.junit.Test

class CaptionSourceViewportTest {
    @Test fun translationGrowthThenRefinementCannotRevealAnOlderUnchangedSourceLine() {
        val viewport = CaptionSourceViewport()
        val limits = listOf(5, 4, 4, 5, 4).map { viewport.limit("The unchanged six-line original.", 350, 225, it) }
        assertEquals(listOf(5, 4, 4, 4, 4), limits)
        assertEquals(listOf(1, 2, 2, 2, 2), limits.map { 6 - it })
    }
    @Test fun aDifferentOriginalUsesTheAvailableSpaceAgain() {
        val viewport = CaptionSourceViewport()
        assertEquals(2, viewport.limit("Earlier original", 350, 225, 2))
        assertEquals(5, viewport.limit("Changed original", 350, 225, 5))
    }
    @Test fun resizingEitherDimensionRecomputesTheSourceViewport() {
        val viewport = CaptionSourceViewport()
        assertEquals(2, viewport.limit("Original", 350, 225, 2))
        assertEquals(5, viewport.limit("Original", 400, 225, 5))
        assertEquals(6, viewport.limit("Original", 400, 300, 6))
    }
    @Test fun appearanceModeOrDeliberateReadingResetsTheRememberedLimit() {
        val viewport = CaptionSourceViewport()
        assertEquals(2, viewport.limit("Original", 350, 225, 2))
        viewport.reset()
        assertEquals(5, viewport.limit("Original", 350, 225, 5))
    }
}
