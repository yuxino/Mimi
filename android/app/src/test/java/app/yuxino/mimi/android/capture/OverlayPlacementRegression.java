package app.yuxino.mimi.android.capture;

/** Same regressions run under Android JUnit and a credential-free host JVM. */
public final class OverlayPlacementRegression {
    private static int cases;
    private static void equal(int expected, int actual) {
        if (expected != actual) throw new AssertionError("Expected " + expected + ", got " + actual);
    }
    private static void check(boolean condition) {
        if (!condition) throw new AssertionError("Placement invariant failed");
    }
    public static void runAll() {
        cases = 0;
        OverlayPlacement p = new OverlayPlacement();
        p.reset(160, false);
        equal(480, p.compactY(3f, 2000, 160));
        check(p.expand());
        check(p.setImmersive(true));
        check(!p.isExpanded());
        equal(480, p.compactY(3f, 2000, 140));
        check(p.setImmersive(false));
        check(p.isExpanded());
        check(p.collapse());
        equal(480, p.compactY(3f, 2000, 160));
        cases++;

        // Duplicate preference callbacks / taps cannot destroy the return state.
        for (int n = 0; n < 20; n++) {
            check(p.expand());
            check(!p.expand());
            check(p.setImmersive(true));
            check(!p.setImmersive(true));
            check(!p.expand());
            check(p.setImmersive(false));
            check(!p.setImmersive(false));
            check(p.isExpanded());
            check(p.collapse());
            equal(160, p.getPreferredOffsetDp());
        }
        cases++;

        // Entering from settings while compact returns to compact.
        p.setImmersive(true);
        p.setImmersive(false);
        check(!p.isExpanded());
        cases++;

        // Rotation clamps temporarily; portrait restores the saved dp anchor.
        equal(180, p.compactY(3f, 360, 180));
        equal(160, p.getPreferredOffsetDp());
        equal(480, p.compactY(3f, 2000, 180));
        cases++;

        // Caption/font height changes grow upward around the bottom anchor.
        equal(480, p.compactY(3f, 1200, 40));
        equal(480, p.compactY(3f, 1200, 240));
        equal(220, p.compactY(3f, 600, 380));
        equal(480, p.compactY(3f, 1200, 240));
        cases++;

        int safeHeight = OverlayPlacement.availableExtent(480, 24, 48);
        int safeWidth = OverlayPlacement.availableExtent(1080, 80, 32);
        equal(408, safeHeight);
        equal(968, safeWidth);
        equal(228, p.compactY(3f, safeHeight, 180));
        equal(1, OverlayPlacement.availableExtent(10, 10, 10));
        cases++;

        // Insets are consumed by WindowManager; never added to y a second time.
        p.reset(48, true);
        equal(144, p.compactY(3f, safeHeight, 180));
        check(p.setImmersive(false));
        check(!p.isExpanded());
        cases++;

        p.reset(2000, false);
        equal(0, p.compactY(3f, 200, 240));
        equal(0, OverlayPlacement.clampY(-80, 400, 40));
        equal(360, OverlayPlacement.clampY(900, 400, 40));
        equal(2000, p.getPreferredOffsetDp());
        cases++;

        // Fractional densities use dp, not repeated truncating px round trips.
        p.reset(51, false);
        equal(134, p.compactY(2.625f, 1200, 80));
        for (int n = 0; n < 50; n++) { p.setImmersive(true); p.setImmersive(false); }
        equal(134, p.compactY(2.625f, 1200, 80));
        equal(51, p.getPreferredOffsetDp());
        cases++;

        // A genuine user drag replaces the preference; resets retire mode state.
        p.setPreferredOffsetDp(90);
        equal(236, p.compactY(2.625f, 1200, 80));
        p.expand(); p.setImmersive(true); p.reset(90, false);
        check(!p.isExpanded()); check(!p.isImmersive());
        equal(236, p.compactY(2.625f, 1200, 80));
        cases++;

        // Restore stays adjacent to captions: below if possible, otherwise above.
        equal(66, OverlayPlacement.exitY(120, 100, 1000, 48, 6));
        equal(106, OverlayPlacement.exitY(0, 100, 1000, 48, 6));
        equal(952, OverlayPlacement.exitY(0, 1000, 1000, 48, 6));
        equal(0, OverlayPlacement.exitY(0, 100, 40, 48, 6));
        cases++;
    }
    public static void main(String[] args) {
        runAll();
        System.out.println("Overlay placement: " + cases + " regression scenarios passed (host JVM; no Android window).");
    }
}
