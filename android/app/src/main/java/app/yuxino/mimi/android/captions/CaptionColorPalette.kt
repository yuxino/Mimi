package app.yuxino.mimi.android.captions

import android.content.Context
import android.view.ViewGroup

/** Wrap swatches on narrow surfaces instead of shrinking their touch targets. */
internal class CaptionColorPalette(context: Context) : ViewGroup(context) {
    private var columns = 5
    private val touchSize get() = (48 * resources.displayMetrics.density + .5f).toInt()

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        val width = MeasureSpec.getSize(widthMeasureSpec)
        columns = (width / touchSize).coerceIn(1, childCount.coerceAtLeast(1))
        val cellWidth = width / columns
        for (index in 0 until childCount) getChildAt(index).measure(
            MeasureSpec.makeMeasureSpec(cellWidth, MeasureSpec.EXACTLY),
            MeasureSpec.makeMeasureSpec(touchSize, MeasureSpec.EXACTLY))
        setMeasuredDimension(width, resolveSize(((childCount + columns - 1) / columns) * touchSize, heightMeasureSpec))
    }

    override fun onLayout(changed: Boolean, left: Int, top: Int, right: Int, bottom: Int) {
        val cellWidth = width / columns
        for (index in 0 until childCount) {
            val x = index % columns * cellWidth
            val y = index / columns * touchSize
            getChildAt(index).layout(x, y, x + cellWidth, y + touchSize)
        }
    }
    override fun generateDefaultLayoutParams() = LayoutParams(LayoutParams.WRAP_CONTENT, LayoutParams.WRAP_CONTENT)
}
