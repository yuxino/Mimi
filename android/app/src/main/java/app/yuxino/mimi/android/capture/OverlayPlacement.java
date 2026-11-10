package app.yuxino.mimi.android.capture;

/** Native window placement only; never interprets subtitle or provider events. */
public final class OverlayPlacement {
    private int preferredOffsetDp;
    private int expandedOffsetDp;
    private boolean immersive;

    public void reset(int offsetDp, boolean immersive) {
        setPreferredOffsetDp(offsetDp);
        this.immersive = immersive;
        expandedOffsetDp = 0;
    }

    public void setPreferredOffsetDp(int offsetDp) {
        preferredOffsetDp = Math.max(0, offsetDp);
    }

    public void setExpandedOffsetDp(int offsetDp) { expandedOffsetDp = Math.max(0, offsetDp); }
    public int expandedY(float density, int availableHeight, int panelHeight) {
        return clampY(Math.round(expandedOffsetDp * density), availableHeight, panelHeight);
    }
    public static int clampX(int desiredX, int availableWidth, int windowWidth) {
        int limit = Math.max(0, (availableWidth - windowWidth) / 2);
        return Math.max(-limit, Math.min(desiredX, limit));
    }
    public static int windowWidth(int availableWidth, int percent, int minimumWidth, int margin) {
        int maximum = Math.max(1, availableWidth - Math.max(0, margin));
        int desired = Math.round(availableWidth * Math.max(55, Math.min(100, percent)) / 100f);
        return Math.min(maximum, Math.max(minimumWidth, desired));
    }

    public int getPreferredOffsetDp() { return preferredOffsetDp; }
    public boolean isImmersive() { return immersive; }
    public boolean isExpanded() { return !immersive; }

    /** A single interactive panel alternates with touch-through immersive captions. */
    public boolean setImmersive(boolean enabled) {
        if (immersive == enabled) return false;
        immersive = enabled;
        return true;
    }

    /** Clamp the displayed position, not the user's saved preference. */
    public int compactY(float density, int availableHeight, int captionHeight) {
        return clampY(Math.round(preferredOffsetDp * density), availableHeight, captionHeight);
    }

    public static int clampY(int desiredY, int availableHeight, int windowHeight) {
        return Math.max(0, Math.min(desiredY, Math.max(0, availableHeight - windowHeight)));
    }

    public static int availableExtent(int total, int startInset, int endInset) {
        return Math.max(1, total - Math.max(0, startInset) - Math.max(0, endInset));
    }

    /** Bottom/right-gravity companion aligned with the normal panel's right edge. */
    public static int adjustmentX(int availableWidth, int panelWidth, int panelX, int controlWidth, int padding) {
        int desired = (availableWidth - panelWidth) / 2 - panelX + padding;
        return Math.max(0, Math.min(desired, Math.max(0, availableWidth - controlWidth)));
    }

    /** Prefer outside above; use below at the top edge without moving saved captions. */
    public static int adjustmentY(int availableHeight, int panelY, int panelHeight, int controlHeight, int gap) {
        int above = panelY + panelHeight + gap;
        if (above + controlHeight <= availableHeight) return Math.max(0, above);
        int below = panelY - controlHeight - gap;
        if (below >= 0) return clampY(below, availableHeight, controlHeight);
        // A very short display may have no free slot; recovery must stay reachable.
        return clampY(above, availableHeight, controlHeight);
    }

    /** Keep the small restore control beside captions, below when space permits. */
    public static int exitY(int captionY, int captionHeight, int availableHeight, int controlHeight, int gap) {
        return exitY(captionY, captionHeight, availableHeight, controlHeight, gap, 0);
    }

    /** A bottom editor is occupied space, even while the restore control is inert. */
    public static int exitY(int captionY, int captionHeight, int availableHeight, int controlHeight, int gap, int minimumY) {
        int below = captionY - controlHeight - gap;
        int desired = below >= Math.max(0, minimumY) ? below : captionY + captionHeight + gap;
        return clampY(Math.max(desired, minimumY), availableHeight, controlHeight);
    }
}
