package app.yuxino.mimi.android.captions


/** A translator revision must not bring already clipped, unchanged reference lines back into view. */
internal class CaptionSourceViewport {
    private var source: String? = null
    private var width = -1
    private var height = -1
    private var limit = Int.MAX_VALUE

    fun limit(text: String, availableWidth: Int, availableHeight: Int, proposed: Int): Int {
        val next = proposed.coerceAtLeast(1)
        limit = if (source == text && width == availableWidth && height == availableHeight) minOf(limit, next) else next
        source = text; width = availableWidth; height = availableHeight
        return limit
    }

    fun reset() {
        source = null; width = -1; height = -1; limit = Int.MAX_VALUE
    }
}
