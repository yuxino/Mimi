package app.yuxino.mimi.android.shortcuts

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CaptionEdgePlacementTest {
    @Test fun releasedPositionDocksToTheNearestSideAndPreservesHeight() {
        val placement = CaptionEdgePlacement()
        val travelX = 320
        val travelY = 640
        fun snapped(x: Int, y: Int): Pair<Int, Int> {
            val moved = placement.moved(x, y, travelX, travelY)
            val docked = moved.dock(moved.nearestSide())
            return docked.left(travelX) to docked.top(travelY)
        }
        assertEquals(0 to 320, snapped(4, 320))
        assertEquals(320 to 480, snapped(300, 480))
        assertEquals(0 to 3, snapped(150, 3))
        assertEquals(320 to 634, snapped(170, 634))
        assertEquals(0 to 640, snapped(-200, 1_200))
    }

    @Test fun rotatingKeepsTheEdgeAndRelativeOffsetWithTheWholeTargetInside() {
        val width = 360
        val height = 720
        val target = 48
        val placement = CaptionEdgePlacement().moved(299, 224, width - target, height - target).dock(CaptionEdge.RIGHT)
        assertEquals(height, placement.left(height - target) + target)
        assertEquals(104, placement.top(width - target))
        assertEquals(placement, placement.moved(100, 100, 0, 0))
    }

    @Test fun crossingSlopOnReleaseOrReturningToThePressIsStillADrag() {
        val press = CaptionEdgeGesture(100f, 200f, 10, 80, 140)
        assertFalse(press.update(106f, 208f))
        assertTrue(press.update(108f, 208f))
        assertEquals(88, press.left(108f))
        assertEquals(148, press.top(208f))
        assertTrue(press.update(100f, 200f))
        assertFalse(press.isTap(50, 500, true))
        assertTrue(CaptionEdgeGesture(0f, 0f, 10, 0, 0).update(11f, 0f))
    }

    @Test fun onlyShortReleasesInsideTheCharacterCountAsTaps() {
        val press = CaptionEdgeGesture(0f, 0f, 10, 0, 0)
        assertFalse(press.isTap(500, 500, true))
        assertFalse(press.isTap(50, 500, false))
        assertTrue(press.isTap(50, 500, true))
    }
}
