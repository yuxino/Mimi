package app.yuxino.mimi.android

import android.view.View
import android.view.ViewGroup
import android.widget.TextView

/** Called on the UI thread after layout, against the actual floating views. */
internal fun checkLatestCompactCaption(compact: View, requireOverflow: Boolean = true): Int {
    val container = compact as ViewGroup
    check(compact.isShown) { "Compact caption is not visible" }
    var overflowing = 0
    var visible = 0
    for (index in 1..2) {
        val caption = container.getChildAt(index) as TextView
        if (!caption.isShown) continue
        visible++
        val layout = checkNotNull(caption.layout)
        if (layout.lineCount > caption.maxLines) overflowing++
        check(layout.getLineEnd(layout.lineCount - 1) == caption.text.length) { "Full caption was discarded" }
        val visibleHeight = caption.height - caption.compoundPaddingTop - caption.compoundPaddingBottom
        val firstVisibleLine = (layout.lineCount - caption.maxLines).coerceAtLeast(0)
        check(layout.getLineTop(firstVisibleLine) == caption.scrollY) { "Compact caption cuts through a line" }
        check(layout.getLineBottom(layout.lineCount - 1) - caption.scrollY <= visibleHeight + 1) {
            "Latest compact caption line is clipped: lines=${layout.lineCount}, scroll=${caption.scrollY}, height=$visibleHeight"
        }
        if (layout.lineCount <= caption.maxLines) check(caption.scrollY == 0) { "Short caption kept an old scroll offset" }
    }
    check(visible > 0) { "Compact caption has no visible text view" }
    if (requireOverflow) check(overflowing == 2) { "Fixture must overflow both compact captions" }
    return overflowing
}
