package app.yuxino.mimi.android.capture

import org.junit.Test

class OverlayPlacementTest {
    @Test fun preservesCompactAnchorAndPresentationAcrossModeAndBoundsChanges() {
        OverlayPlacementRegression.runAll()
    }
}
