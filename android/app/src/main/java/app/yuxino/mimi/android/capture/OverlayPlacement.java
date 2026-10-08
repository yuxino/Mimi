package app.yuxino.mimi.android.capture;

/** Native window placement only; never interprets subtitle or provider events. */
public final class OverlayPlacement {
    private int preferredOffsetDp;
    private boolean immersive;
    private boolean expanded;
    private boolean restoreExpanded;

    public void reset(int offsetDp, boolean immersive) {
        setPreferredOffsetDp(offsetDp);
        this.immersive = immersive;
        expanded = false;
        restoreExpanded = false;
    }

    public void setPreferredOffsetDp(int offsetDp) {
        preferredOffsetDp = Math.max(0, offsetDp);
    }

    public int getPreferredOffsetDp() { return preferredOffsetDp; }
    public boolean isImmersive() { return immersive; }
    public boolean isExpanded() { return expanded; }

    /** Duplicate preference notifications cannot overwrite the return state. */
    public boolean setImmersive(boolean enabled) {
        if (immersive == enabled) return false;
        if (enabled) {
            restoreExpanded = expanded;
            expanded = false;
        } else {
            expanded = restoreExpanded;
            restoreExpanded = false;
        }
        immersive = enabled;
        return true;
    }

    public boolean expand() {
        if (immersive || expanded) return false;
        expanded = true;
        return true;
    }

    public boolean collapse() {
        if (!expanded) return false;
        expanded = false;
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

    /** Keep the small restore control beside captions, below when space permits. */
    public static int exitY(int captionY, int captionHeight, int availableHeight, int controlHeight, int gap) {
        int desired = captionY >= controlHeight + gap
            ? captionY - controlHeight - gap : captionY + captionHeight + gap;
        return clampY(desired, availableHeight, controlHeight);
    }
}
