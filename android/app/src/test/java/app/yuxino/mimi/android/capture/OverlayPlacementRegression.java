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
        check(p.isExpanded());
        check(p.setImmersive(true));
        check(!p.isExpanded());
        equal(480, p.compactY(3f, 2000, 140));
        check(p.setImmersive(false));
        check(p.isExpanded());
        equal(480, p.compactY(3f, 2000, 160));
        cases++;

        // Duplicate preference callbacks / taps cannot destroy the return state.
        for (int n = 0; n < 20; n++) {
            check(p.setImmersive(true));
            check(!p.setImmersive(true));
            check(p.setImmersive(false));
            check(!p.setImmersive(false));
            check(p.isExpanded());
            equal(160, p.getPreferredOffsetDp());
        }
        cases++;

        // Every exit returns to the one interactive panel.
        p.setImmersive(true);
        p.setImmersive(false);
        check(p.isExpanded());
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
        check(p.isExpanded());
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
        p.setImmersive(true); p.reset(90, false);
        check(p.isExpanded()); check(!p.isImmersive());
        equal(236, p.compactY(2.625f, 1200, 80));
        cases++;

        p.reset(48, false);
        p.setExpandedOffsetDp(120);
        equal(360, p.expandedY(3f, 1200, 600));
        equal(0, p.expandedY(3f, 300, 380));
        equal(360, p.expandedY(3f, 1200, 600));
        equal(144, p.compactY(3f, 1200, 100));
        equal(-80, OverlayPlacement.clampX(-500, 600, 440));
        equal(80, OverlayPlacement.clampX(500, 600, 440));
        equal(0, OverlayPlacement.clampX(40, 300, 400));
        equal(330, OverlayPlacement.windowWidth(600, 55, 280, 8));
        equal(592, OverlayPlacement.windowWidth(600, 100, 280, 8));
        equal(192, OverlayPlacement.windowWidth(200, 55, 280, 8));
        cases++;

        // The independent adjustment switch follows the SAME normal panel. Its
        // bottom/right coordinates remain reachable after drag, resize or rotation.
        equal(48, OverlayPlacement.adjustmentX(720, 640, 0, 64, 8));
        equal(8, OverlayPlacement.adjustmentX(720, 640, 40, 64, 8));
        equal(88, OverlayPlacement.adjustmentX(720, 640, -40, 64, 8));
        equal(656, OverlayPlacement.adjustmentX(720, 640, -1000, 64, 8));
        equal(0, OverlayPlacement.adjustmentX(40, 640, 0, 64, 8));
        equal(554, OverlayPlacement.adjustmentY(1600, 100, 512, 48, 10));
        // Even flush with the top edge, the switch stays INSIDE the same panel.
        equal(1542, OverlayPlacement.adjustmentY(1600, 1088, 512, 48, 10));
        equal(564, OverlayPlacement.adjustmentY(1600, 100, 512, 48, 0));
        equal(1552, OverlayPlacement.adjustmentY(1600, 1088, 512, 48, 0));
        equal(0, OverlayPlacement.adjustmentY(40, 0, 512, 48, 10));
        equal(1548, OverlayPlacement.adjustmentY(1600, 0, 1600, 48, 4));
        equal(44, OverlayPlacement.adjustmentX(720, 640, 0, 48, 4));
        equal(4, OverlayPlacement.adjustmentX(720, 640, 40, 48, 4));
        equal(84, OverlayPlacement.adjustmentX(720, 640, -40, 48, 4));
        equal(45, OverlayPlacement.adjustmentX(721, 640, 0, 48, 4));
        equal(44, OverlayPlacement.adjustmentX(720, 641, 0, 48, 4));
        // Exhaust every pixel of small safe extents: corners, drag positions,
        // resized panels, odd/even centered widths and extents shorter than a target.
        for (int extent = 1; extent <= 128; extent++) {
            for (int panel = 1; panel <= extent; panel++) {
                for (int y = 0; y <= extent - panel; y++) {
                    for (int top : new int[] {0, 4, 10}) checkInsetY(extent, y, panel, top);
                }
                int limit = (extent - panel) / 2;
                for (int x = -limit; x <= limit; x++) checkInsetX(extent, panel, x, 4);
            }
        }
        // Ordinary portrait/landscape bounds, all vertical drag positions and both horizontal edges.
        for (int extent : new int[] {240, 400, 721, 900, 1600}) {
            for (int panel : new int[] {48, 52, 100, 181, 200}) {
                for (int y = 0; y <= extent - panel; y++) checkInsetY(extent, y, panel, 0);
                int limit = (extent - panel) / 2;
                for (int x : new int[] {-limit, 0, limit}) checkInsetX(extent, panel, x, 4);
            }
        }
        cases++;

        // Restore stays adjacent to captions: below if possible, otherwise above.
        equal(66, OverlayPlacement.exitY(120, 100, 1000, 48, 6));
        equal(106, OverlayPlacement.exitY(0, 100, 1000, 48, 6));
        equal(952, OverlayPlacement.exitY(0, 1000, 1000, 48, 6));
        equal(0, OverlayPlacement.exitY(0, 100, 40, 48, 6));
        // An editor blocks the below-caption slot; move above, or use the
        // below-caption slot only when the entire target clears the editor.
        equal(806, OverlayPlacement.exitY(700, 100, 1000, 48, 6, 700));
        equal(726, OverlayPlacement.exitY(780, 100, 1000, 48, 6, 700));
        cases++;
    }
    private static void checkInsetY(int safeHeight, int panelY, int panelHeight, int topInset) {
        int y = OverlayPlacement.adjustmentY(safeHeight, panelY, panelHeight, 48, topInset);
        check(y >= 0 && y <= Math.max(0, safeHeight - 48));
        if (panelHeight >= 48 + topInset) {
            check(y >= panelY && y + 48 <= panelY + panelHeight);
            equal(topInset, panelY + panelHeight - (y + 48));
        }
        // Impossible full containment below 48px still clamps deterministically, never negative.
        if (safeHeight < 48) equal(0, y);
    }
    private static void checkInsetX(int safeWidth, int panelWidth, int panelX, int rightInset) {
        int x = OverlayPlacement.adjustmentX(safeWidth, panelWidth, panelX, 48, rightInset);
        check(x >= 0 && x <= Math.max(0, safeWidth - 48));
        if (panelWidth >= 48 + rightInset) {
            int panelLeft = (safeWidth - panelWidth) / 2 + panelX;
            int panelRight = panelLeft + panelWidth;
            int right = safeWidth - x;
            check(right - 48 >= panelLeft && right <= panelRight);
            equal(rightInset, panelRight - right);
        }
        if (safeWidth < 48) equal(0, x);
    }
    public static void main(String[] args) {
        runAll();
        System.out.println("Overlay placement: " + cases + " regression scenarios passed (host JVM; no Android window).");
    }
}
